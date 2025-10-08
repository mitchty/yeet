use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy_tokio_tasks::{TaskContext, TokioTasksRuntime};

use crate::rpc::{
    greeter::{MyGreeter, greeter::greeter_server::GreeterServer},
    loglevel::{MyLogLevel, loglevel::log_level_server::LogLevelServer},
};
use crate::{Dest, RpcEvent, Source, SyncEventReceiver, SyncEventSender};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

// Now takes/handles RpcEvent for both types!
fn poll_rpc_events(mut event_writer: EventWriter<RpcEvent>, receiver: Res<SyncEventReceiver>) {
    let mut r = receiver.0.lock().unwrap(); // TODO unwrap sus...
    while let Ok(event) = r.try_recv() {
        debug!("found gprc event {:?}", event);
        event_writer.write(event);
    }
}

fn handle_rpc_event(
    mut commands: Commands,
    mut events: EventReader<RpcEvent>,
        log_handle: Option<Res<crate::systems::loglevel::LogHandle>>,
) {
    for event in events.read() {
        match event {
            RpcEvent::SpawnSync { name } => {
                debug!("spawning bevy event: {:?}", event);
                commands.spawn((
                    Source(std::path::PathBuf::from(name)),
                    Dest(std::path::PathBuf::from(name)), // MVP: same path for dest
                ));
            }
            RpcEvent::LogLevel { level } => {
                debug!("handling loglevel event: {:?}", level);
                if let Some(ref handle) = log_handle {
                    let set_level = match *level {
                        crate::rpc::loglevel::loglevel::Level::Trace => {
                            "trace"
                        },
                        crate::rpc::loglevel::loglevel::Level::Debug => {
                            "debug"
                        },
                        crate::rpc::loglevel::loglevel::Level::Info => {
                            "info"
                        },
                        crate::rpc::loglevel::loglevel::Level::Warn => {
                            "warn"
                        },
                        crate::rpc::loglevel::loglevel::Level::Error => {
                            "error"
                        },
                        crate::rpc::loglevel::loglevel::Level::Off => {
                            "info"
                        },
                    };
                    let _ = handle.set_max_level(String::from(set_level));
                }
            }
        }
    }
}

pub struct GrpcDaemon;

// TODO: get the mpsc bridge between the server working
// Bevy event we're passing into the main Bevy thread via mpsc for spawning a
// sync from userland.
// #[derive(Debug, Default)]
// struct SpawnSyncEvent {
//     lhs: String,
//     rhs: String,
// }

// #[derive(Debug, Default)]
// struct GrpcDaemonImpl {
//     sender: tokio::sync::mpsc::UnboundedSender<SpawnSyncEvent>,
// }

// #[tonic::async_trait]
// impl SyncerService for GrpcDaemonImpl {
//     async fn do_something(
//         &self,
//         request: Request<SyncerService>,
//     ) -> Result<Response<MyResponse>, Status> {
//         let name = request.into_inner().name;
//         // Send into Bevy ECS via channel
//         let _ = self.sender.send(SpawnEntityEvent { name });
//         Ok(Response::new(MyResponse { ok: true }))
//     }
// }

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

        app.add_event::<RpcEvent>()
            .insert_resource(SyncEventReceiver(rx.clone()))
            .insert_resource(SyncEventSender(tx.clone()))
            .add_systems(
                Startup,
                //            (startup, start_uds, start_tcp, hello_event).chain(),
                (startup, start_tcp, hello_event).chain(),
            )
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

//use crate::{Dest, Source};
use std::sync::atomic::{AtomicUsize, Ordering};

fn hello_event(runtime: ResMut<'_, TokioTasksRuntime>) {
    runtime.spawn_background_task(|mut ctx| async move {
        debug!("Loop start");
        let idx = AtomicUsize::new(1);

        loop {
            let cidx = idx.fetch_add(1, Ordering::SeqCst);

            ctx.run_on_main_thread(move |_mainctx| {
                // mainctx.world.commands().spawn((
                //     Source(std::path::PathBuf::from(format!("/lhs{}", cidx))),
                //     Dest(std::path::PathBuf::from(format!("/rhs{}", cidx))),
                // ));
                //                debug!("main thread tick {}", cidx);
            })
            .await;
            tokio::time::sleep(std::time::Duration::from_secs(7)).await;
        }
    });
}

// bevy wrapper system that spawns the async tokio grpc task for listening
// fn start_uds(runtime: ResMut<'_, TokioTasksRuntime>, mut commands: Commands) {
//     let service = LogLevelService {
//         state: Arc::new(Mutex::new(proto::LogState::default())),
//         reset: Arc::new(Mutex::new(false)),
//     };
//     commands.insert_resource(service.clone());

//     runtime.spawn_background_task(run_uds);
// }

fn start_tcp(runtime: ResMut<'_, TokioTasksRuntime>, event_sender: Res<crate::SyncEventSender>) {
    let sender = event_sender.0.clone();
    runtime.spawn_background_task(move |ctx| run_tcp(ctx, sender));
    //    runtime.spawn_background_task(|ctx| async move { info!("a"); run_tcp(ctx, sender); info!("b"); });
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
    tonic::include_proto!("greeter");
    tonic::include_proto!("loglevel");

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
//async fn runserver(mut tasks: bevy_tokio_tasks::TokioTasksRuntime, mut ev: EventWriter<SpawnEntityEvent>) {
//async fn runserver(mut tasks: bevy_tokio_tasks::TokioTasksRuntime) {
// async fn run_tcp<'SyncEventSender>(_ctx: TaskContext, event_sender: Res<'_,
//                                                                         SyncEventSender>) {
async fn run_tcp(_ctx: TaskContext, event_sender: Arc<Mutex<UnboundedSender<RpcEvent>>>) {
    //    async fn run_tcp(_ctx: TaskContext,event_sender: Res<'_, Arc<UnboundedSender<SpawnSyncEvent>>>) {
    use tonic::transport::Server;

    let addr = "[::]:50051".parse().unwrap();
    let event_sender = event_sender.clone();

    let greetertcp = MyGreeter::new(event_sender.clone());
    let logleveltcp = MyLogLevel::new(event_sender.clone());
    let reflectiontcp = match tonic_reflection::server::Builder::configure()
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
        .add_service(GreeterServer::new(greetertcp))
        .add_service(LogLevelServer::new(logleveltcp))
        .add_service(reflectiontcp)
        .serve(addr);

    // Bail if we can't bind to the socket
    if let Err(e) = tcp_server.await {
        error!("fatal: Couldn't spawn tcp server, is yeet already running?");
        eprintln!("{:?}", eyre::eyre!(e));
        std::process::exit(2);
    }

    info!("tcp server started");
    //    tasks.spawn_background_task(|mut ctx| async move {
    // let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    // if let Err(e) = tokio::try_join!(uds_server, tcp_server) {
    //     error!("fatal: Couldn't join uds and tcp server together {:?}", e);
    //     std::process::exit(2);
    // }
    // while let Some(event) = rx.recv().await {
    //     ctx.run_on_main_thread(move |world| {
    //         use bevy::ecs::world::CommandQueue;
    //         let mut command_queue = CommandQueue::default();

    //         // We could manually add our commands to the queue
    //         command_queue.push(MyCommand);

    //         // We can apply the commands to a given world:
    //         command_queue.apply(world);
    //         //                world.send_event(event);
    //     })
    //     .await;
    // }
    //    });
}

//use eyre::{Result, WrapErr, eyre};
//use eyre::eyre;

// async fn run_uds(mut _ctx: TaskContext) {
//     use std::os::unix::fs::FileTypeExt;
//     use tokio::net::UnixListener;
//     use tokio_stream::wrappers::UnixListenerStream;
//     use tonic::transport::Server;

//     // Sue me, working first ~/.cache/yeet later.
//     let path = std::path::Path::new("/tmp/yeet.uds");

//     use std::fs;

//     // At some point this won't be in /tmp so mkdir on all parents will be in order
//     let parent = path.parent();

//     // Ensure our socket file exists, and is actually a unix domain socket and
//     // not a file/dir/fifo whatever the hell might be there instead.
//     if let Ok(metadata) = fs::metadata(path) {
//         if !metadata.file_type().is_socket() {
//             error!(
//                 "{} exists but is not a Unix Domain Socket file, cannot run until removed",
//                 path.display()
//             );
//             std::process::exit(2);
//         } else {
//             // TODO: worth keeping, seems useless.
//             //            debug!("removing existing Unix Domain Socket {}", path.display());
//             let _ = std::fs::remove_file(path);
//         }
//     } else {
//         if let Some(p) = parent
//             && let Ok(_parentage) = std::fs::create_dir_all(p)
//         {
//             debug!(
//                 "created parentage {} Unix Domain Socket {}",
//                 p.display(),
//                 path.display()
//             );
//         } else {
//             error!("fatal: couldn't create parent path for {}", path.display());
//             std::process::exit(2);
//         }
//     };

//     let uds = UnixListener::bind(path).unwrap_or_else(|e| {
//         error!(
//             "fatal: failed to bind unix socket {}: {}",
//             path.display(),
//             e
//         );
//         std::process::exit(2);
//     });

//     let addr_uds = UnixListenerStream::new(uds);

//     let greeter = MyGreeter::default();
//     let loglevel = MyLogLevel::default();

//     let reflection = tonic_reflection::server::Builder::configure()
//         .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
//         .build_v1()
//         .unwrap_or_else(|e| {
//             error!("{}", eyre!("fatal: uds reflection registration failed {e}"));
//             std::process::exit(2);
//         });

//     let uds_server = Server::builder()
//         .add_service(GreeterServer::new(greeter))
//         .add_service(LogLevelServer::new(loglevel))
//         .add_service(reflection)
//         .serve_with_incoming(addr_uds);

//     uds_server.await.unwrap_or_else(|e| {
//         error!("fatal: Couldn't spawn uds server {:?}", e);
//         std::process::exit(2);
//     });

//     info!("uds server started");

//     //    tasks.spawn_background_task(|mut ctx| async move {
//     // let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

//     // if let Err(e) = tokio::try_join!(uds_server, tcp_server) {
//     //     error!("fatal: Couldn't join uds and tcp server together {:?}", e);
//     //     std::process::exit(2);
//     // }
//     // while let Some(event) = rx.recv().await {
//     //     ctx.run_on_main_thread(move |world| {
//     //         use bevy::ecs::world::CommandQueue;
//     //         let mut command_queue = CommandQueue::default();

//     //         // We could manually add our commands to the queue
//     //         command_queue.push(MyCommand);

//     //         // We can apply the commands to a given world:
//     //         command_queue.apply(world);
//     //         //                world.send_event(event);
//     //     })
//     //     .await;
//     // }
//     //    });
// }

// use bevy::ecs::system::Command;

// struct MyCommand;

// impl Command for MyCommand {
//     fn apply(self, world: &mut World) {
//         info!("Hello, world!");
//     }
// }
