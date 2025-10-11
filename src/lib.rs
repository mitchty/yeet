pub mod rpc;
pub mod systems;

use bevy::prelude::*;
use std::path::PathBuf;

// Common components for ecs systems/rpc... TODO: better spot for this crap?
// Whatever... future mitch task sucker past mitch regrets nothing.

// Marker struct for ecs query simplification.
// #[derive(Debug, Default, Component)]
// pub struct SyncRequest;

// Sync sources and destination are abused in most system entities.
#[derive(Debug, Default, Component, Deref)]
pub struct Source(pub PathBuf);

#[derive(Debug, Default, Component, Deref)]
pub struct Dest(pub PathBuf);

#[derive(Debug, Clone, Event, Message)]
pub enum RpcEvent {
    SpawnSync {
        name: String,
    },
    LogLevel {
        level: crate::rpc::loglevel::loglevel::Level,
    },
}

use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedReceiver;

#[derive(Resource, Clone)]
pub struct SyncEventReceiver(pub Arc<Mutex<UnboundedReceiver<RpcEvent>>>);

use tokio::sync::mpsc::UnboundedSender;

#[derive(Resource, Clone)]
pub struct SyncEventSender(pub Arc<Mutex<UnboundedSender<RpcEvent>>>);
