use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::LARGE_FILE_THRESHOLD;
use super::error::IoError;
use super::exclude::ExcludeRules;
use super::metadata::{DirMetadata, FileMetadata};
use super::progress::Progress;
use super::work::WorkItem;

// Reduce the amount of atomic updates
const PROGRESS_UPDATE_INTERVAL: u64 = 1000;

/// Reader pool that traverses the source directory and queues work
pub struct ReaderPool {
    uuid: u128,
    source: PathBuf,
    work_tx: tokio::sync::mpsc::UnboundedSender<WorkItem>,
    progress: Progress,
    errors: Arc<Mutex<Vec<IoError>>>,
    shutdown: Arc<Mutex<bool>>,
    done: Arc<Mutex<bool>>,
    exclude_rules: ExcludeRules,
}

impl ReaderPool {
    pub fn new(
        uuid: u128,
        source: PathBuf,
        work_tx: tokio::sync::mpsc::UnboundedSender<WorkItem>,
        progress: Progress,
        errors: Arc<Mutex<Vec<IoError>>>,
        done: Arc<Mutex<bool>>,
    ) -> Self {
        Self {
            uuid,
            source,
            work_tx,
            progress,
            errors,
            shutdown: Arc::new(Mutex::new(false)),
            done,
            exclude_rules: ExcludeRules::new(),
        }
    }

    pub async fn start(self: Arc<Self>) {
        let pool = self.clone();
        let uuid = pool.uuid;
        let source = pool.source.clone();

        tokio::spawn(async move {
            tracing::debug!(
                "{} reader pool starting for path {}",
                uuid::Uuid::from_u128(uuid),
                source.display()
            );
            if let Err(e) = pool.clone().run().await {
                tracing::error!("{} reader pool error: {}", uuid::Uuid::from_u128(uuid), e);
                let mut errors = pool.errors.lock().await;
                errors.push(IoError::source(
                    format!("reader pool error: {}", e),
                    pool.source.clone(),
                ));
            }
            tracing::debug!(
                "reader pool finished for UUID {}",
                uuid::Uuid::from_u128(uuid)
            );
        });
    }

    /// Main reader loop
    async fn run(self: Arc<Self>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let root = self.source.clone();
        tracing::debug!("reader traversing root: {}", root.display());

        let mut local_dirs_found = 0u64;
        let mut local_files_found = 0u64;
        let mut local_total_size = 0u64;
        let mut local_skipped = 0u64;
        let mut items_since_update = 0u64;

        // Run traversal in blocking task (uses std::fs, not tokio::fs that was slow af)
        // Work items are sent via channel immediately as they're discovered to
        // keep writers busy.
        let pool = self.clone();
        let result = tokio::task::spawn_blocking(
            move || -> Result<(u64, u64, u64, u64), Box<dyn std::error::Error + Send + Sync>> {
                pool.traverse_directory_blocking(
                    root.clone(),
                    PathBuf::new(),
                    &mut local_dirs_found,
                    &mut local_files_found,
                    &mut local_total_size,
                    &mut local_skipped,
                    &mut items_since_update,
                )?;
                Ok((
                    local_dirs_found,
                    local_files_found,
                    local_total_size,
                    local_skipped,
                ))
            },
        )
        .await;

        match result {
            Ok(Ok((dirs, files, size, skipped))) => {
                let atomic_progress = self.progress.get_or_create(self.uuid);
                atomic_progress
                    .dirs_found
                    .store(dirs, std::sync::atomic::Ordering::Relaxed);
                atomic_progress
                    .files_found
                    .store(files, std::sync::atomic::Ordering::Relaxed);
                atomic_progress
                    .total_size
                    .store(size, std::sync::atomic::Ordering::Relaxed);
                atomic_progress
                    .skipped_count
                    .store(skipped, std::sync::atomic::Ordering::Relaxed);

                local_dirs_found = dirs;
                local_files_found = files;
                local_total_size = size;
                local_skipped = skipped;
            }
            Ok(Err(e)) => {
                tracing::error!("reader traversal error: {}", e);
                let mut errors = self.errors.lock().await;
                errors.push(IoError::source(
                    format!("reader traversal error: {}", e),
                    self.source.clone(),
                ));
            }
            Err(e) => {
                tracing::error!("reader task panicked: {}", e);
                let mut errors = self.errors.lock().await;
                // TODO: I need a better error approach in general
                errors.push(IoError::source(
                    format!("reader task panic: {}", e),
                    self.source.clone(),
                ));
            }
        }

        // TODO: Add duration logging?
        tracing::info!(
            "reader traversal complete: {} dirs, {} files, {} skipped, {} bytes total",
            local_dirs_found,
            local_files_found,
            local_skipped,
            local_total_size
        );

        // Send the sentinel enum
        self.send_scan_complete();

        let mut done = self.done.lock().await;
        *done = true;
        tracing::debug!("{} reader complete", uuid::Uuid::from_u128(self.uuid));

        Ok(())
    }

    /// Traverse a directory using blocking I/O (std::fs)
    /// Much faster than async I/O for scanning that spent too much time in mutex bs
    fn traverse_directory_blocking(
        &self,
        source_path: PathBuf,
        relative_path: PathBuf,
        local_dirs_found: &mut u64,
        local_files_found: &mut u64,
        local_total_size: &mut u64,
        local_skipped: &mut u64,
        items_since_update: &mut u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::trace!(
            "traversing directory: {} rel: {}",
            source_path.display(),
            relative_path.display()
        );

        *local_dirs_found += 1;
        *items_since_update += 1;

        let dir_metadata = match self.get_dir_metadata_blocking(&source_path) {
            Ok(m) => m,
            Err(e) => {
                let error_msg = format!("failed to read directory metadata: {}", e);
                tracing::error!("{}: {}", error_msg, source_path.display());
                return Ok(());
            }
        };

        // Send directory creation data to enable any blocked workers to do work
        if let Err(e) = self.work_tx.send(WorkItem::CreateDir {
            uuid: self.uuid,
            source_path: source_path.clone(),
            dest_path: relative_path.clone(),
            metadata: dir_metadata,
        }) {
            tracing::error!("failed to send WorkItem::CreateDir: {}", e);
        }

        let entries = match std::fs::read_dir(&source_path) {
            Ok(e) => e,
            Err(e) => {
                let error_msg = format!("failed to read directory: {}", e);
                tracing::error!("{}: {}", error_msg, source_path.display());
                return Ok(());
            }
        };

        // First pass: process files and symlinks in this directory only
        // Collect subdirectories for second pass usage
        let mut subdirs = Vec::new();

        for entry in entries {
            let entry = entry?;
            let entry_path = entry.path();
            let file_name = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(_) => continue, // Skip invalid UTF-8 names? How do I handle files with names I can't create? Sounds like a future human problem.
            };

            let entry_relative = relative_path.join(&file_name);

            let metadata = match self.get_file_metadata_blocking(&entry_path) {
                Ok(m) => m,
                Err(e) => {
                    let error_msg = format!("failed to read metadata: {}", e);
                    tracing::error!("{}: {}", error_msg, entry_path.display());
                    continue;
                }
            };

            use super::metadata::FileKind;

            match metadata.kind {
                FileKind::Special => {
                    // Skip special files (FIFO, socket, device, etc.)
                    tracing::warn!("skipping special file type: {}", entry_path.display());
                    self.skip_special_file_blocking(entry_path, &metadata, local_skipped);
                    *items_since_update += 1;
                }
                FileKind::Symlink => {
                    // Handle symlink - don't follow it!
                    self.enqueue_symlink_blocking(entry_path, entry_relative, local_files_found);
                    *items_since_update += 1;
                }
                FileKind::Directory => {
                    if self.exclude_rules.should_exclude_dir(&file_name) {
                        tracing::info!("excluding directory: {}", entry_path.display());
                        *local_skipped += 1;
                        *items_since_update += 1;
                    } else {
                        subdirs.push((entry_path, entry_relative));
                    }
                }
                FileKind::File => {
                    if self.exclude_rules.should_exclude_file(&file_name) {
                        tracing::info!("excluding file: {}", entry_path.display());
                        *local_skipped += 1;
                        *items_since_update += 1;
                    } else {
                        self.enqueue_file_blocking(
                            entry_path,
                            entry_relative,
                            metadata,
                            local_files_found,
                            local_total_size,
                        );
                        *items_since_update += 1;
                    }
                }
                FileKind::Unknown => {
                    // Skip unknown file types
                    tracing::warn!("skipping unknown file type: {}", entry_path.display());
                    *local_skipped += 1;
                    *items_since_update += 1;
                }
            }
        }

        // Send sentinel: this directory's immediate contents are now queued for doing wrok
        // This allows the tree queue to mark children as ready and a worker to start doing crap for this dir.
        self.send_directory_scanned(relative_path.clone());

        if *items_since_update >= PROGRESS_UPDATE_INTERVAL {
            self.update_progress_blocking(
                *local_dirs_found,
                *local_files_found,
                *local_total_size,
                *local_skipped,
            );
            *items_since_update = 0;
        }

        // Second pass: recursively process subdirectories.
        // TODO: Basically BFS, Make this DFS too so users can choose leaf vfs branch syncing? Seems silly to bother
        for (subdir_path, subdir_relative) in subdirs {
            self.traverse_directory_blocking(
                subdir_path,
                subdir_relative,
                local_dirs_found,
                local_files_found,
                local_total_size,
                local_skipped,
                items_since_update,
            )?;
        }

        Ok(())
    }

    fn get_file_metadata_blocking(&self, path: &std::path::Path) -> std::io::Result<FileMetadata> {
        use super::metadata::FileKind;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;

        let metadata = std::fs::symlink_metadata(path)?;
        let file_type = metadata.file_type();

        let kind = if file_type.is_symlink() {
            FileKind::Symlink
        } else if file_type.is_dir() {
            FileKind::Directory
        } else if file_type.is_file() {
            FileKind::File
        } else {
            FileKind::Special
        };

        #[cfg(unix)]
        {
            Ok(FileMetadata {
                path: path.to_path_buf(),
                size: metadata.len(),
                mode: metadata.mode(),
                uid: metadata.uid(),
                gid: metadata.gid(),
                kind,
                mtime: metadata.mtime() as u64,
            })
        }
        #[cfg(not(unix))]
        {
            Ok(FileMetadata {
                path: path.to_path_buf(),
                size: metadata.len(),
                kind,
                mtime: metadata
                    .modified()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            })
        }
    }

    fn get_dir_metadata_blocking(&self, path: &std::path::Path) -> std::io::Result<DirMetadata> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;

        #[cfg(unix)]
        let metadata = std::fs::metadata(path)?;
        #[cfg(not(unix))]
        let _metadata = std::fs::metadata(path)?;
        #[cfg(unix)]
        {
            Ok(DirMetadata {
                path: path.to_path_buf(),
                mode: metadata.mode(),
                uid: metadata.uid(),
                gid: metadata.gid(),
            })
        }
        #[cfg(not(unix))]
        {
            Ok(DirMetadata {
                path: path.to_path_buf(),
            })
        }
    }

    fn enqueue_file_blocking(
        &self,
        source_path: PathBuf,
        relative_path: PathBuf,
        metadata: FileMetadata,
        local_files_found: &mut u64,
        local_total_size: &mut u64,
    ) {
        *local_files_found += 1;
        *local_total_size += metadata.size;

        let work_item = if metadata.size >= LARGE_FILE_THRESHOLD {
            WorkItem::CopyLargeFile {
                uuid: self.uuid,
                source_path,
                dest_path: relative_path,
                metadata,
            }
        } else {
            WorkItem::CopySmallFile {
                uuid: self.uuid,
                source_path,
                dest_path: relative_path,
                metadata,
            }
        };

        if let Err(e) = self.work_tx.send(work_item) {
            tracing::error!("failed to send file work item: {}", e);
        }
    }

    fn enqueue_symlink_blocking(
        &self,
        source_path: PathBuf,
        relative_path: PathBuf,
        local_files_found: &mut u64,
    ) {
        use super::metadata::SymlinkMetadata;

        let metadata = match std::fs::read_link(&source_path) {
            Ok(target) => SymlinkMetadata {
                path: source_path.clone(),
                target,
                #[cfg(unix)]
                mode: std::fs::symlink_metadata(&source_path)
                    .ok()
                    .map(|m| {
                        use std::os::unix::fs::MetadataExt;
                        m.mode()
                    })
                    .unwrap_or(0o777),
                #[cfg(unix)]
                uid: std::fs::symlink_metadata(&source_path)
                    .ok()
                    .map(|m| {
                        use std::os::unix::fs::MetadataExt;
                        m.uid()
                    })
                    .unwrap_or(0),
                #[cfg(unix)]
                gid: std::fs::symlink_metadata(&source_path)
                    .ok()
                    .map(|m| {
                        use std::os::unix::fs::MetadataExt;
                        m.gid()
                    })
                    .unwrap_or(0),
            },
            Err(e) => {
                tracing::error!("failed to read symlink: {}: {}", source_path.display(), e);
                return;
            }
        };

        *local_files_found += 1;

        if let Err(e) = self.work_tx.send(WorkItem::CreateSymlink {
            uuid: self.uuid,
            source_path,
            dest_path: relative_path,
            metadata,
        }) {
            tracing::error!("failed to send symlink work item: {}", e);
        }
    }

    fn skip_special_file_blocking(
        &self,
        source_path: PathBuf,
        metadata: &FileMetadata,
        local_skipped: &mut u64,
    ) {
        #[cfg(unix)]
        tracing::warn!(
            "skipping special file: {} mode: {:o}",
            source_path.display(),
            metadata.mode
        );
        #[cfg(not(unix))]
        tracing::warn!("skipping special file: {}", source_path.display());
        *local_skipped += 1;
    }

    fn send_directory_scanned(&self, dest_path: PathBuf) {
        let sentinel = WorkItem::DirectoryScanned {
            uuid: self.uuid,
            dest_path,
        };
        if let Err(e) = self.work_tx.send(sentinel) {
            tracing::error!("failed to send directory fully scanned sentinel: {}", e);
        }
    }

    fn send_scan_complete(&self) {
        let sentinel = WorkItem::ScanComplete { uuid: self.uuid };
        if let Err(e) = self.work_tx.send(sentinel) {
            tracing::error!("failed to send scan complete sentinel: {}", e);
        }
    }

    fn update_progress_blocking(
        &self,
        dirs_found: u64,
        files_found: u64,
        total_size: u64,
        skipped: u64,
    ) {
        use std::sync::atomic::Ordering;
        let atomic_progress = self.progress.get_or_create(self.uuid);
        atomic_progress
            .dirs_found
            .store(dirs_found, Ordering::Relaxed);
        atomic_progress
            .files_found
            .store(files_found, Ordering::Relaxed);
        atomic_progress
            .total_size
            .store(total_size, Ordering::Relaxed);
        atomic_progress
            .skipped_count
            .store(skipped, Ordering::Relaxed);
    }

    pub async fn shutdown(&self) {
        let mut shutdown = self.shutdown.lock().await;
        *shutdown = true;
    }
}
