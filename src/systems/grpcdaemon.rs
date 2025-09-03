use bevy::prelude::*;
use bevy_tokio_tasks::{TaskContext, TokioTasksRuntime};
// use tokio::sync::mpsc;
// use tonic::transport::Server;

use crate::rpc::{
    greeter::{MyGreeter, greeter::greeter_server::GreeterServer},
    loglevel::{MyLogLevel, loglevel::log_level_server::LogLevelServer},
};

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

        app.add_systems(Startup, startup);
    }
}

// TODO: wrap me in a cfg unix directive once windows is figured out
//
// At that point figure out the "right" thing to abuse in lieu of UDS for local
// process IPC.

// bevy wrapper system that spawns the async tokio grpc task
fn startup(runtime: ResMut<TokioTasksRuntime>) {
    runtime.spawn_background_task(runserver);
}

// Note the tcp socket might come in more handy for stuff running non locally
// like maybe a web gooey. That... is a future mitch problem though. I couldn't
// build a gooey worth a shit to save my life.
//
// Or even configure to allow multiple listen addresses?
//
// I like domain sockets as then I could skip a bit of logic around local
// authentication, if the local system is compromised its game off anyway.
mod proto {
    tonic::include_proto!("greeter");
    tonic::include_proto!("loglevel");

    pub(crate) const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("reflection");
}

// For future mitch or anyone more able to do windows than me, until
// https://github.com/rust-lang/rust/issues/56533 is resolved maybe this will
// work for a unix domain socket solution on windows for client/daemon
// communication there? https://docs.rs/uds_windows/latest/uds_windows/
//
// Looks like it might be the best solution perf wise on windows
// https://www.yanxurui.cc/posts/server/2023-11-28-benchmark-tcp-uds-namedpipe/
async fn runserver(_ctx: TaskContext) {
    use std::os::unix::fs::FileTypeExt;
    use tokio::net::UnixListener;
    use tokio_stream::wrappers::UnixListenerStream;
    use tonic::transport::Server;

    // Sue me, working first ~/.cache/yeet later.
    let path = std::path::Path::new("/tmp/yeet.uds");

    use std::fs;

    // At some point this won't be in /tmp so mkdir on all parents will be in order
    let parent = path.parent();

    // Ensure our socket file exists, and is actually a unix domain socket and
    // not a file/dir/fifo whatever the hell might be there instead.
    if let Ok(metadata) = fs::metadata(path) {
        if !metadata.file_type().is_socket() {
            error!(
                "{} exists but is not a Unix Domain Socket file, cannot run until removed",
                path.display()
            );
            std::process::exit(2);
        } else {
            // TODO: worth keeping, seems useless.
            //            debug!("removing existing Unix Domain Socket {}", path.display());
            let _ = std::fs::remove_file(path);
        }
    } else {
        if let Some(p) = parent
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
        }
    };

    let uds = match UnixListener::bind(path) {
        Ok(l) => l,
        Err(e) => {
            error!(
                "fatal: failed to bind unix socket {}: {:?}",
                path.display(),
                e
            );
            std::process::exit(2);
        }
    };
    let addr_uds = UnixListenerStream::new(uds);
    let addr = "[::]:50051".parse().unwrap();

    let greeter = MyGreeter::default();
    let loglevel = MyLogLevel::default();

    let reflection = match tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1()
    {
        Ok(r) => r,
        Err(e) => {
            error!("fatal: uds reflection registration failed {:?}", e);
            std::process::exit(2);
        }
    };

    let greetertcp = MyGreeter::default();
    let logleveltcp = MyLogLevel::default();
    let reflectiontcp = match tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1()
    {
        Ok(r) => r,
        Err(e) => {
            error!("fatal: tcp reflection registration failed {:?}", e);
            std::process::exit(2);
        }
    };

    let uds_server = Server::builder()
        .add_service(GreeterServer::new(greeter))
        .add_service(LogLevelServer::new(loglevel))
        .add_service(reflection)
        .serve_with_incoming(addr_uds);

    let tcp_server = Server::builder()
        .add_service(GreeterServer::new(greetertcp))
        .add_service(LogLevelServer::new(logleveltcp))
        .add_service(reflectiontcp)
        .serve(addr);

    info!("started");

    if let Err(e) = tokio::try_join!(uds_server, tcp_server) {
        error!("fatal: Couldn't join uds and tcp server together {:?}", e);
        std::process::exit(2);
    }
}
