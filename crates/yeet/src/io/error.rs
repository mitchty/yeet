use std::path::PathBuf;
use std::time::SystemTime;

/// Represents which side of the copy operation an error occurred on
// This will be more useful once I get syncing working.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSide {
    Source,
    Destination,
}

/// Error information captured during I/O operations
// Need to make yeet monitor abuse this data and replicate it to clients
// No clue how i'll handle that output
#[derive(Debug, Clone)]
pub struct IoError {
    /// The error message or whatever is at issue
    pub error: String,

    /// When the error occurred
    pub timestamp: SystemTime,

    /// The path where the error occurred
    pub path: PathBuf,

    /// Which side of the operation failed
    pub side: ErrorSide,
}

impl IoError {
    pub fn new(error: String, path: PathBuf, side: ErrorSide) -> Self {
        Self {
            error,
            timestamp: SystemTime::now(),
            path,
            side,
        }
    }

    // Helper fn's for callers to be less stupid

    /// Create a source-side error
    pub fn source(error: String, path: PathBuf) -> Self {
        Self::new(error, path, ErrorSide::Source)
    }

    /// Create a destination-side error
    pub fn destination(error: String, path: PathBuf) -> Self {
        Self::new(error, path, ErrorSide::Destination)
    }
}

impl std::fmt::Display for IoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?}:{:?} ({}): {}",
            self.timestamp,
            self.side,
            self.path.display(),
            self.error
        )
    }
}

impl std::error::Error for IoError {}
