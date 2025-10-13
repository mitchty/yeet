use bevy::prelude::*;
use bevy_cronjob::prelude::*;

use crate::{Dest, OneShot, Source, SyncComplete, Uuid};

pub struct Syncer;

// We'll start out abusing the builtin bevy IoTaskPool, bevy tokio tasks is ok
// but I was only wanting to use it for a grpc<->bevy bridge where it makes
// sense.
//
// But it might make sense to use that for bridging events from
// epoll()/ionotify/ebpf in future too.
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
        // We don't add this ourselves, the caller is responsible for
        // configuring the the cronjob plugin as its shared across Plugins
        assert!(app.is_plugin_added::<bevy_cronjob::CronJobPlugin>());

        app.add_systems(Update, update.run_if(schedule_passed("every second")));
        app.add_systems(
            Update,
            (
                spawn_sync_tasks,
                check_sync_completion.after(spawn_sync_tasks),
            ),
        );

        // Dump out I'm idle bra every 60 seconds or so
        app.add_systems(
            Update,
            update_idle.run_if(schedule_passed("every 1 minute")),
        );
        // app.add_systems(Startup, move |cmd: Commands| startup(cmd))
        //     .add_systems(Update, update);
    }
}

// Stupid idle system when there is nothing to sync.
fn update_idle(_: Query<(), (Without<Source>, Without<Dest>)>) -> Result {
    debug!("idle");

    Ok(())
}

// Dump out anything that is actively syncing (or just was and isn't updated in
// this tick) when run.
//
// Note, for now we'll only deal with things with a OneShot marker
fn update(
    mut _commands: Commands,
    query: Query<(Entity, &Source, &Dest, &Uuid, &OneShot), Without<SyncComplete>>,
) -> Result {
    for (_entity, lhs, rhs, uuid, _os) in &query {
        info!(
            "oneshot sync {}->{} uuid {}",
            lhs.display(),
            rhs.display(),
            uuid::Uuid::from_u128(uuid.0)
        );

        //        commands.entity(entity).insert(SyncComplete);
    }
    Ok(())
}

// Go away clippy its not a problem, I know its complex its just a lot of crap and how bevy works.
#[allow(clippy::type_complexity)]
fn spawn_sync_tasks(
    mut commands: Commands,
    query: Query<(Entity, &Source, &Dest, &OneShot), (Without<SyncTask>, Without<SyncComplete>)>,
) -> Result {
    let task_pool = bevy::tasks::IoTaskPool::get();

    for (entity, source, dest, _ignored) in &query {
        let source = source.0.clone();
        let dest = dest.0.clone();

        // Oneshot is a glorified cp for now.
        info!(
            "spawning oneshot sync task {} -> {}",
            source.display(),
            dest.display()
        );

        let task = task_pool.spawn(async move { oneshot(source, dest).await });

        commands.entity(entity).insert(SyncTask(task));
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
                    commands
                        .entity(entity)
                        .remove::<SyncTask>()
                        .insert(SyncComplete);
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

async fn oneshot(source: std::path::PathBuf, dest: std::path::PathBuf) -> Result<(), String> {
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
