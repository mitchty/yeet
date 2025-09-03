use tonic::{
    //transport::Server,
    Request,
    Response,
    Status,
};

use crate::rpc::greeter::greeter::greeter_server::Greeter;
use greeter::{HiReply, HiRequest};

pub mod greeter {
    tonic::include_proto!("greeter");
}

use bevy::prelude::debug;

#[derive(Debug, Default)]
pub struct MyGreeter {}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hi(&self, request: Request<HiRequest>) -> Result<Response<HiReply>, Status> {
        debug!("Got a request: {:?}", request);

        let reply = HiReply {
            message: format!("Hello {}!", request.into_inner().name),
        };

        Ok(Response::new(reply))
    }
}
