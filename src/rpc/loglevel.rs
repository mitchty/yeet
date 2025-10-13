use bevy::prelude::Resource;

tonic::include_proto!("loglevel");

use crate::rpc::loglevel::log_level_server::LogLevel;

use bevy::prelude::info;

#[derive(Debug, Clone, Resource)]
pub struct MyLogLevel {
    event_sender:
        std::sync::Arc<std::sync::Mutex<tokio::sync::mpsc::UnboundedSender<crate::RpcEvent>>>,
}

impl MyLogLevel {
    pub fn new(
        event_sender: std::sync::Arc<
            std::sync::Mutex<tokio::sync::mpsc::UnboundedSender<crate::RpcEvent>>,
        >,
    ) -> Self {
        Self { event_sender }
    }
}

// Note: the loglevel rpc/service/whatever avoids the bevy ecs, future me change
// all of this to Insert an Entity/Component and let logging be an ECS level
// thing?
#[tonic::async_trait]
impl LogLevel for MyLogLevel {
    async fn set_level(
        &self,
        request: tonic::Request<Request>,
    ) -> Result<tonic::Response<()>, tonic::Status> {
        info!("Got a set request: {:?}", request);

        let s = self
            .event_sender
            .lock()
            .expect("could not lock event sender");
        let newlevel = Level::try_from(request.into_inner().level).unwrap_or(Level::Info);
        let _ = s.send(crate::RpcEvent::LogLevel { level: newlevel });

        Ok(tonic::Response::new(()))
    }
}
