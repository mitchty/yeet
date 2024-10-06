use notify::*;

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

impl Ord for Task {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.priority.cmp(&self.priority)
    }
}

// Instead of passing in a metric butt tonne of stuff, lets yeet it all into a
// config struct.
//
// This is *mostly* the same as the clap struct.
#[derive(Debug)]
pub struct Config {
    pub excludes: Vec<String>,
}

impl AsRef<Config> for Config {
    fn as_ref(&self) -> &Config {
        &self
    }
}

// Will be the main entry point for all syncs
// For now only additive/updates only not handling removals yet
pub fn sync<U: AsRef<Path>, V: AsRef<Path>>(from: U, to: V, config: Config) -> Result<()> {
    let mut stack = Vec::new();
    stack.push(PathBuf::from(from.as_ref()));

    let src = PathBuf::from(from.as_ref());
    let dest = PathBuf::from(to.as_ref());

    let output_root = PathBuf::from(to.as_ref());
    let input_root = PathBuf::from(from.as_ref()).components().count();

    // Optimization/dum user check that source and dest aren't the same
    if src.as_path() == dest.as_path() {
        println!(
            "note: syncing to the same directory makes no logical sense, you tried to sync: {:?} -> {:?}",
            src, dest
        );
        return Ok(());
    } else {
        // Canonicalize paths and test that too
        if let Ok(csrc) = fs::canonicalize(&src) {
            if let Ok(cdest) = fs::canonicalize(&dest) {
                if csrc == cdest {
                    println!(
            "note: syncing to the same directory makes no logical sense, you tried to sync: {:?} -> {:?}",
			csrc, cdest);
                    return Ok(());
                }
            }
        } else {
            println!("todo: better error checking lazy jerk");
            return Ok(());
        }

        println!("yeet: {:?} -> {:?}", src, dest);
    }

    while let Some(working_path) = stack.pop() {
        println!("process: {:?}", &working_path);

        // Generate a relative path
        let src: PathBuf = working_path.components().skip(input_root).collect();

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
                println!("dbg: dirname {:?}", basename);

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

    // Once we've synced the upstream start watching for changes via inotify

    // Watching crap
    let (tx, rx) = std::sync::mpsc::channel();
    // This example is a little bit misleading as you can just create one Config and use it for all watchers.
    // That way the pollwatcher specific stuff is still configured, if it should be used.
    let mut watcher: Box<dyn Watcher> = if RecommendedWatcher::kind() == WatcherKind::PollWatcher {
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

    let mut tasks = BinaryHeap::new();

    tasks.push(Task {
        priority: 3,
        description: "Low priority task".to_string(),
    });
    tasks.push(Task {
        priority: 1,
        description: "High priority task".to_string(),
    });

    while let Some(Task {
        priority,
        description,
    }) = tasks.pop()
    {
        println!("Executing task: {}, Priority: {}", description, priority);
    }

    Ok(())
}
