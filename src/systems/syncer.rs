use bevy::prelude::*;
use bevy_cronjob::prelude::*;

use crate::systems::ssh::forwarding::{Pending as ForwardingPending, Request as ForwardingRequest};
use crate::systems::ssh::pool::{
    Pending as ConnectionPending, Ref as ConnectionRef, Request as ConnectionRequest,
};
use crate::{Dest, RemoteHost, SimpleCopy, Source, SshForwarding, SyncComplete, Uuid};

pub struct Syncer;

// We'll start out abusing the builtin bevy IoTaskPool, bevy tokio tasks is ok
// but I was only wanting to use it for a grpc<->bevy bridge where it makes
// sense.
//
// But it might make sense to use that for bridging events from
// epoll()/ionotify/ebpf in future too.
//
// This is a "future mitch" task from past jerk mitch to figure out
#[derive(Component)]
struct SyncTask(bevy::tasks::Task<Result<(), String>>);

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
        // We don't add these ourselves, the caller is responsible for
        // configuring shared plugins for Syncer for now. Future me gets to
        // brain a less tacky way to do this.
        assert!(app.is_plugin_added::<bevy_cronjob::CronJobPlugin>());
        // SSH operations require SSH pool and forwarding managers
        assert!(app.is_plugin_added::<crate::systems::ssh::Pool>());
        assert!(app.is_plugin_added::<crate::systems::ssh::Manager>());

        // Don't spam these debug messages too often, kinda silly.
        app.add_systems(Update, update.run_if(schedule_passed("every 3 seconds")));

        app.add_systems(
            Update,
            (
                request_ssh_connections,
                request_ssh_forwarding.after(request_ssh_connections),
                spawn_sync_tasks.after(request_ssh_forwarding),
                check_sync_completion.after(spawn_sync_tasks),
            ),
        );
    }
}

// Dump out anything that is actively syncing (or just was and isn't updated in
// this tick) when run.
//
// Note, for now we'll only deal with things with a SimpleCopy marker
fn update(
    query: Query<
        (Entity, &Source, &Dest, &Uuid, &SimpleCopy),
        (Without<SyncComplete>, With<SyncTask>),
    >,
) -> Result {
    let len = query.iter().count();

    if len > 0 {
        debug!("simplecopies: {}", len);
    }
    // for (entity, lhs, rhs, uuid, _os) in &query {
    //     info!(
    //         "simplecopy sync {entity} {}->{} uuid {}",
    //         lhs.display(),
    //         rhs.display(),
    //         uuid::Uuid::from_u128(uuid.0)
    //     );
    // }
    Ok(())
}

fn spawn_sync_tasks(
    mut commands: Commands,
    query: Query<(Entity, &Source, &Dest, &SimpleCopy), (Without<SyncTask>, Without<SyncComplete>)>,
) -> Result {
    let task_pool = bevy::tasks::IoTaskPool::get();

    for (entity, source, dest, _ignored) in &query {
        let source = source.0.clone();
        let dest = dest.0.clone();

        // SimpleCopy is a glorified cp for now.
        info!(
            "spawning simplecopy sync task {} -> {}",
            source.display(),
            dest.display()
        );

        let task = task_pool.spawn(async move { simplecopy(source, dest).await });

        commands.entity(entity).insert((
            SyncTask(task),
            crate::systems::protocol::SyncStartTime(std::time::Instant::now()),
        ));
    }
    Ok(())
}

fn check_sync_completion(
    mut commands: Commands,
    mut query: Query<(Entity, &mut SyncTask)>,
) -> Result {
    for (entity, mut sync_task) in &mut query {
        if let Some(result) =
            futures_lite::future::block_on(futures_lite::future::poll_once(&mut sync_task.0))
        {
            match result {
                Ok(_) => {
                    info!("sync completed for entity {:?}", entity);
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_secs();
                    commands
                        .entity(entity)
                        .remove::<SyncTask>()
                        .insert((
                            SyncComplete(now),
                            crate::systems::protocol::SyncStopTime(std::time::Instant::now()),
                        ));
                }
                Err(e) => {
                    error!("sync failed for entity {:?}: {}", entity, e);
                    commands.entity(entity).remove::<SyncTask>();
                }
            }
        }
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
) -> Result {
    for (entity, remote_host, _) in &query {
        let host_spec = remote_host.0.clone();

        info!(
            "Requesting SSH connection to {} for entity {:?}",
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
) -> Result {
    for (entity, _ssh_ref) in &query {
        let remote_port = 50051; // gRPC port

        info!(
            "Requesting SSH forwarding for entity {:?} to port {}",
            entity, remote_port
        );

        commands
            .entity(entity)
            .insert(ForwardingRequest { remote_port });
    }
    Ok(())
}

async fn simplecopy(source: std::path::PathBuf, dest: std::path::PathBuf) -> Result<(), String> {
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;

    let entries = std::fs::read_dir(&source).map_err(|e| e.to_string())?;

    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();

        if path.is_file() {
            let dest_path = dest.join(entry.file_name());
            std::fs::copy(&path, &dest_path).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}
