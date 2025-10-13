use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy_tokio_tasks::{TaskContext, TokioTasksRuntime};

use crate::rpc::{
    loglevel::{MyLogLevel, log_level_server::LogLevelServer},
    yeet::{MyYeet, yeet_server::YeetServer},
};
use crate::{Dest, OneShot, RpcEvent, Source, SyncEventReceiver, SyncEventSender, Uuid};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

// Poll for grpc events on the mpsc channel first.
fn poll_rpc_events(mut event_writer: MessageWriter<RpcEvent>, receiver: Res<SyncEventReceiver>) {
    if let Ok(mut r) = receiver.0.lock() {
        while let Ok(event) = r.try_recv() {
            debug!("found gprc event {:?}", event);
            event_writer.write(event);
        }
    }
}

// This always runs after ^^^ to minimize the ecs seeing components between ticks.
fn handle_rpc_event(
    mut commands: Commands,
    mut events: MessageReader<RpcEvent>,
    log_handle: Option<Res<crate::systems::loglevel::LogHandle>>,
) {
    use crate::rpc::loglevel::Level;

    for event in events.read() {
        match event {
            RpcEvent::OneshotSync { lhs, rhs, uuid } => {
                debug!(
                    "got a one shot sync request lhs {lhs}, rhs {rhs}, uuid {uuid} {}",
                    uuid::Uuid::from_u128(*uuid)
                );
                commands.spawn((
                    Source(std::path::PathBuf::from(lhs)),
                    Dest(std::path::PathBuf::from(rhs)),
                    Uuid(*uuid),
                    OneShot {},
                ));
            }
            RpcEvent::LogLevel { level } => {
                debug!("handling loglevel event: {:?}", level);
                if let Some(ref handle) = log_handle {
                    let set_level = match *level {
                        Level::Trace => "trace",
                        Level::Debug => "debug",
                        Level::Info => "info",
                        Level::Warn => "warn",
                        Level::Error => "error",
                        Level::NoneUnspecified => "info",
                    };
                    let _ = handle.set_max_level(String::from(set_level));
                }
            }
        }
    }
}

pub struct GrpcDaemon;

// There are 2 different server "types", or will be at some point
//
// On unix systems use a unix domain socket for IPC between client/daemon, we'll
// presume that if a system we're running on is compromised its game over anyway
// so whatever.
//
// We will also setup a localhost port for from/to for each system's rpc between
// systems, 2 per maybe? And one will be for control plane rpc, the other for
// sending sync data rpc (or not at all if that turns out to not work well
// enough).
impl Plugin for GrpcDaemon {
    fn build(&self, app: &mut App) {
        use tap::prelude::*;
        // We don't add this ourselves, the caller is responsible for
        // configuring the tokio tasks plugin, if not its a fatal error.
        assert!(app.is_plugin_added::<bevy_tokio_tasks::TokioTasksPlugin>());

        // Add our systems, note unix uses a unix domain socket for all client
        // cli <-> daemon rpc.
        //
        // Each is its own tokio task runtime, the cli is kinda "control-plan"
        // type rpc and the socket will eventually be private inter yeet rpc.
        //
        // Windows will likely be a unix domain socket as well if/when I get to it.
        //
        // note I'm not dealing with security implications of the socket or uds for now.
        //
        // Also for now the socket and uds servers are the same for dev simplicity
        // I'll revisit this later.
        //
        // Also need to brain through if i have a local socket for cli as well
        // for when/if there is user x trying to talk to user y's daemon.
        //
        // I kinda want to not handle that case at all tbh and let the socket
        // perms determine that stuff. Maybe just allow a way for clients to
        // specify the uds file to talk to.

        let (tx, rx): (UnboundedSender<RpcEvent>, UnboundedReceiver<RpcEvent>) =
            unbounded_channel();
        let tx = Arc::new(Mutex::new(tx)); // shareable by gRPC threads
        let rx = Arc::new(Mutex::new(rx)); // Bevy reads from this

        app.add_message::<RpcEvent>()
            .insert_resource(SyncEventReceiver(rx.clone()))
            .insert_resource(SyncEventSender(tx.clone()))
            .add_systems(Startup, (startup, start_tcp.after(startup)))
            .tap_mut(|a| {
                // Uds support only will exist in unix systems.
                #[cfg(unix)]
                a.add_systems(Startup, start_uds.after(startup));
            })
            .add_systems(
                Update,
                (poll_rpc_events, handle_rpc_event.after(poll_rpc_events)),
            );
    }
}

// TODO: wrap me in a cfg unix directive once/if windows is figured out
//
// At that point figure out the "right" thing to abuse in lieu of UDS for local
// process IPC.
fn startup(mut commands: Commands) {
    let service = LogLevelService {};
    commands.insert_resource(service.clone());
}

// bevy wrapper system that spawns the async tokio grpc task for listening
#[cfg(unix)]
fn start_uds(runtime: ResMut<'_, TokioTasksRuntime>, event_sender: Res<crate::SyncEventSender>) {
    let sender = event_sender.0.clone();
    runtime.spawn_background_task(move |ctx| run_uds(ctx, sender));
}

fn start_tcp(runtime: ResMut<'_, TokioTasksRuntime>, event_sender: Res<crate::SyncEventSender>) {
    let sender = event_sender.0.clone();
    runtime.spawn_background_task(move |ctx| run_tcp(ctx, sender));
}

// Note the tcp socket might come in more handy for stuff running non locally
// like maybe a web gooey. That... is a future mitch problem though. I couldn't
// build a gooey worth a shit to save my life.

//
// Or even configure to allow multiple listen addresses?
//
// I like domain sockets as then I could skip a bit of logic around local
// authentication, if the local system is compromised its game off anyway.
pub mod proto {
    tonic::include_proto!("loglevel");
    tonic::include_proto!("yeet");

    pub(crate) const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("reflection");
}

#[derive(Default, Clone, Resource)]
pub struct LogLevelService {}

// For future mitch or anyone more able to do windows than me, until
// https://github.com/rust-lang/rust/issues/56533 is resolved maybe this will
// work for a unix domain socket solution on windows for client/daemon
// communication there? https://docs.rs/uds_windows/latest/uds_windows/
//
// Looks like it might be the best solution perf wise on windows
// https://www.yanxurui.cc/posts/server/2023-11-28-benchmark-tcp-uds-namedpipe/
async fn run_tcp(_ctx: TaskContext, event_sender: Arc<Mutex<UnboundedSender<RpcEvent>>>) {
    use tonic::transport::Server;

    let addr = "[::]:50051".parse().expect("this shouldn't fail ever...");
    let event_sender = event_sender.clone();

    let loglevel = MyLogLevel::new(event_sender.clone());
    let yeet = MyYeet::new(event_sender.clone());
    let reflection = match tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1()
    {
        Ok(r) => r,
        Err(e) => {
            error!("tcp reflection registration failed");
            eprintln!("{:?}", eyre::eyre!(e));
            std::process::exit(2);
        }
    };

    // TODO make uds/tcp not WET this? I don't think I need this too often copy/pasta is ok for now.
    let tcp_server = Server::builder()
        .add_service(LogLevelServer::new(loglevel))
        .add_service(YeetServer::new(yeet))
        .add_service(reflection)
        .serve(addr);

    // Bail if we can't bind to the socket
    if let Err(e) = tcp_server.await {
        error!("fatal: Couldn't spawn tcp server, is yeet already running?");
        eprintln!("{:?}", eyre::eyre!(e));
        std::process::exit(2);
    }

    info!("tcp server started");
}

#[cfg(unix)]
async fn run_uds(_ctx: TaskContext, event_sender: Arc<Mutex<UnboundedSender<RpcEvent>>>) {
    use std::os::unix::fs::FileTypeExt;
    use tokio::net::UnixListener;
    use tokio_stream::wrappers::UnixListenerStream;
    use tonic::transport::Server;

    extern crate directories;
    use directories::ProjectDirs;

    let proj_cache = ProjectDirs::from("net", "mitchty", "yeet")
        .expect("fatal: could not determine project cache dir");

    let path = proj_cache.cache_dir().join("local.uds");

    let event_sender = event_sender.clone();

    use std::fs;

    // At some point this won't be in /tmp so mkdir on all parents will be in order
    let parent = path.parent();

    // Ensure our socket file exists, and is actually a unix domain socket and
    // not a file/dir/fifo whatever the hell might be there instead.
    if let Ok(metadata) = fs::metadata(&path) {
        if !metadata.file_type().is_socket() {
            error!(
                "{} exists but is not a Unix Domain Socket file, cannot run until removed",
                path.display()
            );
            std::process::exit(2);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    } else if let Some(p) = parent
        && let Ok(_parentage) = std::fs::create_dir_all(p)
    {
        debug!(
            "created parentage {} Unix Domain Socket {}",
            p.display(),
            path.display()
        );
    } else {
        error!("fatal: couldn't create parent path for {}", path.display());
        std::process::exit(2);
    };

    let uds = UnixListener::bind(&path).unwrap_or_else(|e| {
        error!("fatal: failed to bind unix socket {}", &path.display());
        eprintln!("{:?}", eyre::eyre!(e));
        std::process::exit(2);
    });

    let addr_uds = UnixListenerStream::new(uds);

    let loglevel = MyLogLevel::new(event_sender.clone());
    let yeet = MyYeet::new(event_sender.clone());

    let reflection = match tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1()
    {
        Ok(r) => r,
        Err(e) => {
            error!("uds reflection registration failed");
            eprintln!("{:?}", eyre::eyre!(e));
            std::process::exit(2);
        }
    };

    let uds_server = Server::builder()
        .add_service(LogLevelServer::new(loglevel))
        .add_service(YeetServer::new(yeet))
        .add_service(reflection)
        .serve_with_incoming(addr_uds);

    uds_server.await.unwrap_or_else(|e| {
        error!("fatal: couldn't spawn uds server, is yeet already running?");
        eprintln!("{:?}", eyre::eyre!(e));
        std::process::exit(2);
    });

    info!("uds server started");
}
