use std::path::PathBuf;

// Note: Any non unix blocks here are more to act as fillers of "future me or
// preferably someone that knows how the hell windows works and you might sync
// to/from it" work.
//
// I have no idea how one would truly handle syncing to/from windows and mapping
// its filesystem attributes to/from a unix filesystem. Hell not even sure how
// you would want to sync only windows<->windows for that matter. And I only
// have one windows system that I only use for other things.
//
// This is a "future mitch" task for sure.
/// File kind enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileKind {
    /// Regular file
    File,
    /// Directory
    Directory,
    /// Symbolic link
    Symlink,
    // TODO: I KNOW that a fifo technically could be synced insofar is I could
    // create a fifo but see minimal value in the idea to be honest.
    //
    // Special is here more for information purposes, its equivalent to Unknown
    /// Special files e.g. FIFO, socket, device, etc.
    Special,
    /// Unknown kind of file, will not be synced
    #[default]
    Unknown,
}

/// File metadata captured during directory traversal
#[derive(Debug, Clone)]
pub struct FileMetadata {
    /// Path to the file
    pub path: PathBuf,

    /// File size in bytes
    pub size: u64,

    /// Unix file mode e.g. 0600,0440,755....
    #[cfg(unix)]
    pub mode: u32,

    // Do I even need to care about uid/gid? Whatever user the process is
    // running at matters more. But if some yahoo runs as root I suppose I could
    // try chown()ing locally? So many edge cases to consider.
    /// Unix user id
    #[cfg(unix)]
    pub uid: u32,

    /// Unix group id
    #[cfg(unix)]
    pub gid: u32,

    /// File kind (file, directory, symlink, special, or unknown)
    pub kind: FileKind,

    /// Modified time (seconds since epoch)
    pub mtime: u64,
}

impl FileMetadata {
    /// Extract metadata from a file, does not follow symlinks
    #[cfg(unix)]
    pub async fn from_path(path: PathBuf) -> std::io::Result<Self> {
        use std::os::unix::fs::MetadataExt;

        let metadata = tokio::fs::symlink_metadata(&path).await?;

        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            FileKind::Symlink
        } else if file_type.is_dir() {
            FileKind::Directory
        } else if file_type.is_file() {
            FileKind::File
        } else {
            // FIFO, socket, block/char device, etc.
            FileKind::Special
        }; // TODO: Unknown case.

        Ok(Self {
            path,
            size: metadata.len(),
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            kind,
            mtime: metadata.mtime() as u64,
        })
    }

    #[cfg(not(unix))]
    pub async fn from_path(path: PathBuf) -> std::io::Result<Self> {
        let metadata = tokio::fs::symlink_metadata(&path).await?;

        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            FileKind::Symlink
        } else if file_type.is_dir() {
            FileKind::Directory
        } else if file_type.is_file() {
            FileKind::File
        } else {
            // On non-Unix, any non-file/non-dir/non-symlink is "special"
            FileKind::Special
        };
        // Unknown here too

        Ok(Self {
            path,
            size: metadata.len(),
            kind,
            mtime: metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        })
    }

    #[cfg(unix)]
    pub async fn apply_to(&self, dest_path: &std::path::Path) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let perms = std::fs::Permissions::from_mode(self.mode);
        tokio::fs::set_permissions(dest_path, perms).await?;

        // Set ownership (may require elevated privileges), best effort.
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::chown;
            let _ = chown(dest_path, Some(self.uid), Some(self.gid));
        }

        Ok(())
    }

    #[cfg(not(unix))]
    pub async fn apply_to(&self, _dest_path: &std::path::Path) -> std::io::Result<()> {
        // On non-Unix systems, no idea how to deal with this
        Ok(())
    }
}

// TODO: For symlinks need to think how to handle relative vs static symlinks.
//
// Also how do I want to handle symlinks that are broken? Obviously copy it
// simply and maybe let the user know.
/// Symlink metadata
#[derive(Debug, Clone)]
pub struct SymlinkMetadata {
    /// Path to the symlink
    pub path: PathBuf,

    /// Target path the symlink points to
    pub target: PathBuf,

    /// Unix mode
    #[cfg(unix)]
    pub mode: u32,

    /// Unix user id
    #[cfg(unix)]
    pub uid: u32,

    /// Unix group id
    #[cfg(unix)]
    pub gid: u32,
}

impl SymlinkMetadata {
    #[cfg(unix)]
    pub async fn from_path(path: PathBuf) -> std::io::Result<Self> {
        use std::os::unix::fs::MetadataExt;

        let target = tokio::fs::read_link(&path).await?;
        let metadata = tokio::fs::symlink_metadata(&path).await?;

        Ok(Self {
            path,
            target,
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
        })
    }

    // TODO: What the hell is a symlink on windows and how the hell would I sync
    // to<->from such an os?
    #[cfg(not(unix))]
    pub async fn from_path(path: PathBuf) -> std::io::Result<Self> {
        let target = tokio::fs::read_link(&path).await?;

        Ok(Self { path, target })
    }
}

/// Directory metadata
#[derive(Debug, Clone)]
pub struct DirMetadata {
    /// Path to the directory
    pub path: PathBuf,

    /// Unix file mode (permissions)
    #[cfg(unix)]
    pub mode: u32,

    /// Unix user id
    #[cfg(unix)]
    pub uid: u32,

    /// Unix group id
    #[cfg(unix)]
    pub gid: u32,
}

// Also.. I should really unify these three structs. I'm lazy though and want to get things to MVP first.
impl DirMetadata {
    #[cfg(unix)]
    pub async fn from_path(path: PathBuf) -> std::io::Result<Self> {
        use std::os::unix::fs::MetadataExt;

        let metadata = tokio::fs::metadata(&path).await?;

        Ok(Self {
            path,
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
        })
    }

    #[cfg(not(unix))]
    pub async fn from_path(path: PathBuf) -> std::io::Result<Self> {
        let _metadata = tokio::fs::metadata(&path).await?;

        Ok(Self { path })
    }
}
