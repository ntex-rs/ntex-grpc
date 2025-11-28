use ntex::SharedCfg;
use ntex_grpc::client::Client;
use ntex_h2::client as h2;

mod helloworld;
mod unique_id;
use self::helloworld::{GreeterClient, HelloRequest};

#[ntex::main]
async fn main() {
    // std::env::set_var("RUST_LOG", "trace");
    let _ = env_logger::try_init();

    let client = GreeterClient::new(Client::new(
        h2::Client::with_default("127.0.0.1:50051")
            .finish(SharedCfg::default())
            .await
            .unwrap(),
    ));

    let res = client
        .say_hello(&HelloRequest {
            name: "world".into(),
            msg_id: unique_id::UniqueId::new(),
        })
        .await
        .unwrap();

    println!("RES: {res:?}");
}
