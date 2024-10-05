use clap::{Arg, Command, Parser};
use notify::*;
use std::env::{self, SplitPaths};
use std::error::Error;
use std::{fs, path::Path, path::PathBuf, time::Duration};

// yeet things about
use yeet::sync;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    exclude: Vec<String>,

    #[arg(long, default_value_t = false)]
    sync: bool,

    source: String,
    dest: String,
}

fn main() -> Result<()> {
    //fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();

    let cli = Cli::parse();

    let lhs = Path::new(cli.source.as_str());
    let rhs = Path::new(cli.dest.as_str());

    if cli.sync {
        let conf = yeet::Config {
            excludes: cli.exclude.clone(),
        };
        yeet::sync(lhs, rhs, conf)?;
    }

    dbg!(cli.sync, cli.exclude, lhs, rhs);

    Ok(())

    // if let Some(extra_args) = matches.get_many::<String>("dirs") {
    //     let args: Vec<&String> = extra_args.collect();
    //     println!("Trailing arguments: {} and {}", args[0], args[1]);

    // } else {
    //     todo!("buggy bug bug fixit");
    // }

    // if let Some(source) = args.get(1) {
    //     if let Some(dest) = args.get(2) {

    //         return Ok(());
    //     }
    // }

    // mkdir && copy files as needed, note for this local test this is silly

    // Watching crap
    // let (tx, rx) = std::sync::mpsc::channel();
    // // This example is a little bit misleading as you can just create one Config and use it for all watchers.
    // // That way the pollwatcher specific stuff is still configured, if it should be used.
    // let mut watcher: Box<dyn Watcher> = if RecommendedWatcher::kind() == WatcherKind::PollWatcher {
    //     // custom config for PollWatcher kind
    //     // you
    //     let config = Config::default().with_poll_interval(Duration::from_secs(1));
    //     Box::new(PollWatcher::new(tx, config).unwrap())
    // } else {
    //     // use default config for everything else
    //     Box::new(RecommendedWatcher::new(tx, Config::default()).unwrap())
    // };

    // // watch some stuff
    // watcher
    //     .watch(Path::new("."), RecursiveMode::Recursive)
    //     .unwrap();

    // // just print all events, this blocks forever
    // for e in rx {
    //     println!("{:?}", e);
    // }
}
