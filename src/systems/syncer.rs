use bevy::prelude::*;
use bevy_cronjob::prelude::*;

use crate::{Dest, Source};

pub struct Syncer;

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

        app.add_systems(Update, update.run_if(schedule_passed("every 11 seconds")));

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
fn update(mut query: Query<(&Source, &Dest)>) -> Result {
    for (lhs, rhs) in &mut query {
        info!("syncing source {} dest {}", lhs.display(), rhs.display());
    }
    Ok(())
}

// Enum to encompass the idea of a "oneshot" (aka once) sync from lhs -> rhs,
// Or a constant sync from lhs -> rhs
// Or a constant sync from/to lhs <-> rhs
#[derive(Default, Debug, Component)]
pub enum Method {
    OneWay,
    TwoWay,
    #[default]
    OneShot,
}
