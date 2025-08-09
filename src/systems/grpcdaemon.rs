use core::time::Duration;
use std::error::Error;
use std::path::PathBuf;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy_tokio_tasks::{TaskContext, TokioTasksRuntime};
use clap::{Parser, Subcommand};
use tonic::transport::Server;

use crate::yeet::{yeet::greeter_server::GreeterServer, MyGreeter};

pub struct GrpcDaemon;

impl Plugin for GrpcDaemon {
    fn build(&self, app: &mut App) {
        // We don't add this ourselves, the caller is responsible for
        // configuring the tokio tasks plugin, if not its a fatal error.
        assert!(app.is_plugin_added::<bevy_tokio_tasks::TokioTasksPlugin>());

        app.add_systems(Startup, startup);
    }
}

// bevy wrapper system that spawns the async tokio grpc task
fn startup(runtime: ResMut<TokioTasksRuntime>) {
    runtime.spawn_background_task(runserver);
}

// No mut ctx just yet...
// TODO: Instead of a tcp socket, maybe unix domain socket instead for unix
// os's?
//
// Or even configure to allow multiple listen addresses?
//
// I like domain sockets as then I could skip a bit of logic around local authentication
//
// But... then on windows wtf do I do, is there even a uds equivalent? I'll
// tackle this if/when I need to get windows working.
async fn runserver(_ctx: TaskContext) {
    let addr = "[::1]:50051".parse().expect("meh");
    let greeter = MyGreeter::default();

    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve(addr)
        .await
        .expect("wtf"); //TODO: figure out a sane thread safe Result/? option with 0.16 bevy
}
