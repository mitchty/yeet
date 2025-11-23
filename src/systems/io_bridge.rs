use bevy::prelude::*;

use crate::{IoOperation, IoProgress, SimpleCopy, SyncComplete};

/// Plugin that bridges the async I/O subsystem with the Bevy ECS
pub struct IoBridge;

impl Plugin for IoBridge {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (update_io_progress, check_io_completion));
    }
}

/// System to update I/O progress from the async subsystem
/// Rate-limited to ~10Hz by the Progress struct
fn update_io_progress(
    mut query: Query<(&IoOperation, &mut IoProgress), (With<SimpleCopy>, Without<SyncComplete>)>,
) -> bevy::prelude::Result {
    for (io_op, mut progress) in &mut query {
        let subsystem = io_op.subsystem.clone();
        let uuid = io_op.uuid;

        // Block on the async operation - this is acceptable since it's just locking a mutex
        let (current_progress, error_count) = futures_lite::future::block_on(async {
            let progress = subsystem.get_progress(uuid).await;
            let errors = subsystem.error_count().await;
            (progress, errors)
        });

        if let Some(current_progress) = current_progress {
            progress.dirs_found = current_progress.dirs_found;
            progress.files_found = current_progress.files_found;
            progress.total_size = current_progress.total_size;
            progress.dirs_written = current_progress.dirs_written;
            progress.files_written = current_progress.files_written;
            progress.bytes_written = current_progress.bytes_written;
            progress.completion_percent = current_progress.completion_percent();
            progress.error_count = error_count;
            progress.skipped_count = current_progress.skipped_count;
            progress.throughput_bps = current_progress.throughput_bps;

            // Only log if there's actual progress to log keep?
            if progress.files_found > 0 || progress.files_written > 0 {
                trace!(
                    "i/o progress [{}]: {}/{} files, {}/{} dirs, {:.1}% complete, {} errors",
                    uuid::Uuid::from_u128(uuid),
                    progress.files_written,
                    progress.files_found,
                    progress.dirs_written,
                    progress.dirs_found,
                    progress.completion_percent,
                    progress.error_count
                );
            }
        }
    }
    Ok(())
}

fn check_io_completion(
    mut commands: Commands,
    query: Query<(Entity, &IoOperation), (With<SimpleCopy>, Without<SyncComplete>)>,
) -> bevy::prelude::Result {
    for (entity, io_op) in &query {
        let subsystem = io_op.subsystem.clone();
        let uuid = io_op.uuid;

        // Block on the async operation
        let is_complete =
            futures_lite::future::block_on(async move { subsystem.is_complete(uuid).await });

        if is_complete {
            info!(
                "i/o operation complete for entity {:?} (uuid: {})",
                entity,
                uuid::Uuid::from_u128(uuid)
            );

            // Get error count
            let subsystem = io_op.subsystem.clone();
            let error_count =
                futures_lite::future::block_on(async move { subsystem.error_count().await });

            if error_count > 0 {
                warn!(
                    "{} i/o operation completed with about {} errors",
                    uuid::Uuid::from_u128(uuid),
                    error_count
                );

                let subsystem = io_op.subsystem.clone();
                let errors =
                    futures_lite::future::block_on(async move { subsystem.get_errors().await });

                for error in errors.iter().take(10) {
                    warn!("  {}", error);
                }
                if error_count > 10 {
                    warn!("  and about {} more errors sucker", error_count - 10);
                }
            }

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();

            let mut subsystem = io_op.subsystem.clone();
            futures_lite::future::block_on(async move {
                subsystem.shutdown().await;
            });
            info!(
                "{} i/o subsystem shutdown complete",
                uuid::Uuid::from_u128(uuid)
            );

            commands.entity(entity).remove::<IoOperation>().insert((
                SyncComplete(now),
                crate::systems::protocol::SyncStopTime(std::time::Instant::now()),
            ));
        }
    }
    Ok(())
}
