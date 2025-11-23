pub mod error;
pub mod metadata;
pub mod progress;
pub mod reader;
pub mod work;
pub mod work_simple;
pub mod work_tree;
pub mod writer;

use std::sync::Arc;
use tokio::sync::Mutex;

/// Size threshold for considering a file "large"
// 64MiB for now
pub const LARGE_FILE_THRESHOLD: u64 = 64 * 1024 * 1024;

use error::IoError;
use progress::{AtomicOperationProgress, Progress};
use work::WorkItem;
use work_simple::SimpleWorkQueue;

/// The main I/O subsystem that bridges Bevy ECS and async I/O operations.
#[derive(Clone)]
pub struct IoSubsystem {
    /// Shared progress state that Bevy ECS can query
    /// Progress uses internal parking_lot::Mutex for the HashMap only
    /// Actual counters are lock-free atomics (well "lock-free" insofar as an
    /// atomic cmp/xchg is lock free)
    pub progress: Progress,

    /// Shared error log that need more use/abuse
    pub errors: Arc<Mutex<Vec<IoError>>>,

    /// Simple FIFO work queue for local→local copies, inter node copies NYI
    work_queue: Arc<Mutex<SimpleWorkQueue>>,

    /// Channel sender for reader to submit work, no blocking locks
    work_tx: Option<tokio::sync::mpsc::UnboundedSender<WorkItem>>,

    reader_handle: Option<Arc<reader::ReaderPool>>,
    writer_handle: Option<Arc<writer::WriterPool>>,
    reader_done: Arc<Mutex<bool>>,
    writer_done: Arc<Mutex<bool>>,
}

impl IoSubsystem {
    pub fn new() -> Self {
        Self {
            progress: Progress::default(),
            errors: Arc::new(Mutex::new(Vec::new())),
            work_queue: Arc::new(Mutex::new(SimpleWorkQueue::new())),
            work_tx: None,
            reader_handle: None,
            writer_handle: None,
            reader_done: Arc::new(Mutex::new(false)),
            writer_done: Arc::new(Mutex::new(false)),
        }
    }

    pub async fn start(
        &mut self,
        uuid: u128,
        source: std::path::PathBuf,
        dest: std::path::PathBuf,
        num_writers: Option<usize>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Ensure destination root directory exists
        if let Err(e) = tokio::fs::create_dir_all(&dest).await {
            let error_msg = format!(
                "failed to create destination root directory {}: {}",
                dest.display(),
                e
            );
            tracing::error!("{}", error_msg);
            let mut errors = self.errors.lock().await;
            errors.push(error::IoError::destination(error_msg.clone(), dest.clone()));
            return Err(error_msg.into());
        }

        // Create channel for (mostly lock free) reader -> queue communication
        let (work_tx, mut work_rx) = tokio::sync::mpsc::unbounded_channel::<WorkItem>();
        self.work_tx = Some(work_tx.clone());

        // Spawn a tokio to yeet items in batches into the tree queue. Batched
        // to minimize async locking contention.
        let queue = self.work_queue.clone();
        tokio::spawn(async move {
            const BATCH_SIZE: usize = 1000;
            let mut batch = Vec::with_capacity(BATCH_SIZE);

            while let Some(item) = work_rx.recv().await {
                batch.push(item);

                // This could use a PID controller to let BATCH_SIZE scale/up down dynamically.
                while batch.len() < BATCH_SIZE {
                    match work_rx.try_recv() {
                        Ok(item) => batch.push(item),
                        Err(_) => break, // No more items ready
                    }
                }

                // We only lock here with actual stuff to push onto the batch queue.
                {
                    let mut q = queue.lock().await;
                    for item in batch.drain(..) {
                        q.push(item);
                    }
                }
            }
            tracing::trace!("work queue finished");
        });

        // start the reader pool for this uuid operation
        let reader_pool = reader::ReaderPool::new(
            uuid,
            source,
            work_tx,
            self.progress.clone(),
            self.errors.clone(),
            self.reader_done.clone(),
        );

        let reader_handle = Arc::new(reader_pool);
        reader_handle.clone().start().await;
        self.reader_handle = Some(reader_handle);

        // Start writer pool if not already running, writer is shared.
        if self.writer_handle.is_none() {
            let writer_pool = writer::WriterPool::new(
                dest,
                self.work_queue.clone(),
                self.progress.clone(),
                self.errors.clone(),
                self.reader_done.clone(),
                self.writer_done.clone(),
            );

            let writer_handle = Arc::new(writer_pool);
            writer_handle.clone().start(num_writers).await;
            self.writer_handle = Some(writer_handle);
        }

        Ok(())
    }

    pub async fn get_all_progress(&self) -> Progress {
        self.progress.clone()
    }

    // TODO: I should probably just keep one get fn but this whole things a beast
    pub async fn get_progress(&self, uuid: u128) -> Option<progress::OperationProgress> {
        self.progress.get(uuid)
    }

    pub fn get_atomic_progress(&self, uuid: u128) -> Arc<AtomicOperationProgress> {
        self.progress.get_or_create(uuid)
    }

    pub async fn error_count(&self) -> usize {
        self.errors.lock().await.len()
    }

    pub async fn get_errors(&self) -> Vec<IoError> {
        self.errors.lock().await.clone()
    }

    /// Check if a specific I/O operation is complete or not.
    /// An operation is complete when both reader(s) and writer say they've
    /// completed. Need to have a better option here.
    pub async fn is_complete(&self, uuid: u128) -> bool {
        let reader_complete = *self.reader_done.lock().await;
        let writer_complete = *self.writer_done.lock().await;

        // Check if we actually found files, aka empty dir isn't worth creating read/write nonsense
        let has_work = self
            .progress
            .get(uuid)
            .map_or(false, |p| p.files_found > 0 || p.dirs_found > 0);

        reader_complete && writer_complete && has_work
    }

    pub async fn shutdown(&mut self) {
        if let Some(reader) = &self.reader_handle {
            reader.shutdown().await;
        }
        if let Some(writer) = &self.writer_handle {
            writer.shutdown().await;
        }
    }
}

impl Default for IoSubsystem {
    fn default() -> Self {
        Self::new()
    }
}
