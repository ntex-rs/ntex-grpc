use std::thread;

use ntex::{rt::System, server::Server};
use ntex_grpc::{client::Client, server};
use ntex_h2::client as h2;

use counts::{CountsSearchClient, SearchRequest};

mod counts;

#[ntex::test]
async fn search_count() {
    let address = format!("0.0.0.0:{}", 3060);
    let counts_client = CountsSearchClient::new(Client::new(
        h2::Client::with_default(address.clone()).finish(),
    ));
    thread::spawn(move || {
        let sys = System::new("client");
        sys.block_on(async move {
            Server::build()
                .bind("integration", address, move |_| {
                    server::GrpcServer::new(mock_count::MockcountServer)
                })
                .expect("failed to bind server")
                .workers(1)
                .run()
                .await
                .expect("failed to run server");
        });
    });
    let request = SearchRequest {
        query: "test".into(),
    };
    let response = counts_client
        .search(&request)
        .await
        .expect("failed to search");
    dbg!(response);
}

pub mod mock_count {

    use crate::counts::Counts;

    use super::counts::{
        Count, CountsSearch, CountsSearchMethods, SearchRequest as CountsSearchRequest,
        SearchResponse as CountsSearchResponse,
    };
    use ntex::ServiceFactory;
    use ntex_grpc::server;

    #[derive(Clone)]
    pub struct MockcountServer;

    #[server(CountsSearch)]
    impl MockcountServer {
        #[method(Search)]
        async fn search(&self, _request: CountsSearchRequest) -> CountsSearchResponse {
            CountsSearchResponse {
                results: vec![Counts {
                    counts: vec![
                        Count {
                            value: 382.8263,
                            offset: 1,
                            count: 73,
                        },
                        Count {
                            value: 2129.284,
                            offset: 7,
                            count: 394,
                        },
                        Count {
                            value: 6964.8898,
                            offset: 15,
                            count: 1287,
                        },
                        Count {
                            value: 10017.193,
                            offset: 30,
                            count: 2009,
                        },
                    ],
                }],
            }
        }
    }

    impl ServiceFactory<server::ServerRequest> for MockcountServer {
        type Error = server::ServerError;
        type InitError = ();
        type Response = server::ServerResponse;
        type Service = Self;

        #[allow(clippy::no_effect_underscore_binding)]
        async fn create(&self, _configuration: ()) -> Result<Self::Service, Self::InitError> {
            Ok(Self)
        }
    }
}
