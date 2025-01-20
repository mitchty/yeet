use bevy::{app::ScheduleRunnerPlugin, log::LogPlugin, prelude::*};
use clap::Parser;
use core::time::Duration;
use std::error::Error;
use std::path::Path;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    exclude: Vec<String>,

    // TODO: This should be default true but for me for now its opt in
    #[arg(long, default_value_t = false)]
    sync: bool,

    // Opt into inotify stuff as well (for now...)
    #[arg(long, default_value_t = false)]
    inotify: bool,

    // Opt into task queueing as  well (for now...)
    #[arg(long, default_value_t = false)]
    tasks: bool,

    // Default is like rsync, sync src -> dir only
    //
    // Plan is to make it bidirectional, not sure what default should be.
    #[arg(long, default_value_t = false)]
    bidirectional: bool,

    source: String,
    dest: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    if true {
        App::new()
            .add_plugins(DefaultPlugins.set(ScheduleRunnerPlugin::run_once()))
            .add_systems(Update, hello_world_system)
            .run();

        // This app loops forever at 10 "fps" TODO so if this is my tick count
        // gotta figure out how to do brute force checking of dir trees for
        // changes. Need to have a "worst case" fallback if inotify/ebpf no
        // worky.
        App::new()
            .add_plugins(
                DefaultPlugins
                    .set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                        1.0 / 10.0,
                    )))
                    .disable::<LogPlugin>(),
            )
            .add_systems(Update, counter)
            .run();
    } else {
        // TODO: integrate env args with yeet::Config at some point
        //    let args: Vec<String> = env::args().collect();

        let cli = Cli::parse();

        let lhs = Path::new(cli.source.as_str());
        let rhs = Path::new(cli.dest.as_str());

        let conf = yeet::Config {
            excludes: cli.exclude.clone(),
            sync: cli.sync,
            inotify: cli.inotify,
            tasks: cli.tasks,
        };
        yeet::sync(lhs, rhs, conf)?;

        //    dbg!(cli.sync, cli.exclude, lhs, rhs);
    }
    Ok(())
}

fn hello_world_system() {
    println!("hello world");
}

fn counter(mut state: Local<CounterState>) {
    if state.count % 10 == 0 {
        println!("{}", state.count);
    }
    state.count += 1;
}

#[derive(Default)]
struct CounterState {
    count: u32,
}
