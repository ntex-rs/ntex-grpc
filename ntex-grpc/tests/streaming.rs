use std::{cell::Cell, rc::Rc, time::Duration};

use ntex::io::Io;
use ntex::service::{Pipeline, ServiceFactory, cfg::SharedCfg};
use ntex::testing::IoTest;
use ntex::util::join;
use ntex_bytes::{ByteString, Bytes};
use ntex_grpc::client::{Cause, ClientError, Request, StreamRequest};
use ntex_grpc::google_types::{StringValue, UInt32Value};
use ntex_grpc::server::{GrpcServer, ServerError, ServerRequest};
use ntex_grpc::{GrpcStatus, MethodDef, RecvStream, SendStream, ServiceDef, StreamError};
use ntex_h2::{self as h2, client::SimpleClient};
use ntex_http::{HeaderMap, HeaderName, HeaderValue, Method};

/// The error of the call on the client side.
fn client_error(err: &StreamError) -> &ClientError {
    match err {
        StreamError::Client(e) => e,
        e => panic!("not a client error: {e:?}"),
    }
}

const GRPC_STATUS: HeaderName = HeaderName::from_static("grpc-status");
const X_TEST: HeaderName = HeaderName::from_static("x-test");
const X_COUNT: HeaderName = HeaderName::from_static("x-count");

macro_rules! method {
    ($name:ident, $path:literal, $in:ty, $out:ty, $client:literal, $server:literal) => {
        #[derive(Debug, Clone, Copy)]
        struct $name;

        impl MethodDef for $name {
            const NAME: &'static str = stringify!($name);
            const PATH: ByteString = ByteString::from_static($path);
            const CLIENT_STREAMING: bool = $client;
            const SERVER_STREAMING: bool = $server;
            type Input = $in;
            type Output = $out;
        }
    };
}

method!(
    Echo,
    "/test.Stream/Echo",
    StringValue,
    StringValue,
    true,
    true
);
method!(
    Count,
    "/test.Stream/Count",
    UInt32Value,
    UInt32Value,
    false,
    true
);
method!(
    Sum,
    "/test.Stream/Sum",
    UInt32Value,
    UInt32Value,
    true,
    false
);
method!(
    Unary,
    "/test.Stream/Unary",
    UInt32Value,
    UInt32Value,
    false,
    false
);
method!(
    Missing,
    "/test.Stream/Missing",
    UInt32Value,
    UInt32Value,
    true,
    true
);

#[derive(Debug, Clone, Copy)]
struct Stream;

#[derive(Debug, Clone, Copy)]
enum StreamMethods {
    Echo(Echo),
    Count(Count),
    Sum(Sum),
    Unary(Unary),
    Missing(Missing),
}

impl ServiceDef for Stream {
    const NAME: &'static str = "test.Stream";
    type Methods = StreamMethods;

    fn method_by_name(name: &str) -> Option<StreamMethods> {
        match name {
            Echo::NAME => Some(StreamMethods::Echo(Echo)),
            Count::NAME => Some(StreamMethods::Count(Count)),
            Sum::NAME => Some(StreamMethods::Sum(Sum)),
            Unary::NAME => Some(StreamMethods::Unary(Unary)),
            Missing::NAME => Some(StreamMethods::Missing(Missing)),
            _ => None,
        }
    }
}

/// Counts the handlers that are running.
#[derive(Clone, Default)]
struct Svc(Rc<Cell<usize>>);

impl ServiceFactory<(), ServerRequest> for Svc {
    type Res = ntex_grpc::server::ServerResponse;
    type Error = ServerError;
    type Service = Svc;
    type InitError = std::io::Error;

    async fn create(&self, (): &()) -> Result<Svc, std::io::Error> {
        Ok(self.clone())
    }
}

/// Decrements the count of running handlers on drop.
struct Running(Rc<Cell<usize>>);

impl Running {
    fn new(svc: &Svc) -> Self {
        svc.0.set(svc.0.get() + 1);
        Running(svc.0.clone())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

/// `Svc` added under another name.
struct Renamed(Svc);

impl ntex_grpc::server::NamedService for Renamed {
    const NAME: &'static str = "test.Renamed";
}

impl ServiceFactory<(), ServerRequest> for Renamed {
    type Res = ntex_grpc::server::ServerResponse;
    type Error = ServerError;
    type Service = Svc;
    type InitError = std::io::Error;

    async fn create(&self, (): &()) -> Result<Svc, std::io::Error> {
        Ok(self.0.clone())
    }
}

#[ntex_grpc::server(Stream)]
impl Svc {
    /// Echoes every message, `fail` ends the call with `NotFound`, `wait`
    /// waits forever.
    #[method(Echo)]
    async fn echo(
        &self,
        mut rx: RecvStream<StringValue>,
        tx: SendStream<StringValue>,
    ) -> Result<(), ServerError> {
        let _running = Running::new(self);
        if let Some(val) = rx.headers().get(X_TEST) {
            tx.header(X_TEST, val.clone());
        }

        let mut count = 0;
        while let Some(msg) = rx.recv().await {
            let msg = msg?;
            match msg.value.as_ref() {
                "fail" => {
                    return Err(ServerError::new(
                        GrpcStatus::NotFound,
                        HeaderValue::from_static("failed"),
                        None,
                    ));
                }
                "wait" => std::future::pending::<()>().await,
                "cancel" => {
                    tx.cancel();
                    return Ok(());
                }
                _ => {}
            }
            count += 1;
            tx.send(&msg).await?;
        }
        tx.trailer(X_COUNT, HeaderValue::from(count));
        Ok(())
    }

    /// Sends `value` messages, `0..value`, then waits forever if `value` is
    /// larger than 100.
    #[method(Count)]
    async fn count(&self, req: UInt32Value, tx: SendStream<UInt32Value>) {
        let _running = Running::new(self);
        for value in 0..req.value.min(100) {
            if tx.send(&UInt32Value { value }).await.is_err() {
                return;
            }
        }
        if req.value > 100 {
            std::future::pending::<()>().await;
        }
    }

    /// Replies with the sum of the request messages.
    #[method(Sum)]
    async fn sum(&self, mut rx: RecvStream<UInt32Value>) -> UInt32Value {
        let mut value = 0;
        while let Some(Ok(msg)) = rx.recv().await {
            value += msg.value;
        }
        UInt32Value { value }
    }

    #[method(Unary)]
    async fn unary(&self, req: UInt32Value) -> UInt32Value {
        UInt32Value {
            value: req.value + 1,
        }
    }
}

fn start(svc: Svc, server: impl FnOnce(GrpcServer) -> GrpcServer) -> SimpleClient {
    start_server(server(GrpcServer::new(svc)))
}

fn start_server(server: GrpcServer) -> SimpleClient {
    start_server_with_cfg(server, SharedCfg::new("CLI").build())
}

fn start_server_with_cfg(server: GrpcServer, cfg: SharedCfg) -> SimpleClient {
    let (cli, srv) = IoTest::create();
    cli.remote_buffer_cap(64 * 1024 * 1024);
    srv.remote_buffer_cap(64 * 1024 * 1024);

    let srv = Io::new(srv, SharedCfg::new("SRV").build());
    ntex::rt::spawn(async move {
        let _ = Pipeline::new((), server).call(srv).await;
    });

    let io = Io::new(cli, cfg);
    SimpleClient::new(io, false, "localhost".into())
}

fn client() -> SimpleClient {
    start(Svc::default(), |s| s)
}

fn string(value: &'static str) -> StringValue {
    StringValue {
        value: ByteString::from_static(value),
    }
}

/// Waits until no handler is running.
async fn wait_idle(svc: &Svc) {
    for _ in 0..100 {
        if svc.0.get() == 0 {
            return;
        }
        ntex::time::sleep(ntex::time::Millis(10)).await;
    }
    panic!("handler is still running");
}

#[ntex::test]
async fn bidi() {
    let client = client();
    let mut req = StreamRequest::<_, Echo, _>::bidi(&client);
    req.header(X_TEST, HeaderValue::from_static("hdr"));
    let (tx, mut rx) = req.open().await.unwrap();

    // replies come while the request stream is open
    for value in ["a", "b", "c"] {
        tx.send(&string(value)).await.unwrap();
        assert_eq!(rx.recv().await.unwrap().unwrap(), string(value));
    }
    assert_eq!(rx.headers().get(X_TEST).unwrap(), "hdr");
    tx.close();

    assert!(rx.recv().await.is_none());
    assert_eq!(rx.status(), Some(GrpcStatus::Ok));
    assert_eq!(rx.trailers().get(X_COUNT).unwrap(), "3");
}

#[ntex::test]
async fn bidi_empty() {
    let client = client();
    let (tx, mut rx) = StreamRequest::<_, Echo, _>::bidi(&client)
        .open()
        .await
        .unwrap();
    drop(tx);

    assert!(rx.recv().await.is_none());
    assert_eq!(rx.status(), Some(GrpcStatus::Ok));
    assert_eq!(rx.trailers().get(X_COUNT).unwrap(), "0");
}

#[ntex::test]
async fn bidi_error() {
    let client = client();
    let (tx, mut rx) = StreamRequest::<_, Echo, _>::bidi(&client)
        .open()
        .await
        .unwrap();
    tx.send(&string("a")).await.unwrap();
    tx.send(&string("fail")).await.unwrap();

    assert_eq!(rx.recv().await.unwrap().unwrap(), string("a"));
    let err = rx.recv().await.unwrap().unwrap_err();
    assert_eq!(err.status(), GrpcStatus::NotFound);
    assert!(
        matches!(client_error(&err), ClientError::Status(_)),
        "{err:?}"
    );
    assert!(rx.recv().await.is_none());
    assert_eq!(rx.status(), Some(GrpcStatus::NotFound));
}

async fn check_response_headers_failure(send_message: bool) {
    let svc = Svc::default();
    let cfg = SharedCfg::new("CLI")
        .add(h2::ServiceConfig::new().set_max_header_list_size(1024))
        .build();
    let client = start_server_with_cfg(GrpcServer::new(svc.clone()), cfg);
    let mut req = StreamRequest::<_, Echo, _>::bidi(&client);
    // Accepted as request metadata, but too large to echo to this client.
    req.header(X_TEST, HeaderValue::try_from("a".repeat(2048)).unwrap());
    let (tx, mut rx) = req.open().await.unwrap();
    if send_message {
        tx.send(&string("a")).await.unwrap();
    } else {
        tx.close();
    }

    ntex::time::timeout(Duration::from_secs(1), async {
        let err = rx.recv().await.unwrap().unwrap_err();
        let ClientError::StreamFailed(failure) = client_error(&err) else {
            panic!("{err:?}");
        };
        assert_eq!(failure.code, GrpcStatus::Internal);
        assert!(
            matches!(
                &failure.cause,
                Some(Cause::Stream(h2::StreamError::Reset(
                    h2::frame::Reason::INTERNAL_ERROR
                )))
            ),
            "{err:?}"
        );
        assert_eq!(rx.status(), Some(GrpcStatus::Internal));
        assert!(rx.headers().is_empty());
        assert!(rx.recv().await.is_none());
        wait_idle(&svc).await;

        let res = Request::<_, Unary>::new(&client, &UInt32Value { value: 1 })
            .send()
            .await
            .unwrap();
        assert_eq!(res.output, UInt32Value { value: 2 });
    })
    .await
    .expect("response-header failure did not terminate the RPC or left the connection stalled");
}

#[ntex::test]
async fn streaming_response_headers_failure() {
    check_response_headers_failure(true).await;
}

#[ntex::test]
async fn empty_streaming_response_headers_failure() {
    check_response_headers_failure(false).await;
}

#[ntex::test]
async fn bidi_cancel() {
    let svc = Svc::default();
    let client = start(svc.clone(), |s| s);
    let (tx, mut rx) = StreamRequest::<_, Echo, _>::bidi(&client)
        .open()
        .await
        .unwrap();
    tx.send(&string("a")).await.unwrap();
    assert_eq!(rx.recv().await.unwrap().unwrap(), string("a"));
    tx.send(&string("wait")).await.unwrap();
    assert_eq!(svc.0.get(), 1);

    // dropping the response stream resets the call, the handler is dropped
    drop(rx);
    wait_idle(&svc).await;
    assert!(matches!(
        tx.send(&string("b")).await,
        Err(StreamError::Closed | StreamError::Reset(_))
    ));
}

#[ntex::test]
async fn bidi_deadline() {
    let svc = Svc::default();
    let client = start(svc.clone(), |s| s);
    let mut req = StreamRequest::<_, Echo, _>::bidi(&client);
    req.timeout(Duration::from_millis(100));
    let (tx, mut rx) = req.open().await.unwrap();
    tx.send(&string("wait")).await.unwrap();

    let err = rx.recv().await.unwrap().unwrap_err();
    assert_eq!(err.status(), GrpcStatus::DeadlineExceeded);
    assert!(
        matches!(client_error(&err), ClientError::DeadlineExceeded(_)),
        "{err:?}"
    );
    wait_idle(&svc).await;
}

#[ntex::test]
async fn bidi_message_size() {
    let client = start(Svc::default(), |s| s.max_message_size(16));
    let (tx, mut rx) = StreamRequest::<_, Echo, _>::bidi(&client)
        .open()
        .await
        .unwrap();
    tx.send(&string("small")).await.unwrap();
    assert_eq!(rx.recv().await.unwrap().unwrap(), string("small"));
    let _ = tx.send(&string("a message larger than the limit")).await;

    let err = rx.recv().await.unwrap().unwrap_err();
    assert_eq!(err.status(), GrpcStatus::ResourceExhausted);
}

#[cfg(feature = "compression")]
#[ntex::test]
async fn bidi_compression() {
    let client = client();
    let mut req = StreamRequest::<_, Echo, _>::bidi(&client);
    req.compression(ntex_grpc::Compression::Gzip);
    let (tx, mut rx) = req.open().await.unwrap();

    for value in ["a", "b"] {
        tx.send(&string(value)).await.unwrap();
        assert_eq!(rx.recv().await.unwrap().unwrap(), string(value));
    }
    tx.close();
    assert!(rx.recv().await.is_none());
    assert_eq!(rx.status(), Some(GrpcStatus::Ok));
}

#[ntex::test]
async fn server_streaming() {
    let client = client();
    let req = UInt32Value { value: 5 };
    let mut rx = StreamRequest::<_, Count, _>::server_streaming(&client, &req)
        .open()
        .await
        .unwrap();

    for value in 0..5 {
        assert_eq!(rx.recv().await.unwrap().unwrap(), UInt32Value { value });
    }
    assert!(rx.recv().await.is_none());
    assert_eq!(rx.status(), Some(GrpcStatus::Ok));

    // no messages, the response headers are sent with the trailers
    let req = UInt32Value { value: 0 };
    let mut rx = StreamRequest::<_, Count, _>::server_streaming(&client, &req)
        .open()
        .await
        .unwrap();
    assert!(rx.recv().await.is_none());
    assert_eq!(rx.status(), Some(GrpcStatus::Ok));
}

#[ntex::test]
async fn server_streaming_cancel() {
    let svc = Svc::default();
    let client = start(svc.clone(), |s| s);
    let req = UInt32Value { value: 1000 };
    let mut rx = StreamRequest::<_, Count, _>::server_streaming(&client, &req)
        .open()
        .await
        .unwrap();
    assert_eq!(rx.recv().await.unwrap().unwrap(), UInt32Value { value: 0 });
    assert_eq!(svc.0.get(), 1);

    drop(rx);
    wait_idle(&svc).await;
}

#[ntex::test]
async fn server_streaming_server_deadline() {
    let svc = Svc::default();
    let client = start(svc.clone(), |s| s);
    let mut hdrs = req_headers();
    hdrs.insert(
        HeaderName::from_static("grpc-timeout"),
        HeaderValue::from_static("50m"),
    );
    // 1000 = [0x08, 0xe8, 0x07]
    let (_, data, trailers) = call(
        &client,
        "/test.Stream/Count",
        hdrs,
        &b"\0\0\0\0\x03\x08\xe8\x07"[..],
    )
    .await;
    assert_eq!(data.len(), 5 + 99 * 7);
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "4");
    wait_idle(&svc).await;
}

#[ntex::test]
async fn zero_server_timeout() {
    let svc = Svc::default();
    let client = start(svc.clone(), |s| s);
    ntex::time::timeout(Duration::from_secs(1), async {
        for val in ["0H", "0M", "0S", "0m", "0u", "0n"] {
            for eof in [true, false] {
                let mut hdrs = req_headers();
                hdrs.insert(
                    HeaderName::from_static("grpc-timeout"),
                    HeaderValue::from_static(val),
                );
                // Echo sets this response header as soon as it is called.
                hdrs.insert(X_TEST, HeaderValue::from_static("handler ran"));
                let (snd, rcv) = client
                    .send(Method::POST, Echo::PATH, hdrs, eof)
                    .await
                    .unwrap();
                let (headers, data, trailers) = read(&rcv).await;
                assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "4", "{val}, {eof}");
                assert_eq!(trailers.get("grpc-message").unwrap(), "Deadline exceeded");
                assert!(data.is_empty());
                assert!(!headers.contains_key(X_TEST));
                assert!(!trailers.contains_key(X_TEST));
                assert!(!trailers.contains_key(X_COUNT));
                assert_eq!(svc.0.get(), 0);
                if !eof {
                    while !snd.is_reset() {
                        snd.on_reset().await;
                    }
                }
            }
        }

        let res = Request::<_, Unary>::new(&client, &UInt32Value { value: 1 })
            .send()
            .await
            .unwrap();
        assert_eq!(res.output, UInt32Value { value: 2 });
    })
    .await
    .expect("zero server timeout did not terminate the RPC");
}

#[ntex::test]
async fn submillisecond_server_timeout() {
    let svc = Svc::default();
    let client = start(svc.clone(), |s| s);
    ntex::time::timeout(Duration::from_secs(1), async {
        for val in ["1u", "999u", "1n", "999999n"] {
            let mut hdrs = req_headers();
            hdrs.insert(
                HeaderName::from_static("grpc-timeout"),
                HeaderValue::from_static(val),
            );
            let (snd, rcv) = client
                .send(Method::POST, Echo::PATH, hdrs, false)
                .await
                .unwrap();
            let (_, data, trailers) = read(&rcv).await;
            assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "4", "{val}");
            assert_eq!(trailers.get("grpc-message").unwrap(), "Deadline exceeded");
            assert!(data.is_empty());
            assert_eq!(svc.0.get(), 0);
            while !snd.is_reset() {
                snd.on_reset().await;
            }
        }

        let res = Request::<_, Unary>::new(&client, &UInt32Value { value: 1 })
            .send()
            .await
            .unwrap();
        assert_eq!(res.output, UInt32Value { value: 2 });
    })
    .await
    .expect("submillisecond server timeout became unlimited");
}

#[ntex::test]
async fn server_streaming_cardinality() {
    let client = client();
    // no request message
    let (_, data, trailers) =
        call(&client, "/test.Stream/Count", req_headers(), Bytes::new()).await;
    assert!(data.is_empty());
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "3");

    // two request messages
    let (_, data, trailers) = call(
        &client,
        "/test.Stream/Count",
        req_headers(),
        &b"\0\0\0\0\x02\x08\x01\0\0\0\0\x02\x08\x01"[..],
    )
    .await;
    assert!(data.is_empty());
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "13");

    // the request split across frames
    let (snd, rcv) = client
        .send(
            Method::POST,
            "/test.Stream/Count".into(),
            req_headers(),
            false,
        )
        .await
        .unwrap();
    for chunk in [&b"\0\0\0"[..], b"\0\x02\x08", b"\x02"] {
        snd.send_payload(Bytes::from_static(chunk), false)
            .await
            .unwrap();
    }
    snd.send_payload(Bytes::new(), true).await.unwrap();
    let (_, data, trailers) = read(&rcv).await;
    assert_eq!(data, b"\0\0\0\0\0\0\0\0\0\x02\x08\x01");
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
}

#[ntex::test]
async fn client_streaming() {
    let client = client();
    let tx = SendStream::new();
    let (res, sent) = join(
        StreamRequest::<_, Sum, _>::client_streaming(&client, &tx).send(),
        async {
            for value in 1..=4 {
                tx.send(&UInt32Value { value }).await?;
            }
            tx.close();
            Ok::<_, StreamError>(())
        },
    )
    .await;
    sent.unwrap();
    assert_eq!(res.unwrap().output, UInt32Value { value: 10 });

    // no messages
    let tx = SendStream::new();
    tx.close();
    let res = StreamRequest::<_, Sum, _>::client_streaming(&client, &tx)
        .send()
        .await
        .unwrap();
    assert_eq!(res.output, UInt32Value { value: 0 });
}

#[ntex::test]
async fn unary_and_missing() {
    let client = client();
    let res = Request::<_, Unary>::new(&client, &UInt32Value { value: 1 })
        .send()
        .await
        .unwrap();
    assert_eq!(res.output, UInt32Value { value: 2 });

    // a streaming method without a handler
    let (_, _, trailers) = call(
        &client,
        "/test.Stream/Missing",
        req_headers(),
        &b"\0\0\0\0\0"[..],
    )
    .await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "12");
}

#[ntex::test]
async fn bidi_as_stream() {
    let client = client();
    let (tx, mut rx) = StreamRequest::<_, Echo, _>::bidi(&client)
        .open()
        .await
        .unwrap();
    tx.send(&string("a")).await.unwrap();
    tx.close();

    let msg = ntex::util::stream_recv(&mut rx).await.unwrap().unwrap();
    assert_eq!(msg, string("a"));
    assert!(ntex::util::stream_recv(&mut rx).await.is_none());
    assert_eq!(rx.status(), Some(GrpcStatus::Ok));
}

#[ntex::test]
async fn bidi_client_cancel() {
    let svc = Svc::default();
    let client = start(svc.clone(), |s| s);
    let (tx, mut rx) = StreamRequest::<_, Echo, _>::bidi(&client)
        .open()
        .await
        .unwrap();
    tx.send(&string("a")).await.unwrap();
    assert_eq!(rx.recv().await.unwrap().unwrap(), string("a"));
    assert_eq!(svc.0.get(), 1);

    // the call is reset, the handler sees the request stream fail
    tx.cancel();
    assert!(tx.is_closed());
    assert!(matches!(
        tx.send(&string("b")).await,
        Err(StreamError::Closed)
    ));
    wait_idle(&svc).await;
    let err = rx.recv().await.unwrap().unwrap_err();
    assert!(
        matches!(client_error(&err), ClientError::StreamFailed(f) if f.code == GrpcStatus::Cancelled),
        "{err:?}"
    );
    assert!(rx.recv().await.is_none());
}

#[ntex::test]
async fn client_streaming_cancel() {
    let svc = Svc::default();
    let client = start(svc.clone(), |s| s);
    let tx = SendStream::new();
    let (res, ()) = join(
        StreamRequest::<_, Sum, _>::client_streaming(&client, &tx).send(),
        async {
            tx.send(&UInt32Value { value: 1 }).await.unwrap();
            tx.cancel();
        },
    )
    .await;
    let err = res.unwrap_err();
    assert!(
        matches!(&*err, ClientError::StreamFailed(f) if f.code == GrpcStatus::Cancelled),
        "{err:?}"
    );
}

#[ntex::test]
async fn bidi_server_cancel() {
    let svc = Svc::default();
    let client = start(svc.clone(), |s| s);
    let (tx, mut rx) = StreamRequest::<_, Echo, _>::bidi(&client)
        .open()
        .await
        .unwrap();
    tx.send(&string("a")).await.unwrap();
    assert_eq!(rx.recv().await.unwrap().unwrap(), string("a"));
    tx.send(&string("cancel")).await.unwrap();

    // the handler resets the call instead of ending it with a status
    let err = rx.recv().await.unwrap().unwrap_err();
    assert_eq!(err.status(), GrpcStatus::Cancelled, "{err:?}");
    assert!(rx.recv().await.is_none());
    wait_idle(&svc).await;
}

#[ntex::test]
async fn bidi_invalid_request() {
    let client = client();
    // the message does not decode
    let (_, data, trailers) = call(
        &client,
        "/test.Stream/Echo",
        req_headers(),
        &b"\0\0\0\0\x02\x0a\x05"[..],
    )
    .await;
    assert!(data.is_empty());
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "3");

    // the request ends inside a message
    let (_, data, trailers) = call(
        &client,
        "/test.Stream/Echo",
        req_headers(),
        &b"\0\0\0\0\x03\x0a\x01a\0\0"[..],
    )
    .await;
    assert_eq!(data, b"\0\0\0\0\x03\x0a\x01a");
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "3");

    // a server streaming request ends inside its second message
    let (_, data, trailers) = call(
        &client,
        "/test.Stream/Count",
        req_headers(),
        &b"\0\0\0\0\x02\x08\x01\0\0"[..],
    )
    .await;
    assert!(data.is_empty());
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "3");
}

#[ntex::test]
async fn server_send_message_size() {
    let svc = Svc::default();
    let client = start(svc.clone(), |s| s.max_send_message_size(4));
    let (tx, mut rx) = StreamRequest::<_, Echo, _>::bidi(&client)
        .open()
        .await
        .unwrap();
    tx.send(&string("a")).await.unwrap();
    assert_eq!(rx.recv().await.unwrap().unwrap(), string("a"));
    tx.send(&string("larger")).await.unwrap();

    let err = rx.recv().await.unwrap().unwrap_err();
    assert_eq!(err.status(), GrpcStatus::ResourceExhausted, "{err:?}");
    assert!(rx.recv().await.is_none());
    wait_idle(&svc).await;
}

#[ntex::test]
async fn streaming_service_named() {
    // `service_named()` passes every call as a unary one
    let client = start_server(
        GrpcServer::new(Renamed(Svc::default())).service_named("test.Stream", Svc::default()),
    );
    let (_, _, trailers) = call(
        &client,
        "/test.Stream/Echo",
        req_headers(),
        &b"\0\0\0\0\0"[..],
    )
    .await;
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "12");
    assert_eq!(
        trailers.get("grpc-message").unwrap(),
        "Streaming is not supported for method: Echo"
    );

    // unary methods work
    let (_, data, trailers) = call(
        &client,
        "/test.Stream/Unary",
        req_headers(),
        &b"\0\0\0\0\x02\x08\x01"[..],
    )
    .await;
    assert_eq!(data, b"\0\0\0\0\x02\x08\x02");
    assert_eq!(trailers.get(GRPC_STATUS).unwrap(), "0");
}

/// Request headers of a call.
fn req_headers() -> HeaderMap {
    let mut hdrs = HeaderMap::new();
    hdrs.insert(
        ntex_http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/grpc"),
    );
    hdrs
}

/// Sends a request body, returns the response headers, body and trailers.
async fn call(
    client: &SimpleClient,
    path: &'static str,
    hdrs: HeaderMap,
    body: impl Into<Bytes>,
) -> (HeaderMap, Vec<u8>, HeaderMap) {
    let (snd, rcv) = client
        .send(Method::POST, path.into(), hdrs, false)
        .await
        .unwrap();
    let _ = snd.send_payload(body.into(), true).await;
    read(&rcv).await
}

async fn read(rcv: &h2::client::RecvStream) -> (HeaderMap, Vec<u8>, HeaderMap) {
    let mut headers = HeaderMap::new();
    let mut data = Vec::new();
    loop {
        let msg = rcv.recv().await.unwrap();
        match msg.kind {
            h2::MessageKind::Headers {
                headers: h, eof, ..
            } => {
                if eof {
                    return (HeaderMap::new(), data, h);
                }
                headers = h;
            }
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
