use ntex_grpc::client::Connector;

mod helloworld;
mod unique_id;
use self::helloworld::{GreeterClient, HelloRequest};

#[ntex::main]
async fn main() {
    std::env::set_var("RUST_LOG", "trace");
    env_logger::init();

    let connector = Connector::<&'static str, _>::default();
    let client: GreeterClient<_> = connector.create("127.0.0.1:50051").await.unwrap();

    let res = client
        .say_hello(&HelloRequest {
            name: "world".into(),
            msg_id: unique_id::UniqueId::new(),
        })
        .await
        .unwrap();

    println!("RES: {:?}", res);
}
