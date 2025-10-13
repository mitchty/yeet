use bevy::prelude::*;
use core::time::Duration;
use std::sync::Once;

use bevy_cronjob::prelude::*;

pub struct Stats;

impl Plugin for Stats {
    fn build(&self, app: &mut App) {
        // We don't add this ourselves, the caller is responsible for
        // configuring the the cronjob plugin as its shared across Plugins
        assert!(app.is_plugin_added::<bevy_cronjob::CronJobPlugin>());

        app.add_systems(Startup, move |cmd: Commands| startup(cmd))
            .add_systems(Update, update.run_if(schedule_passed("every 7 seconds")));
    }
}

// Setup the Stats entity for the overall process
//
// Note, at startup we refresh_all and hopefully by the time any update() is
// called the data isn't red level sus.
//
fn startup(mut cmd: Commands) -> Result {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();

    cmd.spawn((
        PidStats,
        Start(std::time::Instant::now()),
        Mem(0),
        Cpu(0.0),
        System(sys),
    ));

    Ok(())
}

// Process stats so I can see how bad of an idea yeeting a ton of data in ecs
// Tables for inodes is/n't.
//
// TODO: make all of this configurable at runtime via the ecs and via grpc calls
// via cli for the background daemon.
fn update(mut stats: Query<(&Start, &mut Mem, &mut Cpu, &mut System)>) -> Result {
    let (start, mut mem, mut cpu, mut system) = stats.single_mut()?;

    // Do less work for stat updates
    //    system.0.refresh_all();
    //    system.0.refresh_cpu_usage();

    // Note, swap isn't included here, lets not care about anything swapped out by the os.
    system
        .0
        .refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram());

    // TODO how is sysinfo returning bad data on macos all the sudden? Future winter debug task.
    let mut sus = false;

    if let Ok(p) = sysinfo::get_current_pid() {
        system
            .0
            .refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
    } else {
        sus = true;
        static WARN_PID_ONCE: Once = Once::new();
        WARN_PID_ONCE.call_once(|| {
            warn!("might be bug in sysinfo crate? could not determine current pid, stats are sus");
        });
    }

    if let Some(process) = system.0.process(sysinfo::Pid::from_u32(std::process::id())) {
        *mem = Mem(process.memory() / 1024);
        *cpu = Cpu(process.cpu_usage());
    } else {
        sus = true;
        static WARN_PROCESS_ONCE: Once = Once::new();
        WARN_PROCESS_ONCE.call_once(|| {
            warn!(
                "might be bug in sysinfo crate? could not get process details for cpu and memory pid data, all data returned will be sus"
            );
        });
    }

    if sus {
        debug!(
            "up: {} mem/cpu sus",
            humantime::format_duration(Duration::from_secs(
                std::time::Instant::now().duration_since(**start).as_secs()
            ))
        );
    } else {
        debug!(
            "up: {} mem: {} cpu: {:.1}%",
            humantime::format_duration(Duration::from_secs(
                std::time::Instant::now().duration_since(**start).as_secs()
            )),
            humansize::format_size(**mem, humansize::BINARY),
            **cpu
        );
    }
    Ok(())
}

// Can turn stat tracking off/on at runtime in systems.
#[derive(Default, Debug, Component)]
pub struct PidStats;

// Process Stats, though I suppose I could record multiple Processes data across
// systems at some point and sync them every second or so.
#[derive(Debug, Component, Deref)] // no Default for this obviously
pub struct Start(pub std::time::Instant); // Not super precise just whever we get the Instant into the world

#[derive(Default, Debug, Component, Deref)]
pub struct Mem(pub u64);

#[derive(Default, Debug, Component, Deref)]
pub struct Cpu(pub f32);

#[derive(Default, Debug, Component, Deref)]
pub struct System(pub sysinfo::System);
