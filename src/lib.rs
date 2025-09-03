pub mod rpc;
pub mod systems;

use bevy::prelude::*;
use std::path::PathBuf;

// Common components for the ecs, better spot for this crap? Whatever.

// Sync sources and destination are abused in most system entities.
#[derive(Debug, Default, Component, Deref)]
pub struct Source(pub PathBuf);

#[derive(Debug, Default, Component, Deref)]
pub struct Dest(pub PathBuf);
