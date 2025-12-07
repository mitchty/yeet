use bevy::prelude::*;

pub struct Build;

impl Plugin for Build {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, move |cmd: Commands| startup(cmd));
    }
}

// System for Dumping out build system info
fn startup(mut _cmd: Commands) -> Result {
    info!(
        "type: {} version: {} commit: {} rustc: {}",
        env!("BUILD_CARGO_PROFILE"),
        env!("CARGO_PKG_VERSION"),
        env!("STUPIDNIXFLAKEHACK"),
        env!("RUSTC_VERSION")
    );
    Ok(())
}
