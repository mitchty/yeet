use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub struct Stats;

impl Plugin for Stats {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, move |cmd: Commands| startup(cmd))
            .add_systems(
                Update,
                update.run_if(bevy::time::common_conditions::on_timer(
                    std::time::Duration::from_secs(1),
                )),
            );
    }
}

// Setup the Stats entity for the overall process
//
// Note, at startup we refresh_all and hopefully by the time any update() is
// called the data isn't red level sus.
//
fn startup(mut commands: Commands) -> Result {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();

    // Docs say to have at least two refreshes before trying to use any data.
    // For now abuse sleep to wait twice and refresh everything at initial
    // startup.
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_all();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);

    if sysinfo::IS_SUPPORTED_SYSTEM {
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            false,
            sysinfo::ProcessRefreshKind::nothing().with_cpu(),
        );
        sys.refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram());

        sys.refresh_processes(
            sysinfo::ProcessesToUpdate::Some(&[sysinfo::get_current_pid()?]),
            true,
        );

        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, false);

        // This is before the main entity spawn as that takes ownership of the
        // System struct.
        let mem: Option<Mem> = if let Some(process) = sys.process(sysinfo::get_current_pid()?) {
            Some(Mem(process.memory()))
        } else {
            warn!("could not get process memory stats?");
            None
        };

        let cpu: Option<Cpu> = if let Some(process) = sys.process(sysinfo::get_current_pid()?) {
            Some(Cpu(process.cpu_usage()))
        } else {
            warn!("could not get process memory stats?");
            None
        };

        let stat_entity = commands
            .spawn((
                PidStats,
                Start(std::time::Instant::now()),
                Uptime(0),
                System(sys),
            ))
            .id();

        if let Some(m) = mem {
            commands.entity(stat_entity).insert(m);
        }

        if let Some(c) = cpu {
            commands.entity(stat_entity).insert(c);
        }
    }

    Ok(())
}

// Marker component to control if the sysinfo crates returning sus data.
//
// Don't run things like update if thats the case, no need to waste cpu if the data is sus.
#[derive(Debug, Default, Component)]
struct Sus;

// Process stats so I can see how bad of an idea yeeting a ton of data in ecs
// Tables for inodes is/n't.
//
// TODO: make all of this configurable at runtime via the ecs and via grpc calls
// via cli for the background daemon.
fn update(
    mut _commands: Commands,
    mut stats: Query<
        (Entity, &Start, &mut Uptime, &mut Mem, &mut Cpu, &mut System),
        (With<System>, Without<Sus>),
    >,
) -> Result {
    match stats.single_mut() {
        Err(_) => {}
        Ok((_entity, start, mut uptime, mut mem, mut cpu, mut system)) => {
            // Do less work for stat updates
            // system.0.refresh_all();
            // system.0.refresh_cpu_usage();

            system.0.refresh_processes(
                sysinfo::ProcessesToUpdate::Some(&[sysinfo::get_current_pid()?]),
                true,
            );

            // Note, swap isn't included here, lets not care about anything swapped out by the os.
            system
                .0
                .refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram());

            if let Some(process) = system.0.process(sysinfo::get_current_pid()?) {
                *mem = Mem(process.memory());
                *cpu = Cpu(process.cpu_usage());
                *uptime = Uptime(std::time::Instant::now().duration_since(**start).as_secs());
            }
        }
    }
    Ok(())
}

// Can turn stat tracking off/on at runtime in systems? Not sure this makes
// sense as a marker. I think I'll need the cpu/memory to build some of the
// control theory later.
#[derive(Default, Debug, Component)]
pub struct PidStats;

// Process Stats, though I suppose I could record multiple Processes data across
// systems at some point and sync them every second or so.
#[derive(Debug, Component, Deref)] // no Default for this obviously
pub struct Start(pub std::time::Instant); // Not super precise just whever we get the Instant into the world

// Replicated stats components - these get sent to clients
#[derive(Default, Debug, Component, Deref, Serialize, Deserialize, Clone, PartialEq)]
pub struct Uptime(pub u64); // Uptime in seconds - replicated version of elapsed time since Start, probably needs a new name.

#[derive(Default, Debug, Component, Deref, Serialize, Deserialize, Clone, PartialEq)]
pub struct Mem(pub u64);

#[derive(Default, Debug, Component, Deref, Serialize, Deserialize, Clone, PartialEq)]
pub struct Cpu(pub f32);

#[derive(Default, Debug, Component, Deref)]
pub struct System(pub sysinfo::System);

#[cfg(test)]
mod tests {
    use super::*;

    // Because I am getting nothing back lets double check I know how to use this dam crate at all...
    #[test]
    fn test_sysinfo_can_get_current_process_memory() {
        // Create a new System instance and refresh memory
        let mut sys = sysinfo::System::new_all();
        sys.refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram());

        let pid_p = sysinfo::get_current_pid();
        assert!(pid_p.is_ok(), "sysinfo can't get the current pid");

        let pid = pid_p.unwrap();

        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), false);

        let process_p = sys.process(pid);
        assert!(
            process_p.is_some(),
            "sysinfo crate can't get the pid process?"
        );

        let process = process_p.unwrap();

        let memory_kb = process.memory() / 1024;

        // Memory should be non-zero for a running process
        assert!(
            memory_kb > 0,
            "process memory should be greater than 0, got {} KB",
            memory_kb
        );

        // Dunno what I might set this to its more in case memory would roll
        // over from 0 to say MAX_SIZE or something stupid.
        assert!(
            memory_kb < 10 * 1024 * 1024,
            "process memory shouldn't be over 10GiB...: {} KB",
            memory_kb
        );
    }

    #[test]
    fn test_sysinfo_can_get_current_process_cpu() {
        let mut sys = sysinfo::System::new_all();

        let pid_p = sysinfo::get_current_pid();
        assert!(pid_p.is_ok(), "sysinfo can't get the current pid");

        let pid = pid_p.unwrap();

        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), false);

        // Refresh process info - need to do this twice for CPU usage
        // First refresh establishes baseline
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), false);

        std::thread::sleep(std::time::Duration::from_millis(100));

        // Second refresh calculates CPU usage allegedly...
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), false);

        let process_p = sys.process(pid);
        assert!(
            process_p.is_some(),
            "sysinfo crate can't get the pid process?"
        );

        let process = process_p.unwrap();

        let cpu_usage = process.cpu_usage();

        // 0% cpu usage is bs and likely a bug somewhere.
        assert!(
            cpu_usage >= 0.0,
            "CPU usage should be >= 0.0, got {}",
            cpu_usage
        );

        // For the test its unlikely we'll use more than 100% of a cpu core... I
        // think... dunno lets see if it trips ever. yolo
        assert!(
            cpu_usage < 10000.0,
            "CPU usage seems unreasonably high: {}",
            cpu_usage
        );
    }

    #[test]
    fn test_sysinfo_process_finds_current_pid() {
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, false);

        let pid_p = sysinfo::get_current_pid();
        assert!(pid_p.is_ok(), "sysinfo can't get the current pid");

        let pid = pid_p.unwrap();

        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), false);

        if let Some(p) = sys.process(pid) {
            assert!(p.memory() > 0);
        }
        assert!(sysinfo::IS_SUPPORTED_SYSTEM);

        // If we got here, we successfully found the process
        assert!(sys.process(pid).is_some());
    }

    #[test]
    fn test_mem_component_serialization() {
        // 1GiB
        let mem = Mem(1024 * 1024);

        let cloned = mem.clone();
        assert_eq!(*mem, *cloned);
        assert_eq!(mem, cloned);
    }

    #[test]
    fn test_cpu_component_serialization() {
        let cpu = Cpu(25.5);

        let cloned = cpu.clone();
        assert_eq!(*cpu, *cloned);
        assert_eq!(cpu, cloned);
    }

    #[test]
    fn test_uptime_component_serialization() {
        let uptime = Uptime(3600); // 1 hour in seconds

        let cloned = uptime.clone();
        assert_eq!(*uptime, *cloned);
        assert_eq!(uptime, cloned);
    }
}
