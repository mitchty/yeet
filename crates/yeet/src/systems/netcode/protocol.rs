use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub struct SyncChannel;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Message)]
pub struct SyncRequest {
    pub source: String,
    pub dest: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Message)]
pub struct SyncStatus {
    pub uuid: u128,
    pub message: String,
}

// TODO: I'm not 100% sure I should keep the replicated components separate from the non.
//
// But it separates out what a user sees from what might be happening nicely so
// I'll keep it for now or until I find something better.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReplicatedSource(pub PathBuf);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReplicatedDest(pub PathBuf);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReplicatedUuid(pub u128);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReplicatedSimpleCopy;

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReplicatedSyncComplete(pub u64);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReplicatedSyncStartTime {
    pub started_secs: u64,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReplicatedSyncStopTime {
    pub stopped_secs: u64,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReplicatedCompletionTime {
    pub completed_secs: u64,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ReplicatedIoProgress {
    pub dirs_found: u64,
    pub files_found: u64,
    pub total_size: u64,
    pub dirs_written: u64,
    pub files_written: u64,
    pub bytes_written: u64,
    pub completion_percent: f64,
    pub error_count: usize,
    pub skipped_count: u64,
    pub throughput_bps: f64,
}

#[derive(Clone)]
pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        app.register_message::<SyncRequest>()
            .add_direction(NetworkDirection::ClientToServer);

        app.register_message::<SyncStatus>()
            .add_direction(NetworkDirection::ServerToClient);

        app.register_component::<ReplicatedSource>();
        app.register_component::<ReplicatedDest>();
        app.register_component::<ReplicatedUuid>();
        app.register_component::<ReplicatedSimpleCopy>();
        app.register_component::<ReplicatedSyncComplete>();
        app.register_component::<ReplicatedSyncStartTime>();
        app.register_component::<ReplicatedSyncStopTime>();
        app.register_component::<ReplicatedCompletionTime>();
        app.register_component::<ReplicatedIoProgress>();

        app.register_component::<crate::systems::stats::Uptime>();
        app.register_component::<crate::systems::stats::Mem>();
        app.register_component::<crate::systems::stats::Cpu>();

        app.add_channel::<SyncChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);
    }
}
