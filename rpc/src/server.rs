use futures::stream;
use helloworld::{ClockRequest, TickEvent, HelloReply, HelloRequest};
use helloworld::greeter_server::{Greeter, GreeterServer};
use std::pin::Pin;
use std::prelude::rust_2021::*;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Duration;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};
use tonic::transport::Server;

pub mod helloworld {
    // include!(concat!(env!("OUT_DIR"), "/helloworld.rs"));
    tonic::include_proto!("helloworld");
}

#[derive(Default, Debug)]
pub struct MyGreeter {}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(&self, request: Request<HelloRequest>) -> Result<Response<HelloReply>, Status> {
        println!("Got a request: {:?}", request);

        let reply = HelloReply {
            message: format!("Hello, {}!", request.into_inner().name)
        };

        // stream::iter(vec!['a', 'b', 'c']);

        Ok(Response::new(reply))
    }

    type SubscribeClockStream = Pin<Box<dyn stream::Stream<Item=Result<TickEvent, Status>> + Send>>;

    async fn subscribe_clock(&self, request: Request<ClockRequest>) -> Result<Response<Self::SubscribeClockStream>, Status> {
        println!("Got a request: {:?}", request);

        let period = Duration::from_millis(request.into_inner().tick_period_millis as u64);

        Ok(Response::new(Box::pin(stream::repeat(())
            .throttle(period)
            .map(|()| Ok(TickEvent { current_time_millis: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64 })))))
        // Err(Status::unimplemented("not implemented"))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "127.0.0.1:50051".parse()?;
    let greeter = MyGreeter::default();

    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve(addr)
        .await?;

    Ok(())
}
