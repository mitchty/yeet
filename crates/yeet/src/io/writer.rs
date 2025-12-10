use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::Mutex;

use super::error::IoError;
use super::metadata::FileMetadata;
use super::progress::Progress;
use super::work::WorkItem;
use super::work_simple::SimpleWorkQueue;

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

/// Filesystem feature detection for handling quirks of different filesystem types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FsFeatures {
    /// Normal POSIX-compliant filesystem
    Normal,
    /// CIFS/Samba mount - chmod/chown may fail with EPERM even after successful copy
    Samba,
}

// TODO: Need to probably use
// https://docs.rs/nix/latest/nix/sys/statvfs/index.html to control if we are
// writing to a network fs or not. and if socontrol those workers to 1/2. This
// is probably a good thing to tackle once I get syncing in place.
const DEFAULT_WORKERS: usize = 4;

/// Get number of workers based off the core count for now. Default is 4 if we
/// can't detect that, god knows if thats right or not. Should probably be 1 or
/// 2 then.
fn get_num_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(DEFAULT_WORKERS)
}

/// Sigh, filesystems suck, network especially. Detect that our destination is a
/// CIFS/Samba mount that lacks support for fchmod/chmod support. If so, we
/// can't really trust errors from libc copy function calls.
#[cfg(unix)]
fn detect_fs_features(dest_dir: &std::path::Path) -> FsFeatures {
    use std::fs::File;
    use std::io::Write;
    use std::os::raw::c_long;

    // TODO: Top is what I found in node for this same general approach
    // https://github.com/nodejs/node/issues/31170
    // bottom is what I see on my desktop. Need to double check which is
    // "right"-er. For now whatever.
    const CIFS_MAGIC_1: c_long = 0xFF534D42u32 as c_long;
    const CIFS_MAGIC_2: c_long = 0xFE534D42u32 as c_long;

    let test_file_path = dest_dir.join(".yeet_fs_feature_detection");

    let detected = (|| -> Result<FsFeatures, std::io::Error> {
        let mut test_file = File::create(&test_file_path)?;
        test_file.write_all(b"test")?;
        test_file.sync_all()?;

        let fd = test_file.as_raw_fd();
        let mut stat: libc::statfs = unsafe { std::mem::zeroed() };

        if unsafe { libc::fstatfs(fd, &mut stat) } == 0 {
            if stat.f_type == CIFS_MAGIC_1 || stat.f_type == CIFS_MAGIC_2 {
                tracing::info!(
                    "detected a CIFS/Samba filesystem {} (f_type: 0x{:X})",
                    dest_dir.display(),
                    stat.f_type
                );
                Ok(FsFeatures::Samba)
            } else {
                tracing::debug!(
                    "detected a normal filesystem {} (f_type: 0x{:X})",
                    dest_dir.display(),
                    stat.f_type
                );
                Ok(FsFeatures::Normal)
            }
        } else {
            tracing::warn!("fstatfs failed, assuming a normal filesystem");
            Ok(FsFeatures::Normal)
        }
    })();

    let _ = std::fs::remove_file(&test_file_path);

    // Not sure if this is the best logic but I'll fix it in post, gotta get this doing real crap first.
    detected.unwrap_or_else(|e| {
        tracing::warn!(
            "filesystem detection failed: {}, assuming normal filesystem",
            e
        );
        FsFeatures::Normal
    })
}

// TODO: on non unix what goes here? Only the shadow knows.
#[cfg(not(unix))]
fn detect_fs_features(_dest_dir: &std::path::Path) -> FsFeatures {
    FsFeatures::Normal
}

/// Writer pool that processes work items from the queue I played around with
/// multiple writers, its... not as useful as I had hoped need to rethink my
/// solution here.
pub struct WriterPool {
    dest: PathBuf,
    work_queue: Arc<Mutex<SimpleWorkQueue>>,
    progress: Progress,
    errors: Arc<Mutex<Vec<IoError>>>,
    shutdown: Arc<Mutex<bool>>,
    active_workers: Arc<Mutex<usize>>,
    _reader_done: Arc<Mutex<bool>>, // TODO: keep?
    done: Arc<Mutex<bool>>,
    fs_features: FsFeatures,
}

impl WriterPool {
    pub fn new(
        dest: PathBuf,
        work_queue: Arc<Mutex<SimpleWorkQueue>>,
        progress: Progress,
        errors: Arc<Mutex<Vec<IoError>>>,
        reader_done: Arc<Mutex<bool>>,
        done: Arc<Mutex<bool>>,
    ) -> Self {
        // Try to figure out if our dest is a problematic fs or not that might not
        // support chmod
        let fs_features = detect_fs_features(&dest);

        Self {
            dest,
            work_queue,
            progress,
            errors,
            shutdown: Arc::new(Mutex::new(false)),
            active_workers: Arc::new(Mutex::new(0)),
            _reader_done: reader_done,
            done,
            fs_features,
        }
    }

    pub async fn is_idle(&self) -> bool {
        let active = self.active_workers.lock().await;
        *active == 0
    }

    /// Check if work is complete and set done flag
    /// Writers only mark themselves as done iff:
    /// - The reader has finished traversing
    /// - The work queue is empty
    /// - No workers are actively processing data
    async fn check_completion(&self) {
        // Check if already marked as done
        {
            let done = self.done.lock().await;
            if *done {
                return;
            }
        }

        // Check if queue reports complete (scan done, no ready/blocked work)
        let queue = self.work_queue.lock().await;
        let queue_complete = queue.is_complete();
        drop(queue);

        // Check if no workers are active
        let active = self.active_workers.lock().await;
        let workers_idle = *active == 0;
        drop(active);

        // Only mark as done if queue is complete and workers are idle
        if queue_complete && workers_idle {
            let mut done = self.done.lock().await;
            if !*done {
                *done = true;
                tracing::debug!("writer pool marked as complete");
            }
        }
    }

    pub async fn start(self: Arc<Self>, num_workers_override: Option<usize>) {
        let num_workers = num_workers_override.unwrap_or_else(get_num_workers);
        let detected_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0);

        if let Some(_override_val) = num_workers_override {
            tracing::debug!(
                "writer pool starting with {} parallel workers (user specified, {} cores detected)",
                num_workers,
                detected_cores
            );
        } else {
            tracing::debug!(
                "writer pool starting with {} parallel workers (auto-detected from {} cores)",
                num_workers,
                detected_cores
            );
        }

        // Start multiple parallel workers as dedicated blocking threads
        // This avoids spawn_blocking overhead for every file operation
        for worker_id in 0..num_workers {
            let pool = self.clone();
            std::thread::spawn(move || {
                tracing::trace!("worker {} starting in blocking thread", worker_id);

                // Create a tokio runtime for this worker thread
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to create worker runtime");

                rt.block_on(async move {
                    pool.worker_blocking(worker_id).await;
                });

                tracing::debug!("worker {} thread exiting", worker_id);
            });
        }
    }

    /// Blocking worker loop - runs in dedicated thread, uses blocking I/O directly
    // may need a revisit once I get inter node copies/syncs working. I think
    // network i/o and disk i/o likely need to be separated.
    async fn worker_blocking(&self, worker_id: usize) {
        // TODO: Numbers pulled out of my ass, need to make it dynamic with a
        // PID controller probably somehow..... more future mitch problems.
        const BATCH_SIZE: usize = 100;

        loop {
            // Check shutdown signal
            if *self.shutdown.lock().await {
                tracing::info!("worker {} received shutdown signal, exiting", worker_id);
                break;
            }

            // Get a batch of work items (dirs first, then files to avoid parentage missing issues via logical sanity)
            let work_batch = {
                let mut queue = self.work_queue.lock().await;
                queue.pop_batch(BATCH_SIZE)
            };

            if !work_batch.is_empty() {
                {
                    let mut active = self.active_workers.lock().await;
                    *active += 1;
                }

                // Process items in parallel
                for item in work_batch {
                    tracing::trace!("worker {} processing: {:?}", worker_id, item);
                    if let Err(e) = self.process_work_item(item).await {
                        tracing::error!("worker {} error: {}", worker_id, e);
                    }
                }

                {
                    let mut active = self.active_workers.lock().await;
                    *active -= 1;
                }

                self.check_completion().await;
            } else {
                self.check_completion().await;

                // Sleep briefly to let locks/mutexes settle
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
        }
    }

    async fn process_work_item(&self, item: WorkItem) -> Result<(), Box<dyn std::error::Error>> {
        let uuid = item.uuid();

        match item {
            WorkItem::CreateDir {
                dest_path,
                metadata,
                ..
            } => {
                self.create_directory(uuid, dest_path, metadata).await?;
            }
            WorkItem::CopySmallFile {
                source_path,
                dest_path,
                metadata,
                ..
            } => {
                self.copy_file(uuid, source_path, dest_path, metadata)
                    .await?;
            }
            WorkItem::CopyLargeFile {
                source_path,
                dest_path,
                metadata,
                ..
            } => {
                self.copy_file(uuid, source_path, dest_path, metadata)
                    .await?;
            }
            WorkItem::CreateSymlink {
                dest_path,
                metadata,
                ..
            } => {
                self.create_symlink(uuid, dest_path, metadata).await?;
            }
            WorkItem::ApplyMetadata {
                dest_path,
                metadata,
                ..
            } => {
                self.apply_metadata(uuid, dest_path, metadata).await?;
            }
            // Sentinel items should never hit a worker, should be a panic/todo
            // but for now whatever lets see if it matters first.
            WorkItem::DirectoryScanned { .. } | WorkItem::ScanComplete { .. } => {
                tracing::warn!(
                    "worker received sentinel item - this is likely a bug mitch should have thought through"
                );
            }
        }
        Ok(())
    }

    /// Update progress counters for a file copy operation
    fn update_file_progress(&self, uuid: u128, bytes_copied: u64, file_size: u64) {
        const FAST_COPY_THRESHOLD: u64 = super::LARGE_FILE_THRESHOLD;

        let atomic_progress = self.progress.get_or_create(uuid);
        atomic_progress
            .files_written
            .fetch_add(1, Ordering::Relaxed);

        // This is for non chunked file copying only (more a perf optimization
        // to use sendfile via libc really)
        if file_size < FAST_COPY_THRESHOLD {
            atomic_progress.record_write(bytes_copied);
        }

        tracing::trace!(
            "{} file complete status: {}/{} files",
            uuid::Uuid::from_u128(uuid),
            atomic_progress.files_written.load(Ordering::Relaxed),
            atomic_progress.files_found.load(Ordering::Relaxed)
        );
    }

    async fn create_directory(
        &self,
        uuid: u128,
        relative_path: PathBuf,
        _metadata: super::metadata::DirMetadata,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dest_path = self.dest.join(&relative_path);

        // Use blocking I/O directly (worker already on blocking thread)
        match std::fs::create_dir_all(&dest_path) {
            Ok(_) => {
                let atomic_progress = self.progress.get_or_create(uuid);
                atomic_progress.dirs_written.fetch_add(1, Ordering::Relaxed);
                tracing::trace!("created directory: {}", dest_path.display());
                Ok(())
            }
            Err(e) => {
                let error_msg = format!("failed to create directory: {}", e);
                tracing::error!("{}: {}", error_msg, dest_path.display());
                let mut errors = self.errors.lock().await;
                errors.push(IoError::destination(error_msg, dest_path));
                Err(Box::new(e))
            }
        }
    }

    async fn copy_file(
        &self,
        uuid: u128,
        source_path: PathBuf,
        relative_path: PathBuf,
        metadata: FileMetadata,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dest_path = self.dest.join(&relative_path);

        // Ensure parent directory exists
        // Normally the reader queues CreateDir before files, but this is a safety net
        // in case work items are processed out of order by different workers
        if let Some(parent) = dest_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                let error_msg = format!("failed to create parent directory: {}", e);
                tracing::error!("{}: {}", error_msg, parent.display());
                let mut errors = self.errors.lock().await;
                errors.push(IoError::destination(error_msg, parent.to_path_buf()));
                return Err(Box::new(e));
            }
        }

        tracing::trace!(
            "cp file {} -> {} ({} bytes)",
            source_path.display(),
            dest_path.display(),
            metadata.size
        );

        // Limit to small files for now is same as "large file threshold" for no reason than cause.
        const FAST_COPY_THRESHOLD: u64 = super::LARGE_FILE_THRESHOLD;

        // Ok this is jank af but, we need to detect if we're copying to a CIFS
        // mount, if so we can't use std::fs::copy() as it implicitly tries to
        // chmod permissions after it copies data. But unless CIFS is mounted
        // with specific flags, that will yield an EPERM 13 errno. So we detect
        // this at copy time and avoid calling std::fs::copy() in a way where it
        // will always fail.
        //
        // Note chmod data on CIFS is useless anyway. I should brain a bit on
        // the "right" approach to syncing metadata to/from filesystems such as
        // these.
        let copy_result: Result<u64, std::io::Error> = if self.fs_features == FsFeatures::Samba {
            // TODO: Should I just implement my own std::fs::copy replacement
            // using io::copy like it does and just skip the perms?
            //
            // I'll sleep on it first.
            self.copy_file_chunked(uuid, &source_path, &dest_path, metadata.size)
                .await
        } else if metadata.size < FAST_COPY_THRESHOLD {
            std::fs::copy(&source_path, &dest_path)
        } else {
            // Slow path for large files - use chunked copy with progress
            self.copy_file_chunked(uuid, &source_path, &dest_path, metadata.size)
                .await
        };

        match copy_result {
            Ok(bytes_copied) => {
                tracing::trace!(
                    "cp complete: {} ({} bytes)",
                    dest_path.display(),
                    bytes_copied
                );

                // apply/sync/pray metadata is correct
                if let Err(e) = metadata.apply_to(&dest_path).await {
                    let error_msg = format!("failed to apply metadata: {}", e);
                    tracing::error!("{}: {}", error_msg, dest_path.display());
                    let mut errors = self.errors.lock().await;
                    errors.push(IoError::destination(error_msg, dest_path.clone()));
                }

                self.update_file_progress(uuid, bytes_copied, metadata.size);
                Ok(())
            }
            Err(e) => {
                let error_msg = format!("failed to copy file: {}", e);
                tracing::error!(
                    "{}: {} -> {}",
                    error_msg,
                    source_path.display(),
                    dest_path.display()
                );
                let mut errors = self.errors.lock().await;
                errors.push(IoError::destination(error_msg, dest_path));
                Err(Box::new(e))
            }
        }
    }

    /// Copy a file using fast blocking I/O with progress tracking.
    // TODO: I need to think about using https://crates.io/crates/bytecraft for
    // this chunked copying once I get inter node copying working.
    async fn copy_file_chunked(
        &self,
        uuid: u128,
        source_path: &std::path::Path,
        dest_path: &std::path::Path,
        total_size: u64,
    ) -> Result<u64, std::io::Error> {
        let source = source_path.to_path_buf();
        let dest = dest_path.to_path_buf();
        let progress = self.progress.clone();

        // tokio task to periodically update progress
        let progress_task = {
            let dest = dest.clone();
            let progress = progress.clone();
            tokio::spawn(async move {
                let mut last_size = 0u64;
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

                    // This is a stupid af hack, stat() the file and use that to
                    // update size data instead of abuse async code to write out
                    // files chunked.
                    if let Ok(metadata) = tokio::fs::metadata(&dest).await {
                        let current_size = metadata.len();
                        let new_bytes = current_size.saturating_sub(last_size);

                        if new_bytes > 0 {
                            let atomic_progress = progress.get_or_create(uuid);
                            atomic_progress.record_write(new_bytes);
                            last_size = current_size;
                        }

                        // Right now, I dont handle if sizes change during copy.
                        if current_size >= total_size {
                            break;
                        }
                    } else {
                        // Files not there bra, retry
                        continue;
                    }
                }
            })
        };

        // Do the actual copy in a blocking task using std::io::copy. Benchmarked
        // way better than async hacks did on macos/linux; bytecraft as
        // mentioned above might be an option
        let copy_result = tokio::task::spawn_blocking(move || -> std::io::Result<u64> {
            use std::io::Write;

            let mut source_file = std::fs::File::open(&source)?;
            let mut dest_file = std::fs::File::create(&dest)?;

            // Note, because this uses underlying vfs hacks, iff the filesystem
            // is COW this can avoid actually copying data.
            let bytes_copied = std::io::copy(&mut source_file, &mut dest_file)?;

            // Hopefully the device driver listens....
            dest_file.flush()?;
            dest_file.sync_all()?;

            Ok(bytes_copied)
        })
        .await;

        // TODO: Ok(Ok()) future me make it right task. spawn_blocking interface is kinda ass ngl.
        let bytes_copied = match copy_result {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
        };

        // copy should be done by here cancel the task
        progress_task.abort();

        let atomic_progress = self.progress.get_or_create(uuid);
        let current_bytes = atomic_progress.bytes_written.load(Ordering::Relaxed);
        if current_bytes < total_size {
            atomic_progress.record_write(total_size - current_bytes);
        }

        Ok(bytes_copied)
    }

    #[cfg(unix)]
    async fn create_symlink(
        &self,
        uuid: u128,
        relative_path: PathBuf,
        metadata: super::metadata::SymlinkMetadata,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dest_path = self.dest.join(&relative_path);

        // Ensure parent directory exists
        if let Some(parent) = dest_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                let error_msg = format!("failed to create parentage for symlink: {}", e);
                tracing::error!("{}: {}", error_msg, parent.display());
                let mut errors = self.errors.lock().await;
                errors.push(IoError::destination(error_msg, parent.to_path_buf()));
                return Err(Box::new(e));
            }
        }

        tracing::trace!(
            "ln -s {} -> {}",
            dest_path.display(),
            metadata.target.display()
        );

        // Remove existing symlink/file if it exists first. Future me be less derp.
        if tokio::fs::symlink_metadata(&dest_path).await.is_ok() {
            let _ = tokio::fs::remove_file(&dest_path).await;
        }

        match tokio::fs::symlink(&metadata.target, &dest_path).await {
            Ok(_) => {
                let atomic_progress = self.progress.get_or_create(uuid);
                atomic_progress
                    .files_written
                    .fetch_add(1, Ordering::Relaxed);

                tracing::trace!(
                    "symlink created: {} -> {}",
                    dest_path.display(),
                    metadata.target.display()
                );
                Ok(())
            }
            Err(e) => {
                let error_msg = format!("failed to create symlink: {}", e);
                tracing::error!(
                    "{}: {} -> {}",
                    error_msg,
                    dest_path.display(),
                    metadata.target.display()
                );
                let mut errors = self.errors.lock().await;
                errors.push(IoError::destination(error_msg, dest_path));
                Err(Box::new(e))
            }
        }
    }

    #[cfg(not(unix))]
    async fn create_symlink(
        &self,
        uuid: u128,
        relative_path: PathBuf,
        metadata: super::metadata::SymlinkMetadata,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dest_path = self.dest.join(&relative_path);

        if let Some(parent) = dest_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                let error_msg = format!("failed to create parent directory for symlink: {}", e);
                tracing::error!("{}: {}", error_msg, parent.display());
                let mut errors = self.errors.lock().await;
                errors.push(IoError::destination(error_msg, parent.to_path_buf()));
                return Err(Box::new(e));
            }
        }

        tracing::trace!(
            "creating symlink {} -> {}",
            dest_path.display(),
            metadata.target.display()
        );

        // Remove existing symlink/file if it exists? This is mostly copy/pasted code to let things build on like windows. None of this craps tested.
        if tokio::fs::symlink_metadata(&dest_path).await.is_ok() {
            let _ = tokio::fs::remove_file(&dest_path).await;
        }

        // On Windows, use symlink_file or symlink_dir
        let is_dir = tokio::fs::metadata(&metadata.target)
            .await
            .map(|m| m.is_dir())
            .unwrap_or(false);

        // ? I don't know how symlinks work on ntfs/windows this seemed to
        // build/cross compile at least without errors compared to the "unix"
        // way. God help anyone that tries to use this.
        #[cfg(windows)]
        let result = if is_dir {
            tokio::fs::symlink_dir(&metadata.target, &dest_path).await
        } else {
            tokio::fs::symlink_file(&metadata.target, &dest_path).await
        };

        // Hell if I know what to do at this point.
        #[cfg(not(windows))]
        let result: std::io::Result<()> = Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "symlink not supported on this platform",
        ));

        match result {
            Ok(_) => {
                let atomic_progress = self.progress.get_or_create(uuid);
                atomic_progress
                    .files_written
                    .fetch_add(1, Ordering::Relaxed);

                tracing::trace!(
                    "symlink created: {} -> {}",
                    dest_path.display(),
                    metadata.target.display()
                );
                Ok(())
            }
            Err(e) => {
                let error_msg = format!("sailed to create symlink: {}", e);
                tracing::error!(
                    "{}: {} -> {}",
                    error_msg,
                    dest_path.display(),
                    metadata.target.display()
                );
                let mut errors = self.errors.lock().await;
                errors.push(IoError::destination(error_msg, dest_path));
                Err(Box::new(e))
            }
        }
    }

    async fn apply_metadata(
        &self,
        _uuid: u128,
        relative_path: PathBuf,
        metadata: FileMetadata,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dest_path = self.dest.join(&relative_path);

        match metadata.apply_to(&dest_path).await {
            Ok(_) => Ok(()),
            Err(e) => {
                let error_msg = format!("failed to apply metadata: {}", e);
                tracing::error!("{}: {}", error_msg, dest_path.display());
                let mut errors = self.errors.lock().await;
                errors.push(IoError::destination(error_msg, dest_path));
                Err(Box::new(e))
            }
        }
    }

    pub async fn shutdown(&self) {
        tracing::info!("shutting down writer pool - signaling workers to exit");
        let mut shutdown = self.shutdown.lock().await;
        *shutdown = true;
    }
}
