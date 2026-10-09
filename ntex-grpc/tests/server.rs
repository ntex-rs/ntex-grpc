use ntex::io::{Io, IoBoxed};
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
const GRPC_TIMEOUT: HeaderName = HeaderName::from_static("grpc-timeout");
const X_TEST: HeaderName = HeaderName::from_static("x-test");
const X_EXTRA: HeaderName = HeaderName::from_static("x-extra");
#[cfg(feature = "compression")]
const ACCEPT_ENCODING: &str = "gzip,zstd";
#[cfg(not(feature = "compression"))]
const ACCEPT_ENCODING: &str = "identity";

/// Connects a client to a grpc server that answers every call with the
/// request message.
fn client() -> SimpleClient {
    client_with(None)
}

/// Like [`client`], with the server's limit of a request message.
fn client_with(max_size: Option<usize>) -> SimpleClient {
    client_limits(max_size, None)
}

/// Like [`client`], with the server's limits of a request and a response
/// message.
fn client_limits(max_size: Option<usize>, max_send_size: Option<usize>) -> SimpleClient {
    start(max_size, max_send_size, false)
}

/// Like [`client`], with the server behind a boxed io.
fn client_boxed() -> SimpleClient {
    start(None, None, true)
}

/// Answers a call, the behaviour is picked by the method name.
async fn handle(req: ServerRequest) -> Result<ServerResponse, ServerError> {
    let mut payload = BytePages::default();
    payload.append(req.payload);

    match req.name.as_ref() {
        // fails with a status, a message and an extra trailer
        "Fail" => {
            let mut hdrs = HeaderMap::new();
            hdrs.insert(X_TEST, HeaderValue::from_static("error"));
            Err(ServerError::new(
                ntex_grpc::GrpcStatus::NotFound,
                HeaderValue::from_static("nothing here"),
                Some(hdrs),
            ))
        }
        // answers after the deadline of the tests has passed
        "Slow" => {
            ntex::time::sleep(ntex::time::Millis(5_000)).await;
            Ok(ServerResponse::new(payload))
        }
        // echoes the `x-test` header of the request in the trailers
        "Trailers" => {
            let val = req
                .headers
                .get(X_TEST)
                .cloned()
                .unwrap_or_else(|| HeaderValue::from_static("none"));
            Ok(ServerResponse::with_headers(
                payload,
                vec![
                    (X_TEST, val),
                    (X_EXTRA, HeaderValue::from_static("1")),
                    // ignored, the call succeeded
                    (GRPC_STATUS, HeaderValue::from_static("13")),
                    (GRPC_MESSAGE, HeaderValue::from_static("ignored")),
                ],
            ))
        }
        _ => Ok(ServerResponse::new(payload)),
    }
}

/// Spawns a server and connects a client to it.
fn start(max_size: Option<usize>, max_send_size: Option<usize>, boxed: bool) -> SimpleClient {
    let (cli, srv) = IoTest::create();
    cli.remote_buffer_cap(64 * 1024 * 1024);
    srv.remote_buffer_cap(64 * 1024 * 1024);

    let server = GrpcServer::new(fn_factory(async |&()| {
        Ok::<_, std::io::Error>(fn_service(handle))
    }));
    let server = match max_size {
        Some(size) => server.max_message_size(size),
        None => server,
    };
    let server = match max_send_size {
        Some(size) => server.max_send_message_size(size),
        None => server,
    };
    let srv = Io::new(srv, SharedCfg::new("SRV").build());
    ntex::rt::spawn(async move {
        if boxed {
            let _ = Pipeline::new((), server).call(IoBoxed::from(srv)).await;
        } else {
            let _ = Pipeline::new((), server).call(srv).await;
        }
    });

    let io = Io::new(cli, SharedCfg::new("CLI").build());
    SimpleClient::new(io, false, "localhost".into())
}

/// Request headers of a call.
fn req_headers(encoding: Option<&'static str>) -> HeaderMap {
    let mut hdrs = HeaderMap::new();
    hdrs.insert(
        ntex_http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/grpc"),
    );
    if let Some(enc) = encoding {
        hdrs.insert(GRPC_ENCODING, HeaderValue::from_static(enc));
    }
    hdrs
}

/// Sends a request body, returns the response headers, body and trailers.
async fn call(
    client: &SimpleClient,
    encoding: Option<&'static str>,
    body: impl Into<Bytes>,
) -> (HeaderMap, Vec<u8>, HeaderMap) {
    call_to(client, "/test.Svc/Call", req_headers(encoding), body).await
}

/// Like [`call`], with the method path and the request headers.
async fn call_to(
    client: &SimpleClient,
    path: &'static str,
    hdrs: HeaderMap,
    body: impl Into<Bytes>,
) -> (HeaderMap, Vec<u8>, HeaderMap) {
    let (snd, rcv) = client
        .send(Method::POST, path.into(), hdrs, false)
        .await
        .unwrap();
    // the server can reject the request and reset the stream before the body
    // is sent
    let _ = snd.send_payload(body.into(), true).await;

    let mut headers = HeaderMap::new();
    let mut data = Vec::new();
    loop {
        let msg = rcv.recv().await.unwrap();
        match msg.kind {
            h2::MessageKind::Headers { headers: h, .. } => headers = h,
            h2::MessageKind::Data(chunk, cap) => {
                data.extend_from_slice(&chunk);
                cap.consume(chunk.len() as u32);
            }
            h2::MessageKind::Eof(h2::StreamEof::Trailers(trailers)) => {
                return (headers, data, trailers);
            }
            kind => panic!("{kind:?}"),
        }
    }
}

#[ntex::test]
async fn uncompressed() {
    let client = client();
    let (headers, data, trailers) = call(&client, None, &b"\0\0\0\0\0"[..]).await;
    assert_eq!(headers.get(GRPC_ACCEPT_ENCODING).unwrap(), ACCEPT_ENCODING);
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
    assert_eq!(data, b"\0\0\0\0\0");

    // an encoding may be declared, as long as the message is not compressed
    let (_, _, trailers) = call(&client, Some("snappy"), &b"\0\0\0\0\0"[..]).await;
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
            Some("snappy"),
            &b"\x01\0\0\0\0"[..],
            "12",
            "Unsupported grpc-encoding: snappy",
        ),
        (
            None,
            &b"\x02\0\0\0\0"[..],
            "13",
            "Invalid compressed flag 2",
        ),
    ] {
        let (headers, _, trailers) = call(&client, encoding, body).await;
        assert_eq!(headers.get(GRPC_ACCEPT_ENCODING).unwrap(), ACCEPT_ENCODING);
        assert_eq!(trailers.get(GRPC_STATUS).unwrap(), status, "{msg}");
        assert_eq!(trailers.get(GRPC_MESSAGE).unwrap(), msg);
    }
}

#[ntex::test]
async fn short_request() {
    let client = client();
    for body in [&b""[..], &b"\0\0"[..], &b"\0\0\0\0\x03ab"[..]] {
        let (_, _, trailers) = call(&client, None, body).await;
        assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "3");
        assert_eq!(
            trailers.get(GRPC_MESSAGE).unwrap(),
            "grpc: request message is truncated"
        );
    }

    // headers end the stream
    let mut hdrs = HeaderMap::new();
    hdrs.insert(
        ntex_http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/grpc"),
    );
    let (_, rcv) = client
        .send(Method::POST, "/test.Svc/Call".into(), hdrs, true)
        .await
        .unwrap();
    loop {
        match rcv.recv().await.unwrap().kind {
            h2::MessageKind::Headers { .. } => {}
            h2::MessageKind::Eof(h2::StreamEof::Trailers(trailers)) => {
                assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "3");
                assert_eq!(
                    trailers.get(GRPC_MESSAGE).unwrap(),
                    "grpc: request without a message"
                );
                break;
            }
            kind => panic!("{kind:?}"),
        }
    }

    // the connection still works
    let (_, _, trailers) = call(&client, None, &b"\0\0\0\0\0"[..]).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
}

#[ntex::test]
async fn malformed_path() {
    let client = client();
    for eof in [true, false] {
        // dropping the sender would cancel the stream
        let (_snd, rcv) = client
            .send(Method::POST, "/test.Svc".into(), req_headers(None), eof)
            .await
            .unwrap();
        // a trailers-only response
        match rcv.recv().await.unwrap().kind {
            h2::MessageKind::Headers {
                pseudo,
                headers,
                eof,
            } => {
                assert_eq!(pseudo.status, Some(ntex_http::StatusCode::OK));
                assert_eq!(headers.get(GRPC_STATUS).unwrap(), "12");
                assert_eq!(
                    headers.get(GRPC_MESSAGE).unwrap(),
                    "grpc: malformed method name: /test.Svc"
                );
                assert!(eof);
            }
            kind => panic!("{kind:?}"),
        }
    }

    // the connection still works
    let (_, _, trailers) = call(&client, None, &b"\0\0\0\0\0"[..]).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
}

#[ntex::test]
async fn unsupported_method() {
    let client = client();
    for (method, eof) in [(Method::GET, true), (Method::PUT, false)] {
        let (_snd, rcv) = client
            .send(
                method.clone(),
                "/test.Svc/Call".into(),
                req_headers(None),
                eof,
            )
            .await
            .unwrap();
        match rcv.recv().await.unwrap().kind {
            h2::MessageKind::Headers {
                pseudo,
                headers,
                eof,
            } => {
                assert_eq!(
                    pseudo.status,
                    Some(ntex_http::StatusCode::METHOD_NOT_ALLOWED)
                );
                assert!(eof);
                assert_eq!(headers.get(GRPC_STATUS).unwrap(), "13");
                assert_eq!(
                    headers.get(GRPC_MESSAGE).unwrap(),
                    &*format!("grpc: method {method} is not supported")
                );
            }
            kind => panic!("{kind:?}"),
        }
    }
}

#[ntex::test]
async fn unsupported_content_type() {
    let client = client();
    for (ct, eof) in [(Some("application/json"), false), (None, true)] {
        let mut hdrs = HeaderMap::new();
        if let Some(ct) = ct {
            hdrs.insert(
                ntex_http::header::CONTENT_TYPE,
                HeaderValue::from_static(ct),
            );
        }
        let (_snd, rcv) = client
            .send(Method::POST, "/test.Svc/Call".into(), hdrs, eof)
            .await
            .unwrap();
        match rcv.recv().await.unwrap().kind {
            h2::MessageKind::Headers {
                pseudo,
                headers,
                eof,
            } => {
                assert_eq!(
                    pseudo.status,
                    Some(ntex_http::StatusCode::UNSUPPORTED_MEDIA_TYPE)
                );
                assert!(eof);
                assert_eq!(headers.get(GRPC_STATUS).unwrap(), "3");
                let msg = headers.get(GRPC_MESSAGE).unwrap();
                assert!(
                    msg.to_str()
                        .unwrap()
                        .starts_with("grpc: invalid request content-type")
                );
            }
            kind => panic!("{kind:?}"),
        }
    }

    // `+format` is a grpc request
    let mut hdrs = HeaderMap::new();
    hdrs.insert(
        ntex_http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/grpc+proto"),
    );
    let (_, _, trailers) = call_to(&client, "/test.Svc/Call", hdrs, &b"\0\0\0\0\0"[..]).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
}

/// Returns `data` with a length prefix.
fn message(flag: u8, data: &[u8]) -> Vec<u8> {
    let mut msg = vec![flag];
    msg.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
    msg.extend_from_slice(data);
    msg
}

#[ntex::test]
async fn max_message_size() {
    let client = client_with(Some(100));
    let (_, data, trailers) = call(&client, None, message(0, &[1; 100])).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
    assert_eq!(data, message(0, &[1; 100]));

    let (_, data, trailers) = call(&client, None, message(0, &[1; 101])).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "8");
    assert_eq!(
        trailers.get(GRPC_MESSAGE).unwrap(),
        "grpc: received message larger than max (101 vs. 100)"
    );
    assert!(data.is_empty());

    // 4 MiB by default
    let client = client_with(None);
    let max = 4 * 1024 * 1024;
    let (_, _, trailers) = call(&client, None, message(0, &vec![1; max])).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
    let (_, _, trailers) = call(&client, None, message(0, &vec![1; max + 1])).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "8");
}

#[ntex::test]
async fn max_send_message_size() {
    let client = client_limits(None, Some(100));
    let (_, data, trailers) = call(&client, None, message(0, &[1; 100])).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
    assert_eq!(data, message(0, &[1; 100]));

    let (_, data, trailers) = call(&client, None, message(0, &[1; 101])).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "8");
    assert_eq!(
        trailers.get(GRPC_MESSAGE).unwrap(),
        "grpc: trying to send message larger than max (101 vs. 100)"
    );
    assert!(data.is_empty());

    // the connection still works
    let (_, _, trailers) = call(&client, None, message(0, &[1; 100])).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
}

/// Sends `parts` of a request body without ending it, returns the response
/// status and message, and whether the stream is reset.
async fn call_open(client: &SimpleClient, parts: &[&[u8]]) -> (String, String, bool) {
    let mut hdrs = HeaderMap::new();
    hdrs.insert(
        ntex_http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/grpc"),
    );
    let (snd, rcv) = client
        .send(Method::POST, "/test.Svc/Call".into(), hdrs, false)
        .await
        .unwrap();
    for part in parts {
        snd.send_payload(Bytes::copy_from_slice(part), false)
            .await
            .unwrap();
    }
    let trailers = loop {
        match rcv.recv().await.unwrap().kind {
            h2::MessageKind::Headers { .. } => {}
            h2::MessageKind::Eof(h2::StreamEof::Trailers(trailers)) => break trailers,
            kind => panic!("{kind:?}"),
        }
    };
    ntex::time::sleep(ntex::time::Millis(50)).await;
    let reset = snd
        .send_payload(Bytes::from_static(b"x"), true)
        .await
        .is_err();
    (
        trailers
            .get(GRPC_STATUS)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
        trailers
            .get(GRPC_MESSAGE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
        reset,
    )
}

#[ntex::test]
async fn early_reject() {
    let client = client_with(Some(100));
    // the length prefix is over the limit, the message is not read
    for parts in [
        &[&message(0, &[1; 101])[..5]][..],
        &[&message(0, &[1; 101])[..2], &[0, 0, 101, 1]][..],
        &[&message(0, &[1; 101])[..]][..],
    ] {
        let (status, msg, reset) = call_open(&client, parts).await;
        assert_eq!(status, "8");
        assert_eq!(msg, "grpc: received message larger than max (101 vs. 100)");
        assert!(reset);
    }

    // data after the message
    let data = [message(0, &[1; 100]), vec![0]].concat();
    for parts in [&[&data[..]][..], &[&data[..105], &data[105..]][..]] {
        let (status, msg, reset) = call_open(&client, parts).await;
        assert_eq!(status, "13");
        assert_eq!(msg, "grpc: received data after the request message");
        assert!(reset);
    }
    let (_, _, trailers) = call(&client, None, data).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "13");
    assert_eq!(
        trailers.get(GRPC_MESSAGE).unwrap(),
        "grpc: received data after the request message"
    );

    // the connection still works
    let (_, data, trailers) = call(&client, None, message(0, &[1; 100])).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
    assert_eq!(data, message(0, &[1; 100]));
}

#[ntex::test]
async fn service_error() {
    let client = client();
    let (_, data, trailers) = call_to(
        &client,
        "/test.Svc/Fail",
        req_headers(None),
        message(0, b"x"),
    )
    .await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "5");
    assert_eq!(trailers.get(GRPC_MESSAGE).unwrap(), "nothing here");
    // the headers of the error are kept
    assert_eq!(trailers.get(X_TEST).unwrap(), "error");
    assert!(data.is_empty());

    // the connection still works
    let (_, _, trailers) = call(&client, None, message(0, b"x")).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
}

#[ntex::test]
async fn response_headers() {
    let client = client();
    let mut hdrs = req_headers(None);
    hdrs.insert(X_TEST, HeaderValue::from_static("from-headers"));
    let (_, data, trailers) =
        call_to(&client, "/test.Svc/Trailers", hdrs, message(0, b"body")).await;
    assert_eq!(data, message(0, b"body"));
    // the status comes first, then the headers of the response
    assert_eq!(
        trailers.get_all(GRPC_STATUS).collect::<Vec<_>>(),
        ["0"],
        "{trailers:?}"
    );
    assert!(trailers.get(GRPC_MESSAGE).is_none());
    assert_eq!(trailers.get(X_TEST).unwrap(), "from-headers");
    assert_eq!(trailers.get(X_EXTRA).unwrap(), "1");
}

#[ntex::test]
async fn request_trailers() {
    let client = client();
    let (snd, rcv) = client
        .send(
            Method::POST,
            "/test.Svc/Trailers".into(),
            req_headers(None),
            false,
        )
        .await
        .unwrap();
    snd.send_payload(Bytes::from(message(0, b"body")), false)
        .await
        .unwrap();
    let mut req_trailers = HeaderMap::new();
    req_trailers.insert(X_TEST, HeaderValue::from_static("from-trailers"));
    snd.send_trailers(req_trailers).unwrap();

    let mut data = Vec::new();
    let trailers = loop {
        match rcv.recv().await.unwrap().kind {
            h2::MessageKind::Headers { .. } => {}
            h2::MessageKind::Data(chunk, cap) => {
                data.extend_from_slice(&chunk);
                cap.consume(chunk.len() as u32);
            }
            h2::MessageKind::Eof(h2::StreamEof::Trailers(trailers)) => break trailers,
            kind => panic!("{kind:?}"),
        }
    };
    assert_eq!(data, message(0, b"body"));
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
    // the request trailers reach the service as headers
    assert_eq!(trailers.get(X_TEST).unwrap(), "from-trailers");
}

#[ntex::test]
async fn grpc_timeout() {
    let client = client();
    // a valid timeout that does not run out
    let mut hdrs = req_headers(None);
    hdrs.insert(GRPC_TIMEOUT, HeaderValue::from_static("10S"));
    let (_, data, trailers) = call_to(&client, "/test.Svc/Call", hdrs, message(0, b"x")).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
    assert_eq!(data, message(0, b"x"));

    // an invalid timeout is rejected, the service is not called
    for val in ["abc", "1X", ""] {
        let mut hdrs = req_headers(None);
        hdrs.insert(GRPC_TIMEOUT, HeaderValue::from_str(val).unwrap());
        let (_, data, trailers) = call_to(&client, "/test.Svc/Call", hdrs, message(0, b"x")).await;
        assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "3", "{val}");
        assert_eq!(
            trailers.get(GRPC_MESSAGE).unwrap(),
            "Cannot decode grpc-timeout header"
        );
        assert!(data.is_empty());
    }

    // the connection still works
    let (_, _, trailers) = call(&client, None, message(0, b"x")).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
}

#[ntex::test]
async fn deadline_exceeded() {
    let client = client();
    let mut hdrs = req_headers(None);
    hdrs.insert(GRPC_TIMEOUT, HeaderValue::from_static("50m"));
    let (_, data, trailers) = call_to(&client, "/test.Svc/Slow", hdrs, message(0, b"x")).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "4");
    assert_eq!(trailers.get(GRPC_MESSAGE).unwrap(), "Deadline exceeded");
    assert!(data.is_empty());

    // the connection still works
    let (_, _, trailers) = call(&client, None, message(0, b"x")).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
}

#[ntex::test]
async fn boxed_io() {
    let client = client_boxed();
    let (headers, data, trailers) = call(&client, None, message(0, b"body")).await;
    assert_eq!(headers.get(GRPC_ACCEPT_ENCODING).unwrap(), ACCEPT_ENCODING);
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
    assert_eq!(data, message(0, b"body"));
}

#[ntex::test]
async fn request_reset() {
    let client = client();
    let (snd, _rcv) = client
        .send(
            Method::POST,
            "/test.Svc/Call".into(),
            req_headers(None),
            false,
        )
        .await
        .unwrap();
    snd.send_payload(Bytes::from_static(b"\0\0\0\0\x05ab"), false)
        .await
        .unwrap();
    assert!(snd.reset(ntex_h2::frame::Reason::CANCEL));

    // the request is dropped, the connection is closed
    client.on_disconnect().await;
    assert!(client.is_closed());
}

#[ntex::test]
async fn request_disconnect() {
    let client = client();
    let (_snd, rcv) = client
        .send(
            Method::POST,
            "/test.Svc/Call".into(),
            req_headers(None),
            false,
        )
        .await
        .unwrap();
    client.force_close();

    // the open stream is closed with the connection, on both sides
    match rcv.recv().await.unwrap().kind {
        h2::MessageKind::Disconnect(_) => {}
        kind => panic!("{kind:?}"),
    }
    ntex::time::sleep(ntex::time::Millis(50)).await;
}

#[ntex::test]
async fn response_after_reset() {
    let client = client();
    let (snd, _rcv) = client
        .send(
            Method::POST,
            "/test.Svc/Call".into(),
            req_headers(None),
            false,
        )
        .await
        .unwrap();
    // the whole request, then a reset before the server can answer
    snd.send_payload(Bytes::from(message(0, b"body")), true)
        .await
        .unwrap();
    assert!(snd.reset(ntex_h2::frame::Reason::CANCEL));

    // the server drops the answer, the connection still works
    let (_, data, trailers) = call(&client, None, message(0, b"x")).await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
    assert_eq!(data, message(0, b"x"));
}

#[cfg(feature = "compression")]
mod compression {
    use std::io::{Read, Write};

    use super::*;

    const ENCODINGS: [&str; 2] = ["gzip", "zstd"];

    fn compress(enc: &str, data: &[u8]) -> Vec<u8> {
        match enc {
            "gzip" => {
                let mut e =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            }
            _ => zstd::encode_all(data, 0).unwrap(),
        }
    }

    fn decompress(enc: &str, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        match enc {
            "gzip" => {
                flate2::read::GzDecoder::new(data)
                    .read_to_end(&mut out)
                    .unwrap();
            }
            _ => out = zstd::decode_all(data).unwrap(),
        }
        out
    }

    /// Pseudo-random bytes, they do not compress.
    fn random(len: usize) -> Vec<u8> {
        let mut x = 0x2545_f491_u32;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x.to_le_bytes()[0]
            })
            .collect()
    }

    #[ntex::test]
    async fn max_send_message_size() {
        let client = client_limits(None, Some(100));
        for enc in ENCODINGS {
            // the compressed response is under the limit
            let (_, data, trailers) = call(
                &client,
                Some(enc),
                message(1, &compress(enc, &[b'a'; 1000])),
            )
            .await;
            assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0", "{trailers:?}");
            assert_eq!(data[0], 1);
            assert!(data.len() <= 105);

            let input = random(1000);
            let (_, data, trailers) =
                call(&client, Some(enc), message(1, &compress(enc, &input))).await;
            assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "8", "{trailers:?}");
            assert!(
                trailers
                    .get(GRPC_MESSAGE)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("grpc: trying to send message larger than max (")
            );
            assert!(data.is_empty());
        }
    }

    #[ntex::test]
    async fn round_trip() {
        let client = client();
        // small, large when decompressed, large both ways
        let inputs = [
            vec![b'a'; 1000],
            vec![b'a'; 1024 * 1024],
            random(256 * 1024),
        ];
        for enc in ENCODINGS {
            for input in &inputs {
                let (headers, data, trailers) =
                    call(&client, Some(enc), message(1, &compress(enc, input))).await;
                assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0", "{trailers:?}");
                assert_eq!(headers.get(GRPC_ENCODING).unwrap(), enc);
                let len = u32::from_be_bytes(data[1..5].try_into().unwrap()) as usize;
                assert_eq!(len, data.len() - 5);
                if input[0] == b'a' {
                    assert_eq!(data[0], 1);
                    assert_eq!(decompress(enc, &data[5..]), *input);
                } else {
                    // it does not get smaller
                    assert_eq!(data, message(0, input));
                }
            }

            // the response is compressed if the request is not
            let input = [b'a'; 64];
            let (headers, data, trailers) = call(&client, Some(enc), message(0, &input)).await;
            assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
            assert_eq!(headers.get(GRPC_ENCODING).unwrap(), enc);
            assert_eq!(data[0], 1);
            assert_eq!(decompress(enc, &data[5..]), input);

            // a small message is not compressed
            let (headers, data, trailers) =
                call(&client, Some(enc), message(1, &compress(enc, &input[1..]))).await;
            assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
            assert_eq!(headers.get(GRPC_ENCODING).unwrap(), enc);
            assert_eq!(data, message(0, &input[1..]));

            // an empty message is not compressed
            let (headers, data, trailers) =
                call(&client, Some(enc), message(1, &compress(enc, b""))).await;
            assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
            assert_eq!(headers.get(GRPC_ENCODING).unwrap(), enc);
            assert_eq!(data, b"\0\0\0\0\0");
        }

        // nor without an encoding
        let (headers, data, _) = call(&client, None, message(0, b"abc")).await;
        assert!(headers.get(GRPC_ENCODING).is_none());
        assert_eq!(data, message(0, b"abc"));
        let (headers, data, _) = call(&client, Some("identity"), message(0, b"abc")).await;
        assert!(headers.get(GRPC_ENCODING).is_none());
        assert_eq!(data, message(0, b"abc"));
    }

    #[ntex::test]
    async fn accept_encoding() {
        let client = client();
        let input = [b'a'; 1000];
        for enc in ENCODINGS {
            for (accept, compressed) in [
                ("identity", false),
                ("identity, gzip, zstd", true),
                (enc, true),
            ] {
                let mut hdrs = req_headers(Some(enc));
                hdrs.insert(GRPC_ACCEPT_ENCODING, HeaderValue::from_static(accept));
                let (headers, data, trailers) =
                    call_to(&client, "/test.Svc/Call", hdrs, message(0, &input)).await;
                assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0", "{trailers:?}");
                if compressed {
                    assert_eq!(headers.get(GRPC_ENCODING).unwrap(), enc);
                    assert_eq!(data[0], 1);
                    assert_eq!(decompress(enc, &data[5..]), input);
                } else {
                    assert!(headers.get(GRPC_ENCODING).is_none(), "{accept}");
                    assert_eq!(data, message(0, &input));
                }
            }
        }
    }

    #[ntex::test]
    async fn limit() {
        let client = client_with(Some(100));
        for enc in ENCODINGS {
            let (_, _, trailers) =
                call(&client, Some(enc), message(1, &compress(enc, &[1; 100]))).await;
            assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");

            let (_, data, trailers) =
                call(&client, Some(enc), message(1, &compress(enc, &[1; 101]))).await;
            assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "8");
            assert_eq!(
                trailers.get(GRPC_MESSAGE).unwrap(),
                "grpc: received message after decompression larger than max 100"
            );
            assert!(data.is_empty());

            // larger than the inline limit
            let large = client_with(Some(1024 * 1024));
            let input = vec![1; 1024 * 1024 + 1];
            let (_, _, trailers) =
                call(&large, Some(enc), message(1, &compress(enc, &input))).await;
            assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "8");
            assert_eq!(
                trailers.get(GRPC_MESSAGE).unwrap(),
                "grpc: received message after decompression larger than max 1048576"
            );
        }
    }

    #[ntex::test]
    async fn invalid() {
        let client = client();
        for enc in ENCODINGS {
            let (_, _, trailers) = call(&client, Some(enc), message(1, b"abc")).await;
            assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "13");
            let msg = trailers.get(GRPC_MESSAGE).unwrap().to_str().unwrap();
            assert!(
                msg.starts_with("grpc: failed to read decompressed data: "),
                "{msg}"
            );
        }
        // the connection still works
        let (_, _, trailers) = call(&client, Some("gzip"), message(0, b"abc")).await;
        assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
    }
}
