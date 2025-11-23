use bevy::prelude::debug;
use tonic::{
    //transport::Server,
    Request,
    Response,
    Status,
};

use crate::RpcEvent;
use crate::rpc::yeet::yeet_server::Yeet;

tonic::include_proto!("yeet");

#[derive(Debug, Clone)]
pub struct MyYeet {
    event_sender: std::sync::Arc<std::sync::Mutex<tokio::sync::mpsc::UnboundedSender<RpcEvent>>>,
}

impl MyYeet {
    pub fn new(
        event_sender: std::sync::Arc<
            std::sync::Mutex<tokio::sync::mpsc::UnboundedSender<RpcEvent>>,
        >,
    ) -> Self {
        Self { event_sender }
    }
}

#[tonic::async_trait]
impl Yeet for MyYeet {
    async fn simple_copy(
        &self,
        request: Request<SyncSimpleCopyRequest>,
    ) -> Result<Response<SyncSimpleCopyReply>, Status> {
        use uuid::Uuid;

        debug!("Got a request: {:?}", request);

        // Note the u128 is split into two 64 bit uints to make protobuf
        // "happy", its really a 128 bit uuid, I'm too lazy for bytes for this
        // use case. Its only a protocol level wart. The ECS stores the 128 bit
        // value and we decompose that at the grpc layer only.
        //
        // TODO: if this runs on arm/aarch64, is that an issue? The request will
        // determine the random uuid v4 we will store this as so I don't think
        // it matters in this instance but should validate that its split into
        // u64 integers and those will be sent across the wire "sanely".
        let uuid = Uuid::new_v4().as_u128();

        // let high: u64 = (uuid >> 64) as u64;
        // let low: u64 = uuid as u64;

        let binding = request.into_inner();

        let lhs = binding.lhs.clone();
        let rhs = binding.rhs.clone();

        let writers = binding.writers.map(|w| w as usize);

        let s = self
            .event_sender
            .lock()
            .expect("could not lock event sender");
        let _ = s.send(RpcEvent::SimpleCopySync {
            lhs,
            rhs,
            uuid,
            writers,
        });

        let reply = SyncSimpleCopyReply {
            uuid: uuid::Uuid::from_u128(uuid).to_string(),
        };

        Ok(Response::new(reply))
    }
}
