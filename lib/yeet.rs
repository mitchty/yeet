use notify::{PollWatcher, RecommendedWatcher, RecursiveMode, Watcher, WatcherKind};

//use std::error::Error;
use std::collections::BinaryHeap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

// TODO DFS vs BFS traversal as an option

// TODO how do I want to separate this into a work queue instead of a list or other such dum idea.
pub fn dirwalk(dir: &Path, data: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                data.push(path.to_path_buf());
                dirwalk(&path, data)?;
            } else if path.is_file() {
                data.push(path.to_path_buf());
            }
        }
    }
    Ok(())
}

struct Task {
    priority: i32,
    description: String,
}

impl PartialEq for Task {
    fn eq(&self, other: &Task) -> bool {
        self.priority == other.priority
    }
}

impl Eq for Task {}

impl PartialOrd for Task {
    fn partial_cmp(&self, other: &Task) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// Note instead of 0 being highest we kinda implement this as Reverse and for
// the i32 priority 0 is lowest, and go up from there on priority
//
// This lets the overall logic for this entire wack app basically amount to
// knowing when and where to throw something onto the priority queue.
//
// Aka if while doing initial sync, we get hit by an inotify/bpf update to a file we've not seen/transferred we need to be able to:
// - mkdir -p on the parent dirs and ensure they're the right mode/etc... as highest priority (all this should be ONE rpc between systems)
// - then sync the file entirely.. (TODO should these cases preempt user policies when implemented?)
// - ????
//
// Anyway future mitch hopefully past mitch isn't a dork in this regard.
impl Ord for Task {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        //        other.priority.cmp(&self.priority)
        self.priority.cmp(&other.priority)
    }
}

// Instead of passing in a metric butt tonne of stuff, lets yeet it all into a
// config struct.
//
// This is *mostly* the same as the clap struct.
#[derive(Debug)]
pub struct Config {
    pub excludes: Vec<String>,
    pub sync: bool,
    pub inotify: bool,
    pub tasks: bool,
}

impl AsRef<Config> for Config {
    fn as_ref(&self) -> &Config {
        &self
    }
}

// Save on boilerplate defining error struct/trait bs
#[derive(thiserror::Error, Debug)]
pub enum UserError {
    #[error("YERROR01 syncing {0} to the same dir {1} is invalid")]
    Samedir(String, String),

    #[error("YERROR02 syncing {0} to the same underlying dir {1} is invalid")]
    CanonicalSamedir(String, String),
}

#[derive(thiserror::Error, Debug)]
pub enum DebugError {
    #[error("YERROR00 You shouldn't see this {0}")]
    MitchIsLazy(String),
}

// Make sure we're not being asked to sync to the same directory in both args. I can allow this (though it seems silly) at some point in the future.
fn guard_invalid<U: AsRef<Path>, V: AsRef<Path>>(src: U, dest: V) -> Result<(), UserError> {
    // Optimization/dum user check that source and dest aren't the same
    if src.as_ref() == dest.as_ref() {
        return Err(UserError::Samedir(
            format!("{}", src.as_ref().display()),
            format!("{}", dest.as_ref().display()),
        ));
    }

    // Canonicalize paths and test that too
    if let Ok(csrc) = fs::canonicalize(&src) {
        if let Ok(cdest) = fs::canonicalize(&dest) {
            if csrc == cdest {
                return Err(UserError::CanonicalSamedir(
                    format!("{}", csrc.display()),
                    format!("{}", cdest.display()),
                ));
            }
        }
    }
    Ok(())
}

// Will be the main entry point for all syncs
// For now only additive/updates only not handling removals yet
//
// TODO how do I handle if the source or dest goes away during/after a sync? Aka
// what if someone bind mounts over src... while walking the dentries?
//
// Future mitch problem
pub fn sync<U: AsRef<Path>, V: AsRef<Path>>(
    from: U,
    to: V,
    config: Config,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut stack = Vec::new();
    stack.push(PathBuf::from(from.as_ref()));
    let mut tasks = BinaryHeap::new();

    let src = PathBuf::from(from.as_ref());
    let dest = PathBuf::from(to.as_ref());

    let output_root = PathBuf::from(to.as_ref());
    let input_root = PathBuf::from(from.as_ref()).components().count();

    guard_invalid(from, to)?;

    if config.sync {
        println!("yeet: {:?} -> {:?}", src, dest);
    } else {
        return Ok(());
    }

    // TODO hard coded to start transferring files from the get-go via BFS
    // traversal (aka iff we find a dir we throw that onto the priority queue
    // lower than files for now.
    //
    // TODO How do I want to think about policies here? Think I should just let
    // those be additive to the priority and dequeue the highest priority.
    // with a lower priority than a file)

    // TODO Async rust this to be MPSC to submit to a workqueue thread so that
    // notify events can start immediately and take priority over bulk transfers.
    while let Some(working_path) = stack.pop() {
        println!("sync: {:?}", &working_path);

        // Generate a relative path
        let src: PathBuf = working_path.components().skip(input_root).collect();
        println!("wtf: src now {:?}", src);

        // Create a destination if missing
        let dest = if src.components().count() == 0 {
            output_root.clone()
        } else {
            output_root.join(&src)
        };

        if fs::metadata(&dest).is_err() {
            println!("mkdir: {:?}", dest);
            fs::create_dir_all(&dest)?;
        }

        for entry in fs::read_dir(working_path)? {
            let entry = entry?;
            let path = entry.path();

            let mut ignore = false;

            if let Some(basename) = path.as_path().file_name() {
                //                println!("dbg: dirname {:?}", basename);

                for s in &config.excludes {
                    if let Some(bs) = basename.to_str() {
                        if bs == s {
                            if basename.to_str() == Some(s) {
                                ignore = true;
                                continue;
                            }
                        }
                    }
                }
            }

            if ignore {
                continue;
            }

            // Only operate on dirs and files, anything else, fifo/etc... is not worth pondering
            if path.is_dir() {
                stack.push(path);
            } else {
                match path.file_name() {
                    Some(filename) => {
                        if let Ok(metadata) = fs::symlink_metadata(filename) {
                            if metadata.file_type().is_symlink() {
                                println!("todo: symlink ignored {:?}", path.file_name());
                                continue;
                            }
                        }

                        let dest_path = dest.join(filename);
                        println!("cp: {:?} -> {:?}", &path, &dest_path);
                        fs::copy(&path, &dest_path)?;
                    }
                    None => {
                        println!("error: cp: {:?}", path);
                    }
                }
            }
        }
    }

    if config.tasks {
        tasks.push(Task {
            priority: 10,
            description: "Highest (atm) priority task".to_string(),
        });

        tasks.push(Task {
            priority: 3,
            description: "Higher priority task".to_string(),
        });

        tasks.push(Task {
            priority: 3,
            description: "Another Higher priority task".to_string(),
        });

        tasks.push(Task {
            priority: 1,
            description: "Low priority task".to_string(),
        });

        tasks.push(Task {
            priority: 0,
            description: "Lowest priority task".to_string(),
        });

        while let Some(Task {
            priority,
            description,
        }) = tasks.pop()
        {
            println!("Executing task: {}, Priority: {}", description, priority);
        }
    } else {
        println!("no task stuff for yooooo");
    }

    // Once we've synced the upstream start watching for changes via inotify

    if config.inotify {
        // Watching crap
        let (tx, rx) = std::sync::mpsc::channel();
        // This example is a little bit misleading as you can just create one Config and use it for all watchers.
        // That way the pollwatcher specific stuff is still configured, if it should be used.
        let mut watcher: Box<dyn Watcher> =
            if RecommendedWatcher::kind() == WatcherKind::PollWatcher {
                // custom config for PollWatcher kind
                // you
                let config = notify::Config::default().with_poll_interval(Duration::from_secs(1));
                Box::new(PollWatcher::new(tx, config).unwrap())
            } else {
                // use default config for everything else
                Box::new(RecommendedWatcher::new(tx, notify::Config::default()).unwrap())
            };

        // watch some stuff
        watcher.watch(src.as_path(), RecursiveMode::Recursive)?;

        // this is the "real" main loop, we record all actions we see and in a
        // coalesce period do whatever needs to happen
        for e in rx {
            println!("{:?}", e);
        }
    } else {
        println!("not looping in inotify");
    }

    Ok(())
}
