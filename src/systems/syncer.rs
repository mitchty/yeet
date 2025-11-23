use bevy::prelude::*;
use bevy_tokio_tasks::TokioTasksRuntime;

use crate::systems::ssh::forwarding::{Pending as ForwardingPending, Request as ForwardingRequest};
use crate::systems::ssh::pool::{
    Pending as ConnectionPending, Ref as ConnectionRef, Request as ConnectionRequest,
};
use crate::{
    Dest, IoOperation, IoProgress, RemoteHost, SimpleCopy, Source, SshForwarding, SyncComplete,
    Uuid,
};

pub struct Syncer;

// Note: Sync tasks are now handled by the IoSubsystem and IoOperation component.
// The old SyncTask component has been removed.

// The plugin/system that handles syncs from the top level
//
// Essentially this is the main() of syncing for yeet.
//
// Entrypoints are via directly adding an entity for syncing with Source & Dest
// or if the yeet process is a daemon via a gprc call.
//
// I'm abusing observers in bevy to trigger other events at next tick to do work.
//
// Note that all the "real" work is done in async tokio. e.g. sending data, file i/o etc..
//
// I could do this all in bevy but it has a limited setup for i/o type tasks in how it starts at runtime.
//
// So my solution is to implement all things i/o in background tokio async rust.
// Backoff and other logic is there too, the bevy bridge is simply checking on
// what it reports back.
//
// The tokio grpc backend also deals with feature detection of things like ebpf etc...
//
// I think... this is all not entirely fleshed out besides high level right now
// working on all the backend stuff I need more than the actual logic atm like a
// dumass.

// Note, at plugin build time all we really do is add our commands for now.
//
// In future we'll detect thigns like ebpf etc... at startup and this will act
// as a bit of a feature detection plugin to associate capabilities to the system.
//
// Adding an entity to Sync from source to dest kicks off the other work.
impl Plugin for Syncer {
    fn build(&self, app: &mut App) {
        // SSH operations require SSH pool and forwarding managers
        assert!(app.is_plugin_added::<crate::systems::ssh::Pool>());
        assert!(app.is_plugin_added::<crate::systems::ssh::Manager>());

        // Don't spam these debug messages too often, kinda silly.
        #[cfg(debug_assertions)]
        app.add_systems(
            Update,
            debug_hack.run_if(bevy::time::common_conditions::on_timer(
                std::time::Duration::from_secs(7),
            )),
        );

        app.add_systems(
            Update,
            (
                request_ssh_connections,
                request_ssh_forwarding.after(request_ssh_connections),
                spawn_sync_tasks.after(request_ssh_forwarding),
            ),
        );
    }
}

// Dump out anything that is actively syncing (or just was and isn't updated in
// this tick) when run.
//
// Note, for now we'll only deal with things with a SimpleCopy marker
#[cfg(debug_assertions)]
fn debug_hack(
    query: Query<
        (
            Entity,
            &Source,
            &Dest,
            &Uuid,
            &SimpleCopy,
            Option<&IoProgress>,
        ),
        (Without<SyncComplete>, With<IoOperation>),
    >,
) -> bevy::prelude::Result {
    let len = query.iter().count();

    if len > 0 {
        trace!("active copies: {}", len);
        for (_entity, _lhs, _rhs, _uuid, _marker, progress) in &query {
            if let Some(progress) = progress {
                if progress.files_found > 0 || progress.files_written > 0 {
                    trace!(
                        "  progress: {}/{} files, {}/{} dirs, {:.1}% complete",
                        progress.files_written,
                        progress.files_found,
                        progress.dirs_written,
                        progress.dirs_found,
                        progress.completion_percent
                    );
                }
            }
        }
    }
    Ok(())
}

fn spawn_sync_tasks(
    mut commands: Commands,
    runtime: ResMut<TokioTasksRuntime>,
    query: Query<
        (
            Entity,
            &Source,
            &Dest,
            &Uuid,
            &SimpleCopy,
            Option<&crate::NumWriters>,
        ),
        (Without<IoOperation>, Without<SyncComplete>),
    >,
) -> bevy::prelude::Result {
    for (entity, source, dest, uuid, _ignored, num_writers) in &query {
        let source = source.0.clone();
        let dest = dest.0.clone();
        let uuid = uuid.0;

        // SimpleCopy using the new I/O subsystem
        info!(
            "spawning simplecopy I/O operation {} -> {} (uuid: {})",
            source.display(),
            dest.display(),
            uuid::Uuid::from_u128(uuid)
        );

        // Create a new I/O subsystem for this operation
        let mut subsystem = crate::io::IoSubsystem::new();
        let subsystem_clone = subsystem.clone();

        // Get the number of writers (None = use CPU count)
        let writers = num_writers.and_then(|nw| nw.0);

        // Start the I/O subsystem in a tokio task using bevy_tokio_tasks
        runtime.spawn_background_task(move |_ctx| async move {
            if let Err(e) = subsystem.start(uuid, source, dest, writers).await {
                error!("I/O subsystem failed to start: {}", e);
            }
        });

        commands.entity(entity).insert((
            IoOperation {
                uuid,
                subsystem: subsystem_clone,
            },
            IoProgress::default(),
            crate::systems::protocol::SyncStartTime(std::time::Instant::now()),
        ));
    }
    Ok(())
}

// System to request SSH connections for remote syncs
fn request_ssh_connections(
    mut commands: Commands,
    query: Query<
        (Entity, &RemoteHost, &SimpleCopy),
        (
            Without<ConnectionRequest>,
            Without<ConnectionRef>,
            Without<ConnectionPending>,
        ),
    >,
) -> bevy::prelude::Result {
    for (entity, remote_host, _) in &query {
        let host_spec = remote_host.0.clone();

        info!(
            "requesting SSH connection to {} for entity {:?}",
            host_spec, entity
        );

        commands
            .entity(entity)
            .insert(ConnectionRequest { host_spec });
    }
    Ok(())
}

// System to request SSH forwarding for entities with SSH connections
fn request_ssh_forwarding(
    mut commands: Commands,
    query: Query<
        (Entity, &ConnectionRef),
        (
            Without<ConnectionRequest>,
            Without<SshForwarding>,
            Without<ForwardingPending>,
        ),
    >,
) -> bevy::prelude::Result {
    for (entity, _ssh_ref) in &query {
        let remote_port = 50051;

        info!(
            "requesting SSH forwarding for entity {:?} to port {}",
            entity, remote_port
        );

        commands
            .entity(entity)
            .insert(ForwardingRequest { remote_port });
    }
    Ok(())
}

// simplecopy function removed - now handled by IoSubsystem
