use ntex::{http::header::HeaderValue, server::Server, service::ServiceFactory, util::HashMap};
use ntex_grpc::server;

mod helloworld;
use crate::helloworld::{HelloReply, HelloRequest};

/// Calculator server
#[derive(Clone)]
pub struct GreeterServer;

/// Custom error
struct HelloError;

impl server::GrpcError for HelloError {
    fn status(&self) -> server::GrpcStatus {
        server::GrpcStatus::NotFound
    }

    fn message(&self) -> HeaderValue {
        HeaderValue::from_static("Not found error")
    }
}

#[server(crate::helloworld::Greeter)]
impl GreeterServer {
    #[method(SayHello)]
    async fn say_hello(&self, req: HelloRequest) -> Result<HelloReply, HelloError> {
        log::trace!("Received request: {:#?}", req);
        let mut data3 = HashMap::default();
        data3.insert("1".to_string().into(), 10u32);
        Ok(HelloReply {
            // data5: vec![helloworld::DocumentType::Namespace, helloworld::DocumentType::Quota],
            data5: vec![helloworld::DocumentType::Namespace],
            message: format!("Hello {}!", req.name).into(),
            tag: 1,
            data1: vec![1, 2, 3],
            data2: vec!["1".to_string().into()],
            data3,
            data4: helloworld::DocumentType::Namespace,
            data6: vec![-234234234, 123412414, 45456],
        })
    }
}

impl ServiceFactory<server::ServerRequest> for GreeterServer {
    type Response = server::ServerResponse;
    type Error = server::ServerError;
    type InitError = ();
    type Service = GreeterServer;

    async fn create(&self, _: ()) -> Result<Self::Service, Self::InitError> {
        Ok(GreeterServer)
    }
}

#[ntex::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "trace");
    env_logger::init();

    let matches = clap::App::new("helloworld server")
        .version("0.1")
        .args_from_usage(
            "<port> 'Helloworld server port'
                -t, --threads=[NUMBER] 'number of threads to use'",
        )
        .get_matches();

    let port = matches.value_of("port").unwrap().to_owned();
    let threads = parse_usize_default(matches.value_of("threads"), num_cpus::get());

    // bind to socket
    Server::build()
        .bind("helloworld", format!("0.0.0.0:{}", port), move |_| {
            // create service
            server::GrpcServer::new(GreeterServer)
        })?
        .workers(threads)
        .run()
        .await
}

fn parse_usize_default(input: Option<&str>, default: usize) -> usize {
    input
        .map(|v| {
            v.parse()
                .unwrap_or_else(|_| panic!("not a valid number: {}", v))
        })
        .unwrap_or(default)
}
