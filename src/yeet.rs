use tonic::{
    //transport::Server,
    Request,
    Response,
    Status,
};

use yeet::greeter_server::{
    Greeter,
    //    GreeterServer
};
use yeet::{HiReply, HiRequest};

pub mod yeet {
    tonic::include_proto!("yeet");
}

use bevy::prelude::info;

#[derive(Debug, Default)]
pub struct MyGreeter {}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hi(&self, request: Request<HiRequest>) -> Result<Response<HiReply>, Status> {
        info!("Got a request: {:?}", request);

        let reply = HiReply {
            message: format!("Hello {}!", request.into_inner().name),
        };

        Ok(Response::new(reply))
    }
}
