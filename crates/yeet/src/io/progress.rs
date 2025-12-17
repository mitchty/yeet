use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Atomic progress counters for a single operation
#[derive(Debug)]
pub struct AtomicOperationProgress {
    /// Number of directories discovered during traversal
    pub dirs_found: AtomicU64,

    /// Number of files discovered during traversal
    pub files_found: AtomicU64,

    /// Total size of all files discovered (bytes)
    pub total_size: AtomicU64,

    /// Number of directories created on destination
    pub dirs_written: AtomicU64,

    /// Number of files written to destination
    pub files_written: AtomicU64,

    /// Total bytes written so far
    pub bytes_written: AtomicU64,

    /// Number of special files skipped (FIFOs, sockets, devices, etc.)
    pub skipped_count: AtomicU64,

    /// Timestamp of first write (microseconds since UNIX_EPOCH, 0 = not started)
    pub first_write_time_us: AtomicU64,

    /// Timestamp of last write (microseconds since UNIX_EPOCH)
    pub last_write_time_us: AtomicU64,
}

impl AtomicOperationProgress {
    pub fn new() -> Self {
        Self {
            dirs_found: AtomicU64::new(0),
            files_found: AtomicU64::new(0),
            total_size: AtomicU64::new(0),
            dirs_written: AtomicU64::new(0),
            files_written: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
            skipped_count: AtomicU64::new(0),
            first_write_time_us: AtomicU64::new(0),
            last_write_time_us: AtomicU64::new(0),
        }
    }

    /// Record a write operation (updates timestamps and counters atomically)
    pub fn record_write(&self, bytes: u64) {
        use std::sync::atomic::Ordering;

        // Get current time in microseconds
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;

        // Set first_write_time iff first write (cmp && xchg)
        let _ = self.first_write_time_us.compare_exchange(
            0,
            now_us,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );

        self.last_write_time_us.store(now_us, Ordering::Relaxed);
        self.bytes_written.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Get a snapshot of current progress call time.
    pub fn snapshot(&self) -> OperationProgress {
        let first_write_us = self.first_write_time_us.load(Ordering::Relaxed);
        let last_write_us = self.last_write_time_us.load(Ordering::Relaxed);
        let bytes_written = self.bytes_written.load(Ordering::Relaxed);

        // Calculate throughput (bytes per second)
        let throughput_bps = if first_write_us > 0 && last_write_us > first_write_us {
            let elapsed_us = last_write_us - first_write_us;
            let elapsed_secs = elapsed_us as f64 / 1_000_000.0;
            if elapsed_secs > 0.0 {
                bytes_written as f64 / elapsed_secs
            } else {
                0.0
            }
        } else {
            0.0
        };

        OperationProgress {
            dirs_found: self.dirs_found.load(Ordering::Relaxed),
            files_found: self.files_found.load(Ordering::Relaxed),
            total_size: self.total_size.load(Ordering::Relaxed),
            dirs_written: self.dirs_written.load(Ordering::Relaxed),
            files_written: self.files_written.load(Ordering::Relaxed),
            bytes_written,
            skipped_count: self.skipped_count.load(Ordering::Relaxed),
            throughput_bps,
            last_update: None,
        }
    }
}

impl Default for AtomicOperationProgress {
    fn default() -> Self {
        Self::new()
    }
}

// TODO: do I want to unify this struct with ^^^? I feel like having it separate
// will help with ownership more though. Not an issue for now.
/// Progress information for a single operation (snapshot)
#[derive(Debug, Clone, Default)]
pub struct OperationProgress {
    /// Number of directories discovered during traversal
    pub dirs_found: u64,

    /// Number of files discovered during traversal
    pub files_found: u64,

    /// Total size of all files discovered (bytes)
    pub total_size: u64,

    /// Number of directories created on destination
    pub dirs_written: u64,

    /// Number of files written to destination
    pub files_written: u64,

    /// Total bytes written so far
    pub bytes_written: u64,

    /// Number of special files skipped (FIFOs, sockets, devices, etc.)
    pub skipped_count: u64,

    /// Write throughput in bytes per second (calculated from first to last write)
    pub throughput_bps: f64,

    /// Last update time (for rate limiting)
    last_update: Option<Instant>,
}

impl OperationProgress {
    pub fn new() -> Self {
        Self::default()
    }

    // Only update at the fastest every ~10hz to save on useless syscalls
    pub fn should_update(&self) -> bool {
        match self.last_update {
            None => true,
            Some(last) => last.elapsed() >= Duration::from_millis(100),
        }
    }

    pub fn mark_updated(&mut self) {
        self.last_update = Some(Instant::now());
    }

    pub fn inc_dirs_found(&mut self) {
        self.dirs_found += 1;
    }

    pub fn inc_files_found(&mut self) {
        self.files_found += 1;
    }

    pub fn add_size(&mut self, size: u64) {
        self.total_size += size;
    }

    pub fn inc_dirs_written(&mut self) {
        self.dirs_written += 1;
    }

    pub fn inc_files_written(&mut self) {
        self.files_written += 1;
    }

    pub fn add_bytes_written(&mut self, bytes: u64) {
        self.bytes_written += bytes;
    }

    pub fn inc_skipped(&mut self) {
        self.skipped_count += 1;
    }

    /// Get completion percentage (0.0 to 1.0)
    /// Note: this can go down as things are scanned initially.
    pub fn completion_ratio(&self) -> f64 {
        if self.total_size == 0 {
            if self.files_found == 0 {
                return 0.0;
            }
            return self.files_written as f64 / self.files_found as f64;
        }
        self.bytes_written as f64 / self.total_size as f64
    }

    // Ditto ^^^ Not a huge deal just looks odd seeing the % go from like 50% to
    // 3% in the monitor.
    /// Get percentage complete (0-100)
    pub fn completion_percent(&self) -> f64 {
        self.completion_ratio() * 100.0
    }
}

/// Per sync UUID progress tracker. Gated by a Mutex for hash operations only.
/// Future me fix it.
#[derive(Debug, Clone)]
pub struct Progress {
    /// Progress tracking per UUID using `Arc<AtomicOperationProgress>`
    /// Mutex only guards the HashMap, not the atomic counters
    operations: Arc<parking_lot::Mutex<HashMap<u128, Arc<AtomicOperationProgress>>>>,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            operations: Arc::new(parking_lot::Mutex::new(HashMap::new())),
        }
    }
}

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_create(&self, uuid: u128) -> Arc<AtomicOperationProgress> {
        let mut ops = self.operations.lock();
        ops.entry(uuid)
            .or_insert_with(|| Arc::new(AtomicOperationProgress::new()))
            .clone()
    }

    pub fn get(&self, uuid: u128) -> Option<OperationProgress> {
        let ops = self.operations.lock();
        ops.get(&uuid)
            .map(|atomic_progress| atomic_progress.snapshot())
    }

    /// Check if a uuid operation is complete, for now all files written. There
    /// should be more logic around this as edge cases are going to suck for
    /// some of this.
    pub fn is_complete(&self, uuid: u128) -> bool {
        if let Some(progress) = self.get(uuid) {
            progress.files_found > 0 && progress.files_found == progress.files_written
        } else {
            false
        }
    }
}
