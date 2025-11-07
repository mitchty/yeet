use core::time::Duration;
use std::error::Error;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;

use clap::{Parser, Subcommand};

const VERSTR: &str = const_format::formatcp!(
    "{} {} {}",
    env!("CARGO_PKG_VERSION"),
    env!("STUPIDNIXFLAKEHACK"),
    env!("BUILD_CARGO_PROFILE")
);

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
#[derive(Parser)]
#[command(version, about, long_about = None, long_version = VERSTR)]
struct Cli {
    #[command(subcommand)]
    command: SubCommands,
}

#[derive(Subcommand)]
enum SubCommands {
    // TODO: Should make grpc a cargo feature if/when I find out if using bevy
    // directly is a better idea or not.
    /// Grpc related stuff
    Grpc {
        /// Exclude paths
        #[arg(short, long, default_value_t = false)]
        socketonly: bool,
    },

    /// Start in daemon mode
    Serve {
        /// Be verbose or not, doesn't do jack atm
        #[arg(short, long)]
        verbose: bool,
    },

    /// Monitor daemon state and sync progress
    Monitor {
        /// Remote host to monitor (defaults to localhost)
        #[arg(long)]
        host: Option<String>,

        /// Spawn test entities for demonstration
        #[arg(long)]
        test: bool,
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

use log::Level;

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    // Handle monitor command - separate lightweight bevy app
    // if let SubCommands::Monitor { host, test } = &cli.command {
    //     let host = host.clone().unwrap_or_else(|| "localhost".to_string());
    //     return run_monitor(&host, *test);
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

    match cli.command {
        SubCommands::Grpc { socketonly } => {
            if socketonly && let Ok(p) = lib::get_uds_file() {
                println!("{}", p.display());
            };
        }
        SubCommands::Serve { verbose: _verbose } => {
            let app = appbinding.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
                Duration::from_secs_f64(1.0 / 20.0),
            )));

            app.add_plugins((
                lib::systems::loglevel::LogLevelPlugin {
                    level: Level::Trace,
                },
                bevy_tokio_tasks::TokioTasksPlugin {
                    make_runtime: Box::new(|| {
                        let mut runtime = tokio::runtime::Builder::new_current_thread();
                        runtime.enable_all();
                        runtime.build().expect("tokio runtime did not build")
                    }),
                    ..bevy_tokio_tasks::TokioTasksPlugin::default()
                },
                lib::systems::tty::StdinPlugin,
                lib::systems::stats::Stats,
                lib::systems::sys::Sys,
                lib::systems::build::Build,
                lib::systems::ssh::Pool,
                lib::systems::ssh::Manager,
                lib::systems::syncer::Syncer,
                lib::systems::grpc::GrpcPlugin,
                lib::systems::netcode::server::LightYearServerPlugin,
            ));
            app.add_systems(Update, toggle_logging_level_debug);
        }
        SubCommands::Monitor { test: _, .. } => {
            setup_ctrlc_handler();

            let app = appbinding.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
                Duration::from_secs_f64(1.0 / 5.0),
            )));
            app.add_plugins((
                bevy::input::InputPlugin,
                lib::systems::netcode::client::LightYearClientPlugin,
                lib::systems::monitor::MonitorPlugin,
            ));
        }
    }

    appbinding.run();

    Ok(())
}

use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;

// TODO: This is a bit of a hack, should maybe make this a cargo feature.
fn toggle_logging_level_debug(
    log_handle: Res<lib::systems::loglevel::LogHandle>,
    key: Res<ButtonInput<KeyCode>>,
    _modifiers: Res<ButtonInput<KeyModifiers>>,
) {
    let mut change = false;
    let mut level = "info"; // This is the default

    if key.just_pressed(KeyCode::Char('d')) || key.just_pressed(KeyCode::Char('D')) {
        change = true;
        level = "debug";
    }

    if key.just_pressed(KeyCode::Char('t')) || key.just_pressed(KeyCode::Char('T')) {
        change = true;
        level = "trace";
    }

    if key.just_pressed(KeyCode::Char('w')) || key.just_pressed(KeyCode::Char('W')) {
        change = true;
        level = "warn";
    }

    if key.just_pressed(KeyCode::Char('e')) || key.just_pressed(KeyCode::Char('E')) {
        change = true;
        level = "error";
    }

    // This comes last so if multiple keys got pressed this "wins" by being last checked
    if key.just_pressed(KeyCode::Char('i')) || key.just_pressed(KeyCode::Char('I')) {
        change = true;
        level = "info";
    }

    if change {
        eprintln!("log level set to {}\r", level);
        log_handle
            .set_max_level(level.to_string())
            .expect("something wack broke bra");
    }
}

fn setup_ctrlc_handler() {
    ctrlc::set_handler(move || {
        if let Err(e) = crossterm::terminal::disable_raw_mode() {
            eprintln!("erro: failed to disable raw mode: {}\r", e);
        }

        if let Err(e) = crossterm::execute!(std::io::stdout(), crossterm::cursor::Show) {
            eprintln!("error: failed to show cursor: {}\r", e);
        }

        std::process::exit(0);
    })
    .expect("some kinda error setting Ctrl+C handler");
}

// TODO: Keep? I might reuse this later it was a failed experiment
#[allow(dead_code)]
fn setup_monitor_logging()
-> tracing_indicatif::writer::IndicatifWriter<tracing_indicatif::writer::Stderr> {
    use std::io::{self, Write};
    use tracing_subscriber::prelude::*;

    // Enable raw mode for crossterm keyboard input
    crossterm::terminal::enable_raw_mode().expect("couldn't switch tty to raw mode from cooked");

    // Set up panic hook to restore terminal on panic
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        _ = crossterm::terminal::disable_raw_mode();
        println!();
        default_panic(info);
    }));

    // Wrapper to add \r before \n for raw mode
    struct CRLFStderr(io::Stderr);
    impl Write for CRLFStderr {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut out = Vec::new();
            for &b in buf {
                if b == b'\n' {
                    out.push(b'\r');
                }
                out.push(b);
            }
            self.0.write_all(&out)?;
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }

    let indicatif_layer = tracing_indicatif::IndicatifLayer::new();

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(indicatif_layer.get_stderr_writer())
        .with_writer(|| CRLFStderr(io::stderr()));

    let filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive(tracing::Level::INFO.into());

    // Get the writer before moving the layer into init
    let indicatif_writer = indicatif_layer.get_stderr_writer();

    tracing_subscriber::registry()
        .with(filter)
        //        .with(tracing_subscriber::fmt::layer().with_writer(indicatif_layer.get_stderr_writer()))
        .with(indicatif_layer)
        .with(fmt_layer)
        .init();

    // Return the writer so the monitor can use it for coordinated output
    indicatif_writer
}
