use core::time::Duration;
use std::error::Error;
use std::path::PathBuf;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy_tokio_tasks::{TaskContext, TokioTasksRuntime};
use clap::{Parser, Subcommand};
use tonic::transport::Server;

use yeet::{MyGreeter, yeet::greeter_server::GreeterServer};
//use std::path::Path;
//use uuid::Uuid;

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

    // match cli.command {
    //     SubCommands::Sync {
    //         ref exclude,
    //         ref sync,
    //         ref source,
    //         ref dest,
    //     } => {
    //         debug!("Syncing from {} to {}", source.clone(), dest.clone());
    //         debug!("Exclude: {:?}", exclude.clone());
    //         debug!("Sync flag: {}", sync.clone());
    //     }
    //     SubCommands::Serve { ref verbose } => {
    //         debug!("serving, verbose: {}", verbose);
    //     }
    // }
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

    let app = appbinding
        .add_plugins(
            DefaultPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / 60.0,
            ))),
            //                .disable::<LogPlugin>(),
        )
        .add_plugins(bevy_tokio_tasks::TokioTasksPlugin::default())
        .add_systems(Startup, move |cmd: Commands| setup_stats(cmd));

    match cli.command {
        SubCommands::Sync {
            exclude: _,
            sync: _,
            source,
            dest,
        } => {
            app.add_systems(Startup, move |cmd: Commands| {
                setup_sync(cmd, &source.as_str(), &dest.as_str())
            })
            .add_systems(Update, log_periodic);
        }
        SubCommands::Serve { verbose: _ } => {
            app.add_systems(Startup, server_daemon);
        }
    }

    app.add_systems(Update, (update_stats, log_periodic_stats))
        .run();

    Ok(())
}

fn server_daemon(runtime: ResMut<TokioTasksRuntime>) {
    runtime.spawn_background_task(runserver);
}

// No mut ctx just yet...
async fn runserver(_ctx: TaskContext) {
    let addr = "[::1]:50051".parse().expect("meh");
    let greeter = MyGreeter::default();

    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve(addr)
        .await
        .expect("wtf");
}

fn setup_stats(mut cmd: Commands) -> Result {
    cmd.spawn((
        Logger(bevy::time::Stopwatch::new()),
        LoggerElapsed(3.0),
        Updater(bevy::time::Stopwatch::new()),
        UpdaterElapsed(1.0),
        Stats,
        Start(std::time::Instant::now()),
        Mem(0),
        Cpu(0.0),
        System(sysinfo::System::new_all()),
    ));
    Ok(())
}

fn setup_sync(mut cmd: Commands, source: &str, dest: &str) -> Result {
    cmd.spawn((
        Logger(bevy::time::Stopwatch::new()),
        LoggerElapsed(30.0),
        Source(std::path::PathBuf::from(source)),
        Dest(std::path::PathBuf::from(dest)),
    ));
    Ok(())
}

#[derive(Default, Component)]
struct Logger(bevy::time::Stopwatch);

#[derive(Default, Component)]
struct Updater(bevy::time::Stopwatch);

#[derive(Default, Component)]
struct LoggerElapsed(f32);

#[derive(Default, Component)]
struct UpdaterElapsed(f32);

fn log_periodic(
    time: Res<Time>,
    mut l: Query<(&mut Logger, &LoggerElapsed, &Source, &Dest)>,
) -> Result {
    let (mut logger, elapsed, source, dest) = l.single_mut()?;

    if logger.0.elapsed_secs() > elapsed.0 {
        logger.0.reset();
        info!("source {} dest {}", source.0.display(), dest.0.display());
    } else {
        logger.0.tick(time.delta());
    }
    Ok(())
}

fn log_periodic_stats(
    time: Res<Time>,
    mut stats: Query<(&mut Logger, &LoggerElapsed, &Start, &Mem, &Cpu)>,
) -> Result {
    let (mut logger, elapsed, start, mem, cpu) = stats.single_mut()?;

    if logger.0.elapsed_secs() > elapsed.0 {
        logger.0.reset();
        info!(
            "up: {} mem: {} cpu: {:.1}%",
            humantime::format_duration(Duration::from_secs(
                std::time::Instant::now().duration_since(start.0).as_secs()
            )),
            humansize::format_size(mem.0, humansize::BINARY),
            cpu.0
        )
    } else {
        logger.0.tick(time.delta());
    }
    Ok(())
}

// Process stats so I can see how bad of an idea yeeting a ton of data in ecs
// Tables for inodes is/n't.
fn update_stats(
    time: Res<Time>,
    mut stats: Query<(
        &mut Updater,
        &UpdaterElapsed,
        &mut Mem,
        &mut Cpu,
        &mut System,
    )>,
) -> Result {
    let (mut updater, elapsed, mut mem, mut cpu, mut system) = stats.single_mut()?;

    if updater.0.elapsed_secs() > elapsed.0 {
        updater.0.reset();
        system.0.refresh_all();
        if let Some(process) = system.0.process(sysinfo::Pid::from_u32(std::process::id())) {
            *mem = Mem(process.memory() / 1024);
            *cpu = Cpu(process.cpu_usage());
        }
    } else {
        updater.0.tick(time.delta());
    }
    Ok(())
}

#[derive(Debug, Component)]
struct Source(pub PathBuf);
#[derive(Debug, Component)]
struct Dest(pub PathBuf);

// Can turn stat tracking off/on at runtime in systems.
#[derive(Debug, Component)]
struct Stats;

// Process Stats, though I suppose I could record multiple Processes data across
// systems at some point and sync them every second or so.
#[derive(Debug, Component)]
struct Start(std::time::Instant); // Not super precise just whever we get the Instant into the world
#[derive(Debug, Component)]
struct Mem(u64);
#[derive(Debug, Component)]
struct Cpu(f32);
#[derive(Debug, Component)]
struct System(sysinfo::System);
