use core::time::Duration;
use std::error::Error;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;

use bevy_cronjob::prelude::*;
use clap::{Parser, Subcommand};

use lib::{Dest, Source};

// Arc and Mutex are for process local shared state
//
// This is mostly confined to Config level things that don't change often at
// runtime so the impact is minimal but *must* be shared amongst all threads.
//
// That is, say we're syncing /a -> /b, this would be one entry in a mutable vec
// of work items.
//
// The bevy ecs systems take care of tracking things from that point. Config
// updates also update a simple integer to cover generation tracking of updates to the vec.
//
// That is the primary mutex across things. Only a very select few bevy systems
// are impacted by this mutex and all of those cause internal event triggers to
// ensure things are eventually consistent after a change.
//
//
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: SubCommands,
}

#[derive(Subcommand)]
enum SubCommands {
    /// Synchronize files between source and destination
    Sync {
        /// Exclude paths
        #[arg(short, long)]
        exclude: Vec<String>,

        /// This should be default true, but for now it's opt-in
        #[arg(long, default_value_t = false)]
        sync: bool,

        /// Source directory
        source: String,

        /// Destination directory
        dest: String,
    },

    /// Start in daemon mode
    Serve {
        /// Be verbose or not, doesn't do jack atm
        #[arg(short, long)]
        verbose: bool,
    },
}

// OK need to brain a skosh on how I'll handle syncing across systems in a
// stateless ish way.
//
// Thought is this SyncDir can be local and between nodes everyone agrees a
// local SyncDir uuid =='s a global uuid that encompasses "all these dirs
// amongst nodes is the same sync"
//
// For now I'll just tackle simple syncing locally from /a -> /b one way
//
// Then /a <-> /b both ways.
//
// And start braining up how I unit test edge cases of things like conflicts and
// define them so I can be assured this thing doesn't lose data ever but bubbles
// up a sync failure to the user so they can decide what to do.
// bevy ecs related

fn main() -> Result<(), Box<dyn Error>> {
    // TODO: integrate env args with yeet::Config at some point
    //       let args: Vec<String> = env::args().collect();

    //       dbg!(cli.sync, cli.exclude, lhs, rhs);

    let cli = Cli::parse();

    // This bevy app loops forever at 10 "fps" as its internal event loop
    // conceptually. TODO: is this the right tick frequency or should I make
    // it configurable somehow? gotta figure out how to do brute force checking
    // of dir trees for changes. Need to have a "worst case" fallback if
    // inotify/kqueue/ebpf/etc.... no worky.
    //
    // TODO: hacky idea, have a background system that checks mtime of dirs
    // maybe every second or so as a background task and if updates are found
    // triggers a dirty signal for that path forcing the next tick to validate things?
    //
    // Whatever future task implement the stupidest idea first for now future me
    // can fix it in post.. sucker.
    let mut appbinding = App::new();

    let app =
        appbinding
            .add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
                Duration::from_secs_f64(1.0 / 60.0),
            )))
            .add_plugins(bevy::log::LogPlugin::default())
            .add_plugins(CronJobPlugin)
            .add_plugins(lib::systems::stats::Stats);

    match cli.command {
        SubCommands::Sync {
            exclude,
            sync,
            source,
            dest,
        } => {
            debug!("Syncing from {} to {}", source.clone(), dest.clone());
            debug!("Exclude: {:?}", exclude.clone());
            debug!("Sync flag: {}", sync.clone());

            app.add_systems(Startup, move |cmd: Commands| {
                setup_sync(cmd, &source.as_str(), &dest.as_str())
            });
        }
        SubCommands::Serve { verbose: _verbose } => {
            info!("serving");

            app.add_plugins((
                lib::systems::sys::Sys,
                lib::systems::syncer::Syncer,
                bevy_tokio_tasks::TokioTasksPlugin::default(),
                lib::systems::grpcdaemon::GrpcDaemon,
            ));
        }
    }

    app.run();

    Ok(())
}

// TODO: this needs to be done through rpc calls, though for oneshot syncs I can
// skip that bit?
fn setup_sync(mut cmd: Commands, source: &str, dest: &str) -> Result {
    cmd.spawn((
        Source(std::path::PathBuf::from(source)),
        Dest(std::path::PathBuf::from(dest)),
    ));
    Ok(())
}
