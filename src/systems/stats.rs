use bevy::prelude::*;
pub struct Stats;

impl Plugin for Stats {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, move |cmd: Commands| startup(cmd))
            .add_systems(Update, update);
    }
}

fn startup(mut cmd: Commands) -> Result {
    cmd.spawn((
        Logger(bevy::time::Stopwatch::new()),
        LoggerElapsed(3.0),
        Updater(bevy::time::Stopwatch::new()),
        UpdaterElapsed(1.0),
        PidStats,
        Start(std::time::Instant::now()),
        Mem(0),
        Cpu(0.0),
        System(sysinfo::System::new_all()),
    ));
    Ok(())
}

// Process stats so I can see how bad of an idea yeeting a ton of data in ecs
// Tables for inodes is/n't.
//
// TODO: make all of this configurable at runtime via the ecs and via grpc calls
// via cli.
fn update(
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

#[derive(Default, Component)]
pub struct Updater(pub bevy::time::Stopwatch);

#[derive(Default, Component)]
pub struct UpdaterElapsed(pub f32);

#[derive(Default, Component)]
pub struct Logger(pub bevy::time::Stopwatch);

#[derive(Default, Component)]
pub struct LoggerElapsed(pub f32);

// Can turn stat tracking off/on at runtime in systems.
#[derive(Debug, Component)]
pub struct PidStats;

// Process Stats, though I suppose I could record multiple Processes data across
// systems at some point and sync them every second or so.
#[derive(Debug, Component)]
pub struct Start(pub std::time::Instant); // Not super precise just whever we get the Instant into the world
#[derive(Debug, Component)]
pub struct Mem(pub u64);
#[derive(Debug, Component)]
pub struct Cpu(pub f32);
#[derive(Debug, Component)]
pub struct System(pub sysinfo::System);
