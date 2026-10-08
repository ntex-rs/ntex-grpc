use ntex::io::Io;
use ntex::service::{Pipeline, cfg::SharedCfg, fn_factory, fn_service};
use ntex::testing::IoTest;
use ntex_bytes::{BytePages, Bytes};
use ntex_grpc::server::{GrpcServer, ServerError, ServerRequest, ServerResponse};
use ntex_h2::{self as h2, client::SimpleClient};
use ntex_http::{HeaderMap, HeaderName, HeaderValue, Method};

const GRPC_STATUS: HeaderName = HeaderName::from_static("grpc-status");
const GRPC_MESSAGE: HeaderName = HeaderName::from_static("grpc-message");
const GRPC_ENCODING: HeaderName = HeaderName::from_static("grpc-encoding");
const GRPC_ACCEPT_ENCODING: HeaderName = HeaderName::from_static("grpc-accept-encoding");

/// Connects a client to a grpc server that answers every call with an
/// empty message.
fn client() -> SimpleClient {
    let (cli, srv) = IoTest::create();
    cli.remote_buffer_cap(1024 * 1024);
    srv.remote_buffer_cap(1024 * 1024);

    let server = GrpcServer::new(fn_factory(async |&()| {
        Ok::<_, std::io::Error>(fn_service(async |_: ServerRequest| {
            Ok::<_, ServerError>(ServerResponse::new(BytePages::default()))
        }))
    }));
    let srv = Io::new(srv, SharedCfg::new("SRV").build());
    ntex::rt::spawn(async move {
        let _ = Pipeline::new((), server).call(srv).await;
    });

    let io = Io::new(cli, SharedCfg::new("CLI").build());
    SimpleClient::new(io, false, "localhost".into())
}

/// Sends a request body, returns the response headers and trailers.
async fn call(
    client: &SimpleClient,
    encoding: Option<&'static str>,
    body: &'static [u8],
) -> (HeaderMap, HeaderMap) {
    let mut hdrs = HeaderMap::new();
    hdrs.insert(
        ntex_http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/grpc"),
    );
    if let Some(enc) = encoding {
        hdrs.insert(GRPC_ENCODING, HeaderValue::from_static(enc));
    }
    let (snd, rcv) = client
        .send(Method::POST, "/test.Svc/Call".into(), hdrs, false)
        .await
        .unwrap();
    snd.send_payload(Bytes::from_static(body), true)
        .await
        .unwrap();

    let mut headers = HeaderMap::new();
    loop {
        let msg = rcv.recv().await.unwrap();
        match msg.kind {
            h2::MessageKind::Headers { headers: h, .. } => headers = h,
            h2::MessageKind::Data(..) => {}
            h2::MessageKind::Eof(h2::StreamEof::Trailers(trailers)) => {
                return (headers, trailers);
            }
            kind => panic!("{kind:?}"),
        }
    }
}

#[ntex::test]
async fn uncompressed() {
    let client = client();
    let (headers, trailers) = call(&client, None, b"\0\0\0\0\0").await;
    assert_eq!(headers.get(GRPC_ACCEPT_ENCODING).unwrap(), "identity");
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");

    // an encoding may be declared, as long as the message is not compressed
    let (_, trailers) = call(&client, Some("gzip"), b"\0\0\0\0\0").await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
}

#[ntex::test]
async fn compressed_flag() {
    let client = client();
    for (encoding, body, status, msg) in [
        (
            None,
            &b"\x01\0\0\0\0"[..],
            "13",
            "Compressed message without grpc-encoding",
        ),
        (
            Some("identity"),
            &b"\x01\0\0\0\0"[..],
            "13",
            "Compressed message without grpc-encoding",
        ),
        (
            Some("gzip"),
            &b"\x01\0\0\0\0"[..],
            "12",
            "Unsupported grpc-encoding: gzip",
        ),
        (
            None,
            &b"\x02\0\0\0\0"[..],
            "13",
            "Invalid compressed flag 2",
        ),
    ] {
        let (headers, trailers) = call(&client, encoding, body).await;
        assert_eq!(headers.get(GRPC_ACCEPT_ENCODING).unwrap(), "identity");
        assert_eq!(trailers.get(GRPC_STATUS).unwrap(), status, "{msg}");
        assert_eq!(trailers.get(GRPC_MESSAGE).unwrap(), msg);
    }
}

#[ntex::test]
async fn short_request() {
    let client = client();
    for body in [&b""[..], &b"\0\0"[..]] {
        let (_, trailers) = call(&client, None, body).await;
        assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "3");
    }

    // the connection still works
    let (_, trailers) = call(&client, None, b"\0\0\0\0\0").await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
}
