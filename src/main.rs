use notify::*;
use std::env::{self, SplitPaths};
use std::error::Error;
use std::fmt;
// use std::io;
use std::{fs, path::Path, path::PathBuf, time::Duration};

// yeet things about
use yeet::sync;

#[derive(Debug)]
struct ArgError {
    details: String,
}

impl ArgError {
    fn new(msg: &str) -> ArgError {
        ArgError {
            details: msg.to_string(),
        }
    }
}

impl fmt::Display for ArgError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.details)
    }
}

impl Error for ArgError {}

// Specify if we are a source process or sink (for now we are one way a->b only not a<->b yet)
#[derive(PartialEq)]
enum Mode {
    Source,
    Sink,
    Bug,
}

//fn main() -> std::io::Result<()> {
fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    // let mut mode = Mode::Bug;

    // if let Some(wat) = args.get(1) {
    //     if wat == "source" {
    //         mode = Mode::Source;
    //     } else if wat == "sink" {
    //         mode = Mode::Sink;
    //     } else {
    //         mode = Mode::Bug;
    //         panic!("invalid mode {} only source or sink are valid bra", wat);
    //     }
    // } else {
    //     panic!("need an arg bra");
    // }

    if let Some(source) = args.get(1) {
        if let Some(dest) = args.get(2) {
            let lhs = Path::new(source);
            let rhs = Path::new(dest);

            yeet::sync(lhs, rhs)?;
            return Ok(());
        }
    }

    return Ok(());

    // if mode == Mode::Source {
    //     let source = Path::new(".");
    //     let mut source_crap = Vec::new();

    //     dirwalk(&source, &mut source_crap)?;

    //     for file in source_crap {
    //         println!("{}", file.display());
    //     }
    //     return Ok(());
    // }
    // if mode == Mode::Sink {
    //     // Ensure its clean each run, ignore if its missing already
    //     return match std::fs::remove_dir_all(dest) {
    //         Ok(_) => Ok(()),
    //         Err(e) => match e.kind() {
    //             io::ErrorKind::NotFound => {
    //                 println!("dest already missing not removing");
    //                 Ok(())
    //             }
    //             //            io::ErrorKind::PermissionDenied => println!("Error: Permission denied."),
    //             _ => Err(e.into()),
    //         },
    //     };
    // }

    return Ok(());

    // mkdir && copy files as needed, note for this local test this is silly

    // Watching crap
    let (tx, rx) = std::sync::mpsc::channel();
    // This example is a little bit misleading as you can just create one Config and use it for all watchers.
    // That way the pollwatcher specific stuff is still configured, if it should be used.
    let mut watcher: Box<dyn Watcher> = if RecommendedWatcher::kind() == WatcherKind::PollWatcher {
        // custom config for PollWatcher kind
        // you
        let config = Config::default().with_poll_interval(Duration::from_secs(1));
        Box::new(PollWatcher::new(tx, config).unwrap())
    } else {
        // use default config for everything else
        Box::new(RecommendedWatcher::new(tx, Config::default()).unwrap())
    };

    // watch some stuff
    watcher
        .watch(Path::new("."), RecursiveMode::Recursive)
        .unwrap();

    // just print all events, this blocks forever
    for e in rx {
        println!("{:?}", e);
    }
    Ok(())
}
