use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use super::work::WorkItem;

// TODO: This is the stupidest approach I could think of for now.
//
// This DAG/Tree approach isn't ideal cache wise though so I should revisit this
// at some point if speed is a concern. Have to base this off profiling not
// feelings/intuition.
//
// The current worker implementation leaves a lot to be desired and could use
// some brain power to making it better in specific cases like initial scan/copy.
//
// E.g. right now workers will block on parent work so a large parent dir can
// "block" children doing work needlessly if say getdents() takes a long time.
//
// While I can't easily control how fast say an ext4 fs with 4 billion files in
// it returns all of its entries I should be able to do work in branches
// under/over the long af getdents call.
/// Tree-aware work queue that ensures parent directories are created before children
#[derive(Debug)]
pub struct TreeWorkQueue {
    /// Ready directory work (parent dir exists or is root)
    ready_dirs: VecDeque<WorkItem>,

    // TODO: Need to have a way to control how files are prioritized for this
    // deque... size && date first maybe? Prioritize new/small crap over
    // older/larger seems an ok approach methinks mayhaps.
    /// Ready file work (parent dir is scanned and created)
    ready_files_priority: VecDeque<WorkItem>,

    /// Ready bulk file work
    ready_files_bulk: VecDeque<WorkItem>,

    /// Pending work blocked on parent directory creation
    /// Key: parent directory path, Value: list of child work items
    blocked_on_parent: HashMap<PathBuf, Vec<WorkItem>>,

    /// Directories that have been scanned (reader finished)
    scanned_dirs: HashSet<PathBuf>,

    /// Directories that have been created (worker finished)
    created_dirs: HashSet<PathBuf>,

    /// Total items received (for debugging)
    total_received: usize,

    /// Total items completed (for debugging)
    _total_completed: usize,

    /// Whether scanning is complete
    scan_complete: bool,
}

impl TreeWorkQueue {
    /// Create a new tree-aware work queue
    pub fn new() -> Self {
        let mut scanned_dirs = HashSet::new();
        let mut created_dirs = HashSet::new();

        // Pre-mark root as scanned and created so root-level items are immediately ready
        scanned_dirs.insert(PathBuf::from(""));
        created_dirs.insert(PathBuf::from(""));

        Self {
            ready_dirs: VecDeque::new(),
            ready_files_priority: VecDeque::new(),
            ready_files_bulk: VecDeque::new(),
            blocked_on_parent: HashMap::new(),
            scanned_dirs,
            created_dirs,
            total_received: 0,
            _total_completed: 0,
            scan_complete: false,
        }
    }

    /// Add a work item from the reader (via channel)
    /// This processes sentinels and manages the tree structure
    pub fn push(&mut self, item: WorkItem) {
        match &item {
            WorkItem::DirectoryScanned { dest_path, .. } => {
                // Mark directory as scanned - children can now be unblocked
                self.scanned_dirs.insert(dest_path.clone());
                tracing::trace!("Directory scanned: {}", dest_path.display());

                // Unblock any children waiting for this directory
                self.unblock_children(dest_path);
                return;
            }
            WorkItem::ScanComplete { .. } => {
                self.scan_complete = true;
                tracing::info!("Scan complete - {} items received", self.total_received);
                return;
            }
            _ => {
                self.total_received += 1;
            }
        }

        // Check if this work item is ready to be processed
        if self.is_ready(&item) {
            self.enqueue_ready(item);
        } else {
            // Block on parent directory
            if let Some(parent) = item.parent_path() {
                tracing::trace!(
                    "Blocking {} on parent {}",
                    item.dest_path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                    parent.display()
                );
                self.blocked_on_parent.entry(parent).or_default().push(item);
            } else {
                // No parent (root) - always ready
                self.enqueue_ready(item);
            }
        }
    }

    /// Check if a work item is ready to be processed
    fn is_ready(&self, item: &WorkItem) -> bool {
        match item {
            WorkItem::CreateDir { dest_path, .. } => {
                // Directory is ready if parent is created (or it's root)
                if let Some(parent) = dest_path.parent() {
                    if parent == std::path::Path::new("") {
                        return true; // Root level
                    }
                    self.created_dirs.contains(parent)
                } else {
                    true // Root directory
                }
            }
            _ => {
                // Files/symlinks ready if parent is both scanned AND created
                if let Some(parent) = item.parent_path() {
                    if parent == std::path::Path::new("") {
                        return true; // Root level
                    }
                    self.scanned_dirs.contains(&parent) && self.created_dirs.contains(&parent)
                } else {
                    true
                }
            }
        }
    }

    /// Enqueue a ready work item into the appropriate queue
    fn enqueue_ready(&mut self, item: WorkItem) {
        if item.is_dir() {
            self.ready_dirs.push_back(item);
        } else if item.is_bulk() {
            self.ready_files_bulk.push_back(item);
        } else {
            self.ready_files_priority.push_back(item);
        }
    }

    /// Unblock children after a directory is scanned or created
    fn unblock_children(&mut self, dir_path: &std::path::Path) {
        if let Some(children) = self.blocked_on_parent.remove(dir_path) {
            tracing::trace!(
                "Unblocking {} children of {}",
                children.len(),
                dir_path.display()
            );
            for child in children {
                if self.is_ready(&child) {
                    self.enqueue_ready(child);
                } else {
                    // Still blocked - reinsert
                    if let Some(parent) = child.parent_path() {
                        self.blocked_on_parent
                            .entry(parent)
                            .or_default()
                            .push(child);
                    }
                }
            }
        }
    }

    /// Mark a directory as created by a worker - unblocks subdirectories
    pub fn mark_dir_created(&mut self, dir_path: PathBuf) {
        self.created_dirs.insert(dir_path.clone());
        self.unblock_children(&dir_path);
    }

    /// Pop work - prioritizes directories, then files
    pub fn pop(&mut self) -> Option<WorkItem> {
        self.ready_dirs
            .pop_front()
            .or_else(|| self.ready_files_priority.pop_front())
            .or_else(|| self.ready_files_bulk.pop_front())
    }

    /// Pop batch with interleaved dirs and files
    /// Uses ratio: 1 dir : 4 files to ensure dirs created just-in-time
    /// but most time spent copying files
    pub fn pop_batch(&mut self, batch_size: usize) -> Vec<WorkItem> {
        let mut batch = Vec::with_capacity(batch_size);

        // Interleave: for every directory, try to get 4 files
        // This ensures directories are created as needed without blocking file I/O
        while batch.len() < batch_size {
            let mut made_progress = false;

            // Get 1 directory if available
            if let Some(item) = self.ready_dirs.pop_front() {
                batch.push(item);
                made_progress = true;
            }

            // Get up to 4 files (priority first, then bulk)
            for _ in 0..4 {
                if batch.len() >= batch_size {
                    break;
                }

                if let Some(item) = self
                    .ready_files_priority
                    .pop_front()
                    .or_else(|| self.ready_files_bulk.pop_front())
                {
                    batch.push(item);
                    made_progress = true;
                } else {
                    break;
                }
            }

            // If we didn't make any progress, we're done
            if !made_progress {
                break;
            }
        }

        batch
    }

    /// Check if all ready queues are empty
    pub fn is_empty(&self) -> bool {
        self.ready_dirs.is_empty()
            && self.ready_files_priority.is_empty()
            && self.ready_files_bulk.is_empty()
    }

    /// Check if we're truly complete (scan done, no ready/blocked work)
    pub fn is_complete(&self) -> bool {
        self.scan_complete && self.is_empty() && self.blocked_on_parent.is_empty()
    }

    /// Get stats for debugging
    pub fn stats(&self) -> QueueStats {
        QueueStats {
            ready_dirs: self.ready_dirs.len(),
            ready_files: self.ready_files_priority.len() + self.ready_files_bulk.len(),
            blocked: self.blocked_on_parent.values().map(|v| v.len()).sum(),
            total_received: self.total_received,
            scan_complete: self.scan_complete,
        }
    }
}

impl Default for TreeWorkQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Queue statistics for debugging
#[derive(Debug, Clone)]
pub struct QueueStats {
    pub ready_dirs: usize,
    pub ready_files: usize,
    pub blocked: usize,
    pub total_received: usize,
    pub scan_complete: bool,
}
