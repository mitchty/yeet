use std::sync::{Arc, Mutex};

use bevy::prelude::debug;
use greeter::{HiReply, HiRequest};
use tonic::{
    //transport::Server,
    Request,
    Response,
    Status,
};

use crate::RpcEvent;
use crate::rpc::greeter::greeter::greeter_server::Greeter;

pub mod greeter {
    tonic::include_proto!("greeter");
}

#[derive(Debug, Clone)]
pub struct MyGreeter {
    event_sender: Arc<Mutex<tokio::sync::mpsc::UnboundedSender<RpcEvent>>>,
}

impl MyGreeter {
    pub fn new(event_sender: Arc<Mutex<tokio::sync::mpsc::UnboundedSender<RpcEvent>>>) -> Self {
        Self { event_sender }
    }
}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hi(&self, request: Request<HiRequest>) -> Result<Response<HiReply>, Status> {
        debug!("Got a request: {:?}", request);

        let name = request.into_inner().name.clone();

        let s = self
            .event_sender
            .lock()
            .expect("could not lock event sender");
        let _ = s.send(RpcEvent::SpawnSync { name: name.clone() });

        let reply = HiReply {
            message: format!("Hello {}!", name),
        };

        Ok(Response::new(reply))
    }
}
