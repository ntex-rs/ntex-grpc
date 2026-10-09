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
    let (cli, srv) = IoTest::create();
    cli.remote_buffer_cap(64 * 1024 * 1024);
    srv.remote_buffer_cap(64 * 1024 * 1024);

    let server = GrpcServer::new(fn_factory(async |&()| {
        Ok::<_, std::io::Error>(fn_service(async |req: ServerRequest| {
            let mut payload = BytePages::default();
            payload.append(req.payload);
            Ok::<_, ServerError>(ServerResponse::new(payload))
        }))
    }));
    let server = match max_size {
        Some(size) => server.max_message_size(size),
        None => server,
    };
    let srv = Io::new(srv, SharedCfg::new("SRV").build());
    ntex::rt::spawn(async move {
        let _ = Pipeline::new((), server).call(srv).await;
    });

    let io = Io::new(cli, SharedCfg::new("CLI").build());
    SimpleClient::new(io, false, "localhost".into())
}

/// Sends a request body, returns the response headers, body and trailers.
async fn call(
    client: &SimpleClient,
    encoding: Option<&'static str>,
    body: impl Into<Bytes>,
) -> (HeaderMap, Vec<u8>, HeaderMap) {
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
                assert_eq!(data[0], 1);
                let len = u32::from_be_bytes(data[1..5].try_into().unwrap()) as usize;
                assert_eq!(len, data.len() - 5);
                assert_eq!(decompress(enc, &data[5..]), *input);
            }

            // the response is compressed if the request is not
            let (headers, data, trailers) = call(&client, Some(enc), message(0, b"abc")).await;
            assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
            assert_eq!(headers.get(GRPC_ENCODING).unwrap(), enc);
            assert_eq!(data[0], 1);
            assert_eq!(decompress(enc, &data[5..]), b"abc");

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
