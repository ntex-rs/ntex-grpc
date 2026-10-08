use std::{cell::RefCell, rc::Rc, time::Duration};

use ntex::io::Io;
use ntex::service::{Pipeline, cfg::SharedCfg, fn_service};
use ntex::testing::IoTest;
use ntex_bytes::{ByteString, Bytes};
use ntex_error::Error;
use ntex_grpc::client::{ClientError, Request, Response};
use ntex_grpc::{GrpcStatus, HashMap, MethodDef};
use ntex_h2::{self as h2, client::SimpleClient, frame::Reason, frame::StreamId};
use ntex_http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};

macro_rules! method {
    ($name:ident, $path:literal) => {
        struct $name;

        impl MethodDef for $name {
            const NAME: &'static str = stringify!($name);
            const PATH: ByteString = ByteString::from_static($path);
            type Input = ();
            type Output = ();
        }
    };
}

method!(Message, "/test.Svc/Message");
method!(Short, "/test.Svc/Short");
method!(Deadline, "/test.Svc/Deadline");
method!(Silent, "/test.Svc/Silent");
method!(NoStatus, "/test.Svc/NoStatus");
method!(NoTrailers, "/test.Svc/NoTrailers");
method!(Html, "/test.Svc/Html");
method!(HtmlError, "/test.Svc/HtmlError");
method!(NoContentType, "/test.Svc/NoContentType");
method!(Proto, "/test.Svc/Proto");
method!(Busy, "/test.Svc/Busy");
method!(Missing, "/test.Svc/Missing");
method!(ProxyError, "/test.Svc/ProxyError");
method!(Compressed, "/test.Svc/Compressed");
method!(Gzip, "/test.Svc/Gzip");
method!(BadFlag, "/test.Svc/BadFlag");
method!(UnknownStatus, "/test.Svc/UnknownStatus");
method!(BadStatus, "/test.Svc/BadStatus");
method!(UnknownStatusOnly, "/test.Svc/UnknownStatusOnly");
method!(EncodedMessage, "/test.Svc/EncodedMessage");
method!(UserAgent, "/test.Svc/UserAgent");
method!(Accepted, "/test.Svc/Accepted");
method!(NoContent, "/test.Svc/NoContent");

const X_TEST: HeaderName = HeaderName::from_static("x-test");
const GRPC_STATUS: HeaderName = HeaderName::from_static("grpc-status");
const GRPC_MESSAGE: HeaderName = HeaderName::from_static("grpc-message");
const GRPC_ENCODING: HeaderName = HeaderName::from_static("grpc-encoding");

fn grpc_headers() -> HeaderMap {
    let mut hdrs = HeaderMap::new();
    hdrs.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/grpc"),
    );
    hdrs.insert(X_TEST, HeaderValue::from_static("headers"));
    hdrs
}

fn status_ok() -> HeaderMap {
    let mut hdrs = HeaderMap::new();
    hdrs.insert(GRPC_STATUS, HeaderValue::from_static("0"));
    hdrs
}

fn client() -> SimpleClient {
    client_with_resets().0
}

/// Connects a client to an h2 server that answers each request
/// with a crafted response, selected by the request path.
///
/// Also returns the reasons of streams reset by the client.
fn client_with_resets() -> (SimpleClient, Rc<RefCell<Vec<Reason>>>) {
    let (cli, srv) = IoTest::create();
    cli.remote_buffer_cap(1024 * 1024);
    srv.remote_buffer_cap(1024 * 1024);

    // path and headers of each request
    let paths: Rc<RefCell<HashMap<StreamId, (ByteString, HeaderMap)>>> = Rc::default();
    let resets: Rc<RefCell<Vec<Reason>>> = Rc::default();
    let resets2 = resets.clone();
    let publish = fn_service(move |msg: h2::Message| {
        let paths = paths.clone();
        let resets = resets2.clone();
        async move {
            let stream = msg.stream().clone();
            match msg.kind {
                h2::MessageKind::Headers {
                    pseudo, headers, ..
                } => {
                    let path = pseudo.path.unwrap();
                    paths.borrow_mut().insert(stream.id(), (path, headers));
                }
                h2::MessageKind::Eof(h2::StreamEof::Error(err)) => {
                    if let h2::StreamError::Reset(reason) = *err {
                        resets.borrow_mut().push(reason);
                    }
                }
                h2::MessageKind::Eof(_) => {
                    let (path, req) = paths.borrow_mut().remove(&stream.id()).unwrap();
                    match path.as_ref() {
                        // a regular reply: one empty message, then trailers
                        "/test.Svc/Message" => {
                            stream
                                .send_response(StatusCode::OK, grpc_headers(), false)
                                .unwrap();
                            stream
                                .send_payload(Bytes::from_static(b"\0\0\0\0\0"), false)
                                .await
                                .unwrap();
                            stream.send_trailers(status_ok()).unwrap();
                        }
                        // the body is shorter than the 5 byte frame prefix
                        "/test.Svc/Short" => {
                            stream
                                .send_response(StatusCode::OK, grpc_headers(), false)
                                .unwrap();
                            stream
                                .send_payload(Bytes::from_static(b"\0\0"), false)
                                .await
                                .unwrap();
                            stream.send_trailers(status_ok()).unwrap();
                        }
                        // a headers-only reply that carries the grpc status
                        "/test.Svc/Deadline" => {
                            let mut hdrs = grpc_headers();
                            hdrs.insert(GRPC_STATUS, HeaderValue::from_static("4"));
                            stream.send_response(StatusCode::OK, hdrs, true).unwrap();
                        }
                        // a message, then trailers without grpc-status
                        "/test.Svc/NoStatus" => {
                            stream
                                .send_response(StatusCode::OK, grpc_headers(), false)
                                .unwrap();
                            stream
                                .send_payload(Bytes::from_static(b"\0\0\0\0\0"), false)
                                .await
                                .unwrap();
                            let mut hdrs = HeaderMap::new();
                            hdrs.insert(X_TEST, HeaderValue::from_static("trailers"));
                            stream.send_trailers(hdrs).unwrap();
                        }
                        // a message that ends the stream, no trailers
                        "/test.Svc/NoTrailers" => {
                            stream
                                .send_response(StatusCode::OK, grpc_headers(), false)
                                .unwrap();
                            stream
                                .send_payload(Bytes::from_static(b"\0\0\0\0\0"), true)
                                .await
                                .unwrap();
                        }
                        // a regular reply with a non-grpc content-type
                        "/test.Svc/Html" => {
                            let mut hdrs = grpc_headers();
                            hdrs.insert(
                                header::CONTENT_TYPE,
                                HeaderValue::from_static("text/html"),
                            );
                            reply(&stream, hdrs, status_ok()).await;
                        }
                        // a non-grpc content-type, but with an error status
                        "/test.Svc/HtmlError" => {
                            let mut hdrs = grpc_headers();
                            hdrs.insert(
                                header::CONTENT_TYPE,
                                HeaderValue::from_static("text/html"),
                            );
                            let mut trailers = HeaderMap::new();
                            trailers.insert(GRPC_STATUS, HeaderValue::from_static("13"));
                            reply(&stream, hdrs, trailers).await;
                        }
                        // a regular reply without content-type
                        "/test.Svc/NoContentType" => {
                            let mut hdrs = grpc_headers();
                            hdrs.remove(header::CONTENT_TYPE);
                            reply(&stream, hdrs, status_ok()).await;
                        }
                        // a regular reply with a content-type suffix
                        "/test.Svc/Proto" => {
                            let mut hdrs = grpc_headers();
                            hdrs.insert(
                                header::CONTENT_TYPE,
                                HeaderValue::from_static("Application/gRPC+proto"),
                            );
                            reply(&stream, hdrs, status_ok()).await;
                        }
                        // an http error with a body, as a proxy would send
                        "/test.Svc/Busy" => {
                            let mut hdrs = grpc_headers();
                            hdrs.insert(
                                header::CONTENT_TYPE,
                                HeaderValue::from_static("text/plain"),
                            );
                            stream
                                .send_response(StatusCode::SERVICE_UNAVAILABLE, hdrs, false)
                                .unwrap();
                            stream
                                .send_payload(Bytes::from_static(b"busy"), true)
                                .await
                                .unwrap();
                        }
                        // a headers-only http error
                        "/test.Svc/Missing" => {
                            stream
                                .send_response(StatusCode::NOT_FOUND, grpc_headers(), true)
                                .unwrap();
                        }
                        // a headers-only http error that carries the grpc status
                        "/test.Svc/ProxyError" => {
                            let mut hdrs = grpc_headers();
                            hdrs.insert(GRPC_STATUS, HeaderValue::from_static("8"));
                            stream
                                .send_response(StatusCode::INTERNAL_SERVER_ERROR, hdrs, true)
                                .unwrap();
                        }
                        // compressed messages, the client only accepts identity
                        "/test.Svc/Compressed" => {
                            reply_with(&stream, grpc_headers(), b"\x01\0\0\0\0").await;
                        }
                        "/test.Svc/Gzip" => {
                            let mut hdrs = grpc_headers();
                            hdrs.insert(GRPC_ENCODING, HeaderValue::from_static("gzip"));
                            reply_with(&stream, hdrs, b"\x01\0\0\0\0").await;
                        }
                        "/test.Svc/BadFlag" => {
                            reply_with(&stream, grpc_headers(), b"\x02\0\0\0\0").await;
                        }
                        // status codes the client does not know
                        "/test.Svc/UnknownStatus" => {
                            let mut trailers = HeaderMap::new();
                            trailers.insert(GRPC_STATUS, HeaderValue::from_static("17"));
                            trailers.insert(GRPC_MESSAGE, HeaderValue::from_static("boom"));
                            reply(&stream, grpc_headers(), trailers).await;
                        }
                        "/test.Svc/BadStatus" => {
                            let mut trailers = HeaderMap::new();
                            trailers.insert(GRPC_STATUS, HeaderValue::from_static("abc"));
                            reply(&stream, grpc_headers(), trailers).await;
                        }
                        "/test.Svc/UnknownStatusOnly" => {
                            let mut hdrs = grpc_headers();
                            hdrs.insert(GRPC_STATUS, HeaderValue::from_static("999"));
                            stream.send_response(StatusCode::OK, hdrs, true).unwrap();
                        }
                        "/test.Svc/EncodedMessage" => {
                            let mut trailers = HeaderMap::new();
                            trailers.insert(GRPC_STATUS, HeaderValue::from_static("3"));
                            trailers.insert(
                                GRPC_MESSAGE,
                                HeaderValue::from_static("100%25 bad%0Ainput %E2%82%AC"),
                            );
                            reply(&stream, grpc_headers(), trailers).await;
                        }
                        // echoes the request user-agent
                        "/test.Svc/UserAgent" => {
                            let mut hdrs = grpc_headers();
                            if let Some(ua) = req.get(header::USER_AGENT) {
                                hdrs.insert(header::USER_AGENT, ua.clone());
                            }
                            reply(&stream, hdrs, status_ok()).await;
                        }
                        // a valid reply, but with 202
                        "/test.Svc/Accepted" => {
                            stream
                                .send_response(StatusCode::ACCEPTED, grpc_headers(), false)
                                .unwrap();
                            stream
                                .send_payload(Bytes::from_static(b"\0\0\0\0\0"), false)
                                .await
                                .unwrap();
                            stream.send_trailers(status_ok()).unwrap();
                        }
                        // headers only 204
                        "/test.Svc/NoContent" => {
                            stream
                                .send_response(StatusCode::NO_CONTENT, grpc_headers(), true)
                                .unwrap();
                        }
                        // never replies
                        "/test.Svc/Silent" => {}
                        _ => panic!("unexpected request {path}"),
                    }
                }
                _ => {}
            }
            Ok::<_, h2::StreamError>(())
        }
    });
    let control = fn_service(async |msg: h2::Control<h2::StreamError>| Ok::<_, ()>(msg.ack()));

    let srv = Io::new(srv, SharedCfg::new("SRV").build());
    ntex::rt::spawn(async move {
        let _ = h2::server::handle_one(
            srv.into(),
            Pipeline::new((), publish),
            Pipeline::new((), control),
        )
        .await;
    });

    let io = Io::new(cli, SharedCfg::new("CLI").build());
    (SimpleClient::new(io, false, "localhost".into()), resets)
}

/// Sends response headers, the body and an OK status.
async fn reply_with(stream: &h2::StreamRef, hdrs: HeaderMap, body: &'static [u8]) {
    stream.send_response(StatusCode::OK, hdrs, false).unwrap();
    stream
        .send_payload(Bytes::from_static(body), false)
        .await
        .unwrap();
    stream.send_trailers(status_ok()).unwrap();
}

/// Sends response headers, one empty message and trailers.
async fn reply(stream: &h2::StreamRef, hdrs: HeaderMap, trailers: HeaderMap) {
    stream.send_response(StatusCode::OK, hdrs, false).unwrap();
    stream
        .send_payload(Bytes::from_static(b"\0\0\0\0\0"), false)
        .await
        .unwrap();
    stream.send_trailers(trailers).unwrap();
}

async fn send<M: MethodDef<Input = ()>>(
    client: &SimpleClient,
) -> Result<Response<M>, Error<ClientError>> {
    Request::<_, M>::new(client, &()).send().await
}

#[ntex::test]
async fn response_size() {
    let client = client();
    let res = send::<Message>(&client).await.unwrap();
    assert_eq!(res.req_size, 5);
    assert_eq!(res.res_size, 5);
    assert_eq!(res.headers().get(X_TEST).unwrap(), "headers");
    assert_eq!(res.trailers().get(GRPC_STATUS).unwrap(), "0");

    let dbg = format!("{res:?}");
    assert!(dbg.contains("trailers: {\"grpc-status\""), "{dbg}");
}

#[ntex::test]
async fn trailers_without_status() {
    let client = client();
    let err = send::<NoStatus>(&client).await.unwrap_err();
    let ClientError::GrpcStatus(status, trailers, body) = &*err else {
        panic!("{err:?}");
    };
    assert_eq!(*status, GrpcStatus::Unknown);
    assert_eq!(trailers.get(X_TEST).unwrap(), "trailers");
    assert_eq!(
        trailers.get(GRPC_MESSAGE).unwrap(),
        "Response trailers have no grpc-status"
    );
    assert_eq!(body.as_deref(), Some(&b"\0\0\0\0\0"[..]));
}

#[ntex::test]
async fn eof_without_trailers() {
    let client = client();
    let err = send::<NoTrailers>(&client).await.unwrap_err();
    let ClientError::GrpcStatus(status, trailers, body) = &*err else {
        panic!("{err:?}");
    };
    assert_eq!(*status, GrpcStatus::Internal);
    assert_eq!(trailers.len(), 1);
    assert_eq!(
        trailers.get(GRPC_MESSAGE).unwrap(),
        "Response ended without trailers"
    );
    assert_eq!(body.as_deref(), Some(&b"\0\0\0\0\0"[..]));
}

#[ntex::test]
async fn invalid_content_type() {
    let client = client();
    let err = send::<Html>(&client).await.unwrap_err();
    let ClientError::GrpcStatus(status, trailers, body) = &*err else {
        panic!("{err:?}");
    };
    assert_eq!(*status, GrpcStatus::Unknown);
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
    assert_eq!(
        trailers.get(GRPC_MESSAGE).unwrap(),
        "Invalid content-type: text/html"
    );
    assert_eq!(body.as_deref(), Some(&b"\0\0\0\0\0"[..]));

    let err = send::<NoContentType>(&client).await.unwrap_err();
    let ClientError::GrpcStatus(status, trailers, body) = &*err else {
        panic!("{err:?}");
    };
    assert_eq!(*status, GrpcStatus::Unknown);
    assert_eq!(
        trailers.get(GRPC_MESSAGE).unwrap(),
        "Response has no content-type"
    );
    assert!(body.is_some());

    // the server's own error status is kept
    let err = send::<HtmlError>(&client).await.unwrap_err();
    let ClientError::GrpcStatus(status, trailers, body) = &*err else {
        panic!("{err:?}");
    };
    assert_eq!(*status, GrpcStatus::Internal);
    assert!(trailers.get(GRPC_MESSAGE).is_none());
    assert!(body.is_none());

    // the content-type may carry a suffix and is case-insensitive
    send::<Proto>(&client).await.unwrap();
}

#[ntex::test]
async fn unknown_grpc_status() {
    let client = client();

    // the server's message is kept
    let err = send::<UnknownStatus>(&client).await.unwrap_err();
    let ClientError::GrpcStatus(status, trailers, body) = &*err else {
        panic!("{err:?}");
    };
    assert_eq!(*status, GrpcStatus::Unknown);
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "17");
    assert_eq!(trailers.get(GRPC_MESSAGE).unwrap(), "boom");
    assert_eq!(body.as_deref(), Some(&b"\0\0\0\0\0"[..]));

    let err = send::<BadStatus>(&client).await.unwrap_err();
    let ClientError::GrpcStatus(status, trailers, body) = &*err else {
        panic!("{err:?}");
    };
    assert_eq!(*status, GrpcStatus::Unknown);
    assert_eq!(
        trailers.get(GRPC_MESSAGE).unwrap(),
        "Unknown grpc-status: abc"
    );
    assert_eq!(body.as_deref(), Some(&b"\0\0\0\0\0"[..]));

    // headers-only response
    let err = send::<UnknownStatusOnly>(&client).await.unwrap_err();
    let ClientError::GrpcStatus(status, hdrs, body) = &*err else {
        panic!("{err:?}");
    };
    assert_eq!(*status, GrpcStatus::Unknown);
    assert_eq!(hdrs.get(X_TEST).unwrap(), "headers");
    assert_eq!(hdrs.get(GRPC_MESSAGE).unwrap(), "Unknown grpc-status: 999");
    assert_eq!(body.as_deref(), Some(&b""[..]));
}

#[ntex::test]
async fn grpc_message_decoded() {
    let client = client();
    let err = send::<EncodedMessage>(&client).await.unwrap_err();
    let ClientError::GrpcStatus(status, trailers, _) = &*err else {
        panic!("{err:?}");
    };
    assert_eq!(*status, GrpcStatus::InvalidArgument);
    // the trailers keep the raw value
    assert_eq!(
        trailers.get(GRPC_MESSAGE).unwrap(),
        "100%25 bad%0Ainput %E2%82%AC"
    );
    assert_eq!(err.grpc_message().unwrap(), "100% bad\ninput \u{20ac}");

    // plain synthesized messages read the same
    let err = send::<BadStatus>(&client).await.unwrap_err();
    assert_eq!(err.grpc_message().unwrap(), "Unknown grpc-status: abc");

    assert!(
        ClientError::UnexpectedEof(None, HeaderMap::new())
            .grpc_message()
            .is_none()
    );
}

#[ntex::test]
async fn user_agent() {
    let client = client();
    let res = send::<UserAgent>(&client).await.unwrap();
    assert_eq!(
        res.headers().get(header::USER_AGENT).unwrap(),
        concat!("grpc-rust-ntex/", env!("CARGO_PKG_VERSION"))
    );
}

#[ntex::test]
async fn http_status() {
    let client = client();
    let err = send::<Busy>(&client).await.unwrap_err();
    let ClientError::GrpcStatus(status, hdrs, body) = &*err else {
        panic!("{err:?}");
    };
    assert_eq!(*status, GrpcStatus::Unavailable);
    assert_eq!(hdrs.get(X_TEST).unwrap(), "headers");
    assert_eq!(hdrs.get(GRPC_MESSAGE).unwrap(), "HTTP status 503");
    assert_eq!(body.as_deref(), Some(&b"busy"[..]));

    let err = send::<Missing>(&client).await.unwrap_err();
    let ClientError::GrpcStatus(status, hdrs, body) = &*err else {
        panic!("{err:?}");
    };
    assert_eq!(*status, GrpcStatus::Unimplemented);
    assert_eq!(hdrs.get(GRPC_MESSAGE).unwrap(), "HTTP status 404");
    assert_eq!(body.as_deref(), Some(&b""[..]));

    // only 200 is accepted
    fn assert_unknown(err: &Error<ClientError>, code: u16) {
        let ClientError::GrpcStatus(status, hdrs, _) = &**err else {
            panic!("{err:?}");
        };
        assert_eq!(*status, GrpcStatus::Unknown);
        assert_eq!(
            hdrs.get(GRPC_MESSAGE).unwrap(),
            &format!("HTTP status {code}")
        );
    }
    assert_unknown(&send::<Accepted>(&client).await.unwrap_err(), 202);
    assert_unknown(&send::<NoContent>(&client).await.unwrap_err(), 204);

    // grpc-status wins over the http status
    let err = send::<ProxyError>(&client).await.unwrap_err();
    let ClientError::GrpcStatus(status, hdrs, body) = &*err else {
        panic!("{err:?}");
    };
    assert_eq!(*status, GrpcStatus::ResourceExhausted);
    assert!(hdrs.get(GRPC_MESSAGE).is_none());
    assert!(body.is_none());
}

#[ntex::test]
async fn compressed_flag() {
    let client = client();
    for (err, msg, body) in [
        (
            send::<Compressed>(&client).await.unwrap_err(),
            "Compressed message without grpc-encoding",
            &b"\x01\0\0\0\0"[..],
        ),
        (
            send::<Gzip>(&client).await.unwrap_err(),
            "Unsupported grpc-encoding: gzip",
            &b"\x01\0\0\0\0"[..],
        ),
        (
            send::<BadFlag>(&client).await.unwrap_err(),
            "Invalid compressed flag 2",
            &b"\x02\0\0\0\0"[..],
        ),
    ] {
        let ClientError::GrpcStatus(status, trailers, data) = &*err else {
            panic!("{err:?}");
        };
        assert_eq!(*status, GrpcStatus::Internal);
        assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
        assert_eq!(trailers.get(GRPC_MESSAGE).unwrap(), msg);
        assert_eq!(data.as_deref(), Some(body));
    }
}

#[ntex::test]
async fn short_body() {
    let client = client();
    let err = send::<Short>(&client).await.unwrap_err();
    assert!(
        matches!(*err, ClientError::UnexpectedEof(Some(StatusCode::OK), _)),
        "{err:?}"
    );
}

#[ntex::test]
async fn deadline_headers() {
    let client = client();
    let err = send::<Deadline>(&client).await.unwrap_err();
    let ClientError::DeadlineExceeded(ref hdrs) = *err else {
        panic!("{err:?}")
    };
    assert_eq!(hdrs.get(X_TEST).unwrap(), "headers");
}

#[ntex::test]
async fn client_timeout() {
    let (client, resets) = client_with_resets();
    let mut req = Request::<_, Silent>::new(&client, &());
    req.timeout(Duration::from_millis(50));
    let err = ntex::time::timeout(Duration::from_secs(5), req.send())
        .await
        .expect("the client must stop waiting by itself")
        .unwrap_err();
    let ClientError::DeadlineExceeded(ref hdrs) = *err else {
        panic!("{err:?}")
    };
    assert!(hdrs.is_empty());

    // the connection stays usable, the server sees the stream reset
    let res = send::<Message>(&client).await.unwrap();
    assert_eq!(res.res_size, 5);
    assert_eq!(*resets.borrow(), [Reason::CANCEL]);
}

#[ntex::test]
async fn zero_timeout() {
    let (client, resets) = client_with_resets();
    let mut req = Request::<_, Silent>::new(&client, &());
    req.timeout(Duration::ZERO);
    let err = ntex::time::timeout(Duration::from_secs(5), req.send())
        .await
        .expect("the deadline has already passed")
        .unwrap_err();
    let ClientError::DeadlineExceeded(ref hdrs) = *err else {
        panic!("{err:?}")
    };
    assert!(hdrs.is_empty());

    // nothing is sent, so there is no stream to reset
    let res = send::<Message>(&client).await.unwrap();
    assert_eq!(res.res_size, 5);
    assert!(resets.borrow().is_empty());
}

#[ntex::test]
async fn timeout_not_reached() {
    let client = client();
    let mut req = Request::<_, Message>::new(&client, &());
    req.timeout(Duration::from_secs(5));
    let res = req.send().await.unwrap();
    assert_eq!(res.res_size, 5);
}

#[ntex::test]
async fn invalid_header() {
    let client = client();
    let mut req = Request::<_, Message>::new(&client, &());
    req.header("bad header", "value");
    let err = req.send().await.unwrap_err();
    assert!(matches!(*err, ClientError::Http(_)), "{err:?}");

    // the bad header is not sent and the connection stays usable
    let res = send::<Message>(&client).await.unwrap();
    assert_eq!(res.res_size, 5);
}

#[test]
fn client_error_display() {
    let err = ClientError::from(ntex_h2::client::ClientError::Disconnected(
        std::io::Error::other("test"),
    ));
    assert_eq!(err.to_string(), "HTTP2 Client");
}

#[test]
fn duration_errors() {
    use ntex_grpc::google_types::{Duration, NegativeDurationError, OutOfRangeDurationError};

    let err: NegativeDurationError = std::time::Duration::try_from(Duration {
        seconds: -1,
        nanos: 0,
    })
    .unwrap_err();
    assert_eq!(err.to_string(), "Duration is negative: -1s");

    let err: OutOfRangeDurationError =
        Duration::try_from(std::time::Duration::from_secs(u64::MAX)).unwrap_err();
    let err: Box<dyn std::error::Error> = err.into();
    assert_eq!(err.to_string(), "Duration is out of range");
}
