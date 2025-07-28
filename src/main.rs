use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use clap::Parser;
use core::time::Duration;
use std::error::Error;
//use std::path::Path;
use uuid::Uuid;

use lib::*;

// TODO cli module
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    exclude: Vec<String>,

    // TODO: This should be default true but for me for now its opt in
    #[arg(long, default_value_t = false)]
    sync: bool,

    source: String,
    dest: String,
}

#[derive(Component)]
struct Wtf;

#[derive(Default, Component)]
struct SyncDir {
    id: Uuid,
    src: String, // This will probably become a Vec at some point, need to brain on that a bit for when there are multiple source/dests and different nodes too for now local sync only and i'll only do one sync as well.
    dst: String,
}

// bevy ecs related

fn main() -> Result<(), Box<dyn Error>> {
    // TODO: integrate env args with yeet::Config at some point
    //       let args: Vec<String> = env::args().collect();

    //       dbg!(cli.sync, cli.exclude, lhs, rhs);

    let cli = Cli::parse();

    let task = SyncDir {
        id: Uuid::new_v4(),
        src: cli.source.clone(),
        dst: cli.dest.clone(),
    };

    // This app loops forever at 10 "fps" TODO so if this is my tick count
    // gotta figure out how to do brute force checking of dir trees for
    // changes. Need to have a "worst case" fallback if inotify/ebpf no
    // worky.
    App::new()
        .add_plugins(
            DefaultPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / 10.0,
            ))),
            //                .disable::<LogPlugin>(),
        )
        .add_systems(Update, counter)
        .add_systems(Startup, move |cmd: Commands| {
            insert_sync(cmd, &task);
        })
        .run();

    Ok(())
}

// This is a bit of a "special" system in that we create a local resource/struct
// and insert that into the bevy ecs world. From there Components get created
// based on that but we always use the initial data for things (the uuid
// specifically) to identify a group of directories that should be in sync
// somehow.
fn insert_sync(mut _cmd: Commands, config: &SyncDir) {
    log::info!(
        "sync id {} lhs {} to rhs {}",
        config.id,
        config.src,
        config.dst,
    );
}

fn counter(mut state: Local<CounterState>) {
    state.count += 1;

    if state.count % 30 == 0 {
        state.stats.update();
        info!("{}", state.stats);
    }
}

#[derive(Default)]
struct CounterState {
    count: u32,
    stats: PidStats,
}

// HERE

// #[derive(Default)]
// struct MyConfig {
//     magic: usize,
// }

// fn my_system(
//     mut cmd: Commands,
//     my_res: Res<MyStuff>,
//     // note this isn't a valid system parameter
//     config: &MyConfig,
// ) {
//     // TODO: do stuff
// }

// fn main() {
//     let config = MyConfig {
//         magic: 420,
//     };

//     App::new()
//         .add_plugins(DefaultPlugins)

//         // create a "move closure", so we can use the `config`
//         // variable that we created above

//         // Note: we specify the regular system parameters we need.
//         // The closure needs to be a valid Bevy system.
//         .add_systems(Update, move |cmd: Commands, res: Res<MyStuff>| {
//             // call our function from inside the closure,
//             // passing in the system params + our custom value
//             my_system(cmd, res, &config);
//         })
//         .run();
// }
