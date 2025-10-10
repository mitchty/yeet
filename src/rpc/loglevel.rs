use std::sync::{Arc, Mutex};

use tonic::{
    //transport::Server,
    Request,
    Response,
    Status,
};

use crate::rpc::loglevel::loglevel::log_level_server::LogLevel;
use bevy::prelude::Resource;

pub mod loglevel {
    tonic::include_proto!("loglevel");
}

use bevy::prelude::info;

#[derive(Debug, Clone, Resource)]
pub struct MyLogLevel {
    event_sender: Arc<Mutex<tokio::sync::mpsc::UnboundedSender<crate::RpcEvent>>>,
}

impl MyLogLevel {
    pub fn new(
        event_sender: Arc<Mutex<tokio::sync::mpsc::UnboundedSender<crate::RpcEvent>>>,
    ) -> Self {
        Self { event_sender }
    }
}

// Note: the loglevel rpc/service/whatever avoids the bevy ecs, future me change
// all of this to Insert an Entity/Component and let logging be an ECS level
// thing?
#[tonic::async_trait]
impl LogLevel for MyLogLevel {
    async fn set_level(&self, request: Request<loglevel::Request>) -> Result<Response<()>, Status> {
        info!("Got a set request: {:?}", request);

        let s = self
            .event_sender
            .lock()
            .expect("could not lock event sender");
        let newlevel = crate::rpc::loglevel::loglevel::Level::try_from(request.into_inner().level)
            .unwrap_or(crate::rpc::loglevel::loglevel::Level::Info);
        let _ = s.send(crate::RpcEvent::LogLevel { level: newlevel });

        Ok(Response::new(()))
    }
}
