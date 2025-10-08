use bevy::prelude::*;

pub struct Sys;

impl Plugin for Sys {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, move |cmd: Commands| startup(cmd));
    }
}

// System for Dumping out system/build info
// Also dump out more info about the os we're run upon.
fn startup(mut _cmd: Commands) -> Result {
    if cfg!(debug_assertions) {
        info!("debug build");
    } else {
        debug!("release build");
    }
    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();
    if let Some(kernel) = sysinfo::System::kernel_version()
        && let Some(version) = sysinfo::System::os_version()
        && let Some(name) = sysinfo::System::name()
        && let Some(hostname) = sysinfo::System::host_name()
    {
        info!(
            "cores: {} kernel: {} version: {} name: {} hostname: {}",
            sys.cpus().len(),
            kernel,
            version,
            name,
            hostname
        );
    }
    info!(
        "memory: {}/{}",
        humansize::format_size(sys.used_memory(), humansize::BINARY),
        humansize::format_size(sys.total_memory(), humansize::BINARY),
    );

    // Don't dump out swap column if there is no swap to dump
    if sys.total_swap() > 0 {
        info!(
            "swap: {}/{}",
            humansize::format_size(sys.used_swap(), humansize::BINARY),
            humansize::format_size(sys.total_swap(), humansize::BINARY),
        );
    } else {
        debug!("swap: none apparently");
    }

    Ok(())
}
