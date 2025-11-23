use super::work::WorkItem;
use std::collections::VecDeque;

/// Simple FIFO work queue for local→local copies
/// Relies on DFS traversal order to handle dependencies naturally
#[derive(Debug)]
pub struct SimpleWorkQueue {
    /// FIFO work queue
    queue: VecDeque<WorkItem>,

    /// Total items received
    total_received: usize,

    /// Whether scanning is complete
    scan_complete: bool,
}

impl SimpleWorkQueue {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            total_received: 0,
            scan_complete: false,
        }
    }

    pub fn push(&mut self, item: WorkItem) {
        match &item {
            WorkItem::DirectoryScanned { .. } => {
                // Ignore sentinels - not needed for simple queue
                return;
            }
            WorkItem::ScanComplete { .. } => {
                self.scan_complete = true;
                tracing::debug!("scan complete: {} items received", self.total_received);
                return;
            }
            _ => {
                self.total_received += 1;
                self.queue.push_back(item);
            }
        }
    }

    pub fn pop(&mut self) -> Option<WorkItem> {
        self.queue.pop_front()
    }

    pub fn pop_batch(&mut self, batch_size: usize) -> Vec<WorkItem> {
        let mut batch = Vec::with_capacity(batch_size);
        for _ in 0..batch_size {
            if let Some(item) = self.queue.pop_front() {
                batch.push(item);
            } else {
                break;
            }
        }
        batch
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn is_complete(&self) -> bool {
        self.scan_complete && self.queue.is_empty()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    // TODO: need to think this through but can nuke later
    pub fn mark_dir_created(&mut self, _dir_path: std::path::PathBuf) {
        // Not needed for simple queue - create_dir_all handles parents
    }
}

impl Default for SimpleWorkQueue {
    fn default() -> Self {
        Self::new()
    }
}
