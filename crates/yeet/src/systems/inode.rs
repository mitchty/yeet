use bevy::prelude::*;

pub struct Inode;

impl Plugin for Inode {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, move |cmd: Commands| startup(cmd))
            .add_systems(Update, update);
    }
}

fn startup(mut cmd: Commands) -> Result {
    cmd.spawn(());
    Ok(())
}

fn update(//    time: Res<Time>,
    // mut stats: Query<(
    //     &mut Updater,
    //     &UpdaterElapsed,
    //     &mut Mem,
    //     &mut Cpu,
    //     &mut System,
    // )>,
) -> Result {
    // let (mut updater, elapsed, mut mem, mut cpu, mut system) = stats.single_mut()?;

    // if updater.0.elapsed_secs() > elapsed.0 {
    //     updater.0.reset();
    //     system.0.refresh_all();
    //     if let Some(process) = system.0.process(sysinfo::Pid::from_u32(std::process::id())) {
    //         *mem = Mem(process.memory() / 1024);
    //         *cpu = Cpu(process.cpu_usage());
    //     }
    // } else {
    //     updater.0.tick(time.delta());
    // }
    Ok(())
}

// WIP
//
// Note: Whilst I *could* just reuse std::fs::{FileType,MetaData} and newtype
// those, I want to (ab)use change notification in bevy's ecs when things change
// as that will let me setup systems that *just* for example act as chown u+w if
// that is all that changed.
//
// So the newtyping here is a bit egregious but its not without rationale.
//
// I can simulate the overall tick logic in unit tests pretty easily this way.
//
// As well as determine how I want to handle weird race conditions between ecs
// world ticks.

// Relative path of the fsent from sync root
#[derive(Debug, Component)]
pub struct RelPath(pub std::path::PathBuf);

#[derive(Default, Debug, Component)]
pub enum FileTypes {
    Dir,
    File,
    RelativeLink,
    FullLink,
    HardLink,
    #[default]
    Invalid, // Catch all, anything that isn't a symlink/file/dir is out of scope for yeet
}

// Newtype Wrapper around ^^^ enum
#[derive(Default, Debug, Component)]
pub struct FileType(pub FileTypes);

// File size in bytes
#[derive(Default, Debug, Component)]
pub struct FileSizeBytes(pub u64);

// creation time/(ctime), note since local SystemTime per system, e.g. windows
// doesn't have a guaranteed resolution like ns (100ns on windows apparently)
// need to approach this slightly differently
//use std::time::SystemTime;

// match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
//     Ok(n) => println!("1970-01-01 00:00:00 UTC was {} seconds ago!", n.as_secs()),
//     Err(_) => panic!("SystemTime before UNIX EPOCH!"),
// }

// TODO: Windows? How?/When?/Whom? I don't know shit about windows filesystems.
// Passthrough for std::fs::Permissions
