use tonic::{
    //transport::Server,
    Request,
    Response,
    Status,
};

use crate::rpc::loglevel::loglevel::log_level_server::LogLevel;
use loglevel::{LogReply, LogRequest};

pub mod loglevel {
    tonic::include_proto!("loglevel");
}

use bevy::prelude::info;

#[derive(Debug, Default)]
pub struct MyLogLevel {}

#[tonic::async_trait]
impl LogLevel for MyLogLevel {
    async fn get_level(&self, request: Request<LogRequest>) -> Result<Response<LogReply>, Status> {
        info!("Got a request: {:?}", request);

        let reply = LogReply {
            level: format!("Loglevel is {}!", request.into_inner().level),
        };

        Ok(Response::new(reply))
    }
}
