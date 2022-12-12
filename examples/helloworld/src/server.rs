use ntex::{server::Server, service::ServiceFactory, util::Ready};
use ntex_grpc::server;

mod helloworld;
use crate::helloworld::{HelloReply, HelloRequest};

/// Calculator server
#[derive(Clone)]
pub struct GreeterServer;

#[server(crate::helloworld::Greeter)]
impl GreeterServer {
    #[method(SayHello)]
    async fn say_hello(&self, req: HelloRequest) -> HelloReply {
        HelloReply {
            result: helloworld::hello_reply::Result::Success(helloworld::ResponseResult {
                message: format!("Hello {}!", req.name).into(),
            }),
            metadata: 10,
            reply_type: helloworld::hello_reply::Type::Web,
        }
    }
}

impl ServiceFactory<server::ServerRequest> for GreeterServer {
    type Response = server::ServerResponse;
    type Error = server::ServerError;
    type InitError = ();
    type Service = GreeterServer;
    type Future = Ready<Self::Service, Self::InitError>;

    fn new_service(&self, _: ()) -> Self::Future {
        Ready::Ok(GreeterServer)
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
