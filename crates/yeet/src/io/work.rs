use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::path::PathBuf;

use super::metadata::{DirMetadata, FileMetadata, SymlinkMetadata};

// TODO: Need to brain up how I an have a work stealing/priority queue for this
// stuff that a user can control.
//
// My initial use case here is to ensure I can sync small crap before the
// gihugic crap and prevent work queue blocking by giant files from copying
// small stuff. Right now I'm cutting off copies at 128MiB as "large" or bulk.
//
// This distinction is kind academic for now as the data is all local but need
// to prep for transferring over a network, basically I want to stream all data over grpc as:
// - dirs[]
// - files[]
// - bulk[]
//
// With each thing being data the next bit will need aka parents for
// files/bulk[] That way I can ensure metadata/dir work is done prior to I/O.
//
// It might pay off to have two diff grpc calls for priority work over bulk too.
// Future problem to solve.
/// Priority levels for work items
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Low/Bulk priority chungus files
    Bulk = 0,

    /// Normal priority: dirs, files, symlinks, metadata etc...
    /// Directories are interleaved with files so file I/O can start as soon as
    /// entries are yeeted to the mpsc channel and be picked up for work.
    Normal = 1,
}

/// Types of work that can be queued
#[derive(Debug, Clone)]
pub enum WorkItem {
    /// Create a directory
    CreateDir {
        uuid: u128,
        source_path: PathBuf,
        dest_path: PathBuf,
        metadata: DirMetadata,
    },

    /// Copy a small file (<1MiB)
    CopySmallFile {
        uuid: u128,
        source_path: PathBuf,
        dest_path: PathBuf,
        metadata: FileMetadata,
    },

    /// Copy a large file (>=1MiB)
    CopyLargeFile {
        uuid: u128,
        source_path: PathBuf,
        dest_path: PathBuf,
        metadata: FileMetadata,
    },

    /// Create a symlink
    CreateSymlink {
        uuid: u128,
        source_path: PathBuf,
        dest_path: PathBuf,
        metadata: SymlinkMetadata,
    },

    /// Apply metadata to an existing file/directory
    ApplyMetadata {
        uuid: u128,
        dest_path: PathBuf,
        metadata: FileMetadata,
    },

    /// Sentinel: Reader has finished scanning a directory's immediate contents
    /// This allows the queue to mark children as ready for processing
    DirectoryScanned { uuid: u128, dest_path: PathBuf },

    /// Sentinel: Reader has finished all scanning work
    ScanComplete { uuid: u128 },
}

impl WorkItem {
    /// Get the priority of this work item
    pub fn priority(&self) -> Priority {
        match self {
            // Directories are Normal priority now - creates them just-in-time
            // This allows file copying to start immediately instead of waiting
            // for all directories to be created first
            WorkItem::CreateDir { .. } => Priority::Normal,
            WorkItem::ApplyMetadata { .. } => Priority::Normal,
            WorkItem::CopySmallFile { .. } => Priority::Normal,
            WorkItem::CreateSymlink { .. } => Priority::Normal,
            WorkItem::CopyLargeFile { .. } => Priority::Bulk,
            // Sentinels are not queued for workers
            WorkItem::DirectoryScanned { .. } | WorkItem::ScanComplete { .. } => Priority::Normal,
        }
    }

    /// Check if this is a bulk operation
    pub fn is_bulk(&self) -> bool {
        matches!(self, WorkItem::CopyLargeFile { .. })
    }

    /// Check if this is a directory creation task
    pub fn is_dir(&self) -> bool {
        matches!(self, WorkItem::CreateDir { .. })
    }

    /// Check if this is a sentinel (not actual work)
    pub fn is_sentinel(&self) -> bool {
        matches!(
            self,
            WorkItem::DirectoryScanned { .. } | WorkItem::ScanComplete { .. }
        )
    }

    /// Get the UUID for this work item
    pub fn uuid(&self) -> u128 {
        match self {
            WorkItem::CreateDir { uuid, .. } => *uuid,
            WorkItem::CopySmallFile { uuid, .. } => *uuid,
            WorkItem::CopyLargeFile { uuid, .. } => *uuid,
            WorkItem::CreateSymlink { uuid, .. } => *uuid,
            WorkItem::ApplyMetadata { uuid, .. } => *uuid,
            WorkItem::DirectoryScanned { uuid, .. } => *uuid,
            WorkItem::ScanComplete { uuid } => *uuid,
        }
    }

    /// Get the destination path for this work item (None for ScanComplete)
    pub fn dest_path(&self) -> Option<&std::path::Path> {
        match self {
            WorkItem::CreateDir { dest_path, .. } => Some(dest_path),
            WorkItem::CopySmallFile { dest_path, .. } => Some(dest_path),
            WorkItem::CopyLargeFile { dest_path, .. } => Some(dest_path),
            WorkItem::CreateSymlink { dest_path, .. } => Some(dest_path),
            WorkItem::ApplyMetadata { dest_path, .. } => Some(dest_path),
            WorkItem::DirectoryScanned { dest_path, .. } => Some(dest_path),
            WorkItem::ScanComplete { .. } => None,
        }
    }

    /// Get the parent directory path for this work item
    pub fn parent_path(&self) -> Option<PathBuf> {
        self.dest_path()?.parent().map(|p| p.to_path_buf())
    }
}

/// A work item with priority for the queue
#[derive(Debug, Clone)]
struct PrioritizedWork {
    priority: Priority,
    item: WorkItem,
}

impl PartialEq for PrioritizedWork {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl Eq for PrioritizedWork {}

impl PartialOrd for PrioritizedWork {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PrioritizedWork {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
    }
}

/// Priority queue for work items using a BinaryHeap
#[derive(Debug)]
pub struct WorkQueue {
    /// High priority queue (dirs, metadata, small files)
    priority_queue: BinaryHeap<PrioritizedWork>,

    /// Bulk queue (large files)
    bulk_queue: BinaryHeap<PrioritizedWork>,
}

impl WorkQueue {
    /// Create a new work queue
    pub fn new() -> Self {
        Self {
            priority_queue: BinaryHeap::new(),
            bulk_queue: BinaryHeap::new(),
        }
    }

    /// Add a work item to the queue
    pub fn push(&mut self, item: WorkItem) {
        let priority = item.priority();
        let work = PrioritizedWork { priority, item };

        if work.item.is_bulk() {
            self.bulk_queue.push(work);
        } else {
            self.priority_queue.push(work);
        }
    }

    /// Pop the highest priority work item from the priority queue
    pub fn pop_priority(&mut self) -> Option<WorkItem> {
        self.priority_queue.pop().map(|w| w.item)
    }

    /// Pop multiple priority items at once (up to batch_size)
    pub fn pop_priority_batch(&mut self, batch_size: usize) -> Vec<WorkItem> {
        let mut batch = Vec::with_capacity(batch_size);
        for _ in 0..batch_size {
            if let Some(item) = self.priority_queue.pop() {
                batch.push(item.item);
            } else {
                break;
            }
        }
        batch
    }

    /// Pop a bulk work item
    pub fn pop_bulk(&mut self) -> Option<WorkItem> {
        self.bulk_queue.pop().map(|w| w.item)
    }

    /// Pop multiple bulk items at once (up to batch_size)
    pub fn pop_bulk_batch(&mut self, batch_size: usize) -> Vec<WorkItem> {
        let mut batch = Vec::with_capacity(batch_size);
        for _ in 0..batch_size {
            if let Some(item) = self.bulk_queue.pop() {
                batch.push(item.item);
            } else {
                break;
            }
        }
        batch
    }

    /// Check if both queues are empty
    pub fn is_empty(&self) -> bool {
        self.priority_queue.is_empty() && self.bulk_queue.is_empty()
    }

    /// Get the number of items in the priority queue
    pub fn priority_len(&self) -> usize {
        self.priority_queue.len()
    }

    /// Get the number of items in the bulk queue
    pub fn bulk_len(&self) -> usize {
        self.bulk_queue.len()
    }

    /// Get total number of items in all queues
    pub fn total_len(&self) -> usize {
        self.priority_len() + self.bulk_len()
    }
}

impl Default for WorkQueue {
    fn default() -> Self {
        Self::new()
    }
}
