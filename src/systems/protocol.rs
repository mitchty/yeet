use bevy::prelude::*;

#[derive(Component, Debug, Clone)]
pub struct SyncStartTime(pub std::time::Instant);

#[derive(Component, Debug, Clone)]
pub struct SyncStopTime(pub std::time::Instant);

#[derive(Component, Debug, Clone)]
pub struct CompletionTime(pub std::time::Instant);
