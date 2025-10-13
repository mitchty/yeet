pub mod rpc;
pub mod systems;

use bevy::prelude::*;

// Common components for ecs systems/rpc... TODO: better spot for this crap?
// Whatever... future mitch task sucker past mitch regrets nothing.

// Marker struct for ecs query simplification.
// #[derive(Debug, Default, Component)]
// pub struct SyncRequest;

// Sync sources and destination are abused in most system entities.
#[derive(Debug, Default, Component, Deref)]
pub struct Source(pub std::path::PathBuf);

#[derive(Debug, Default, Component, Deref)]
pub struct Dest(pub std::path::PathBuf);

#[derive(Debug, Default, Component, Deref)]
pub struct Uuid(pub u128);

// Marker component to differentiate OneShot syncs vs not
#[derive(Debug, Default, Component)]
pub struct OneShot;

// TODO: Should this be an enum? I kinda want a marker component for "totes
// done" vs intermediate state
#[derive(Debug, Default, Component)]
pub struct SyncComplete;

#[derive(Debug, Clone, Event, Message)]
pub enum RpcEvent {
    OneshotSync {
        lhs: String,
        rhs: String,
        uuid: u128,
    },
    LogLevel {
        level: crate::rpc::loglevel::Level,
    },
}

use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedReceiver;

#[derive(Resource, Clone)]
pub struct SyncEventReceiver(pub Arc<Mutex<UnboundedReceiver<RpcEvent>>>);

use tokio::sync::mpsc::UnboundedSender;

#[derive(Resource, Clone)]
pub struct SyncEventSender(pub Arc<Mutex<UnboundedSender<RpcEvent>>>);

// This is crap code but whatever its good enough for gov work
//
// For a quick mvp going to implement this sync inside a system directly. Which
// is totally wrong but its just to hook stuff up enough to be minimally useful.
pub fn dirwalk(dir: &std::path::Path, data: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
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

// Instead of passing in a metric butt tonne of stuff, lets yeet it all into a
// config struct.
//
// This is *mostly* the same as the clap struct.
#[derive(Default, Debug)]
pub struct SyncConfig {
    pub excludes: Vec<String>, // TODO should probably be Arc not Vec, unless I want to change excludes at runtime? OLD AF CODE NEEDS TO MOVE INTO ECS
    pub sync: bool,
    pub inotify: bool,
    pub tasks: bool,
    pub lhs: std::path::PathBuf,
    pub rhs: std::path::PathBuf,
}

// Save on boilerplate defining error struct/trait stuff
#[derive(thiserror::Error, Debug)]
pub enum UserError {
    #[error("YEETERR1 syncing {0} to the same dir {1} is invalid")]
    Samedir(String, String),

    #[error("YEETERR2 syncing {0} to the same underlying dir {1} is invalid")]
    CanonicalSamedir(String, String),
}

#[derive(thiserror::Error, Debug)]
pub enum DebugError {
    #[error(
        "YEETERR0 You shouldn't see this error, this is definitely a bug. Open an issue for mitch to fix it and stop being lazy {0}"
    )]
    MitchIsLazy(String),
}

// Make sure we're not being asked to sync to the same directory in both args. I can allow this (though it seems silly) at some point in the future.
fn guard_invalid<U: AsRef<std::path::Path>, V: AsRef<std::path::Path>>(
    src: U,
    dest: V,
) -> Result<(), UserError> {
    // Optimization/dum user check that source and dest aren't the same
    if src.as_ref() == dest.as_ref() {
        return Err(UserError::Samedir(
            format!("{}", src.as_ref().display()),
            format!("{}", dest.as_ref().display()),
        ));
    }

    // Canonicalize paths and test that too
    if let Ok(csrc) = std::fs::canonicalize(&src)
        && let Ok(cdest) = std::fs::canonicalize(&dest)
        && csrc == cdest
    {
        return Err(UserError::CanonicalSamedir(
            format!("{}", csrc.display()),
            format!("{}", cdest.display()),
        ));
    }
    Ok(())
}

// Oneshot sync function, note this isn't async so it can run in bevy.
pub fn sync<U: AsRef<std::path::Path>, V: AsRef<std::path::Path>>(
    from: U,
    to: V,
    config: SyncConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut stack = Vec::new();
    stack.push(std::path::PathBuf::from(from.as_ref()));

    let src = std::path::PathBuf::from(from.as_ref());
    let dest = std::path::PathBuf::from(to.as_ref());

    let output_root = std::path::PathBuf::from(to.as_ref());
    let input_root = std::path::PathBuf::from(from.as_ref()).components().count();

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
        let src: std::path::PathBuf = working_path.components().skip(input_root).collect();
        println!("wtf: src now {:?}", src);

        // Create a destination if missing
        let dest = if src.components().count() == 0 {
            output_root.clone()
        } else {
            output_root.join(&src)
        };

        if std::fs::metadata(&dest).is_err() {
            println!("mkdir: {:?}", dest);
            std::fs::create_dir_all(&dest)?;
        }

        for entry in std::fs::read_dir(working_path)? {
            let entry = entry?;
            let path = entry.path();

            let mut ignore = false;

            if let Some(basename) = path.as_path().file_name() {
                //                println!("dbg: dirname {:?}", basename);

                for s in &config.excludes {
                    if let Some(bs) = basename.to_str()
                        && bs == s
                        && basename.to_str() == Some(s)
                    {
                        ignore = true;
                        continue;
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
                        if let Ok(metadata) = std::fs::symlink_metadata(filename)
                            && metadata.file_type().is_symlink()
                        {
                            println!("todo: symlink ignored {:?}", path.file_name());
                            continue;
                        }

                        let dest_path = dest.join(filename);
                        println!("cp: {:?} -> {:?}", &path, &dest_path);
                        std::fs::copy(&path, &dest_path)?;
                    }
                    None => {
                        println!("error: cp: {:?}", path);
                    }
                }
            }
        }
    }
    Ok(())
}
