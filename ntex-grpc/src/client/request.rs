use std::{convert::TryFrom, fmt, ops, time};

use ntex_http::{HeaderMap, HeaderName, HeaderValue, error::Error as HttpError};

#[cfg(feature = "compression")]
use crate::Compression;
use crate::{client::Transport, consts, service::MethodDef};

/// Headers, timeout and flags of a single call.
///
/// [`Request`] collects them and passes them to
/// [`Transport::request()`]. A custom transport that wraps a built-in one
/// passes the context on unchanged.
#[derive(Debug)]
pub struct RequestContext {
    err: Option<HttpError>,
    headers: HeaderMap,
    timeout: Option<time::Duration>,
    max_message_size: usize,
    max_send_message_size: usize,
    #[cfg(feature = "compression")]
    compression: Option<Compression>,
    flags: Flags,
}

/// Default limit of a received message.
const DEFAULT_MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

/// Default limit of a sent message.
const DEFAULT_MAX_SEND_MESSAGE_SIZE: usize = i32::MAX as usize;

bitflags::bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    struct Flags: u8 {
        const DISCONNECT_ON_DROP = 0b0000_0001;
    }
}

impl RequestContext {
    /// Create new `RequestContext` instance
    fn new() -> Self {
        Self {
            err: None,
            headers: HeaderMap::new(),
            timeout: None,
            max_message_size: DEFAULT_MAX_MESSAGE_SIZE,
            max_send_message_size: DEFAULT_MAX_SEND_MESSAGE_SIZE,
            #[cfg(feature = "compression")]
            compression: None,
            flags: Flags::empty(),
        }
    }

    /// Get request timeout
    pub fn get_timeout(&self) -> Option<time::Duration> {
        self.timeout
    }

    /// Set the max duration the request is allowed to take.
    ///
    /// The duration is sent in the `grpc-timeout` header, formatted according
    /// to [the spec] with the most precise unit that fits, rounded up and at
    /// most `99999999H`. Built-in transports also stop waiting when it runs
    /// out and return [`ClientError::DeadlineExceeded`](super::ClientError::DeadlineExceeded).
    /// A zero timeout fails the call without sending it.
    ///
    /// [the spec]: https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
    pub fn timeout<U>(&mut self, timeout: U) -> &mut Self
    where
        time::Duration: From<U>,
    {
        let to = timeout.into();
        self.timeout = Some(to);
        // always valid, the value holds only digits and a unit
        if let Ok(val) = HeaderValue::try_from(duration_to_grpc_timeout(to)) {
            self.headers.insert(consts::GRPC_TIMEOUT, val);
        }
        self
    }

    /// Get the size limit of the response message.
    pub fn get_max_message_size(&self) -> usize {
        self.max_message_size
    }

    /// Set the size limit of the response message, 4 MiB by default.
    ///
    /// Built-in transports check the length the server declares for the
    /// message. If it is over the limit, they reset the stream instead of
    /// reading the message and return
    /// [`GrpcStatus::ResourceExhausted`](crate::GrpcStatus::ResourceExhausted).
    /// A compressed message must fit the limit after decompression too.
    pub fn max_message_size(&mut self, size: usize) -> &mut Self {
        self.max_message_size = size;
        self
    }

    /// Get the size limit of the request message.
    pub fn get_max_send_message_size(&self) -> usize {
        self.max_send_message_size
    }

    /// Set the size limit of the request message, 2 GiB - 1 by default.
    ///
    /// Built-in transports do not send a larger message and return
    /// [`GrpcStatus::ResourceExhausted`](crate::GrpcStatus::ResourceExhausted).
    /// A message is never sent if it is 4 GiB or larger, its length does
    /// not fit the length prefix. A compressed message is checked after
    /// compression.
    pub fn max_send_message_size(&mut self, size: usize) -> &mut Self {
        self.max_send_message_size = size;
        self
    }

    /// Get the compression of the request message.
    ///
    /// Requires the `compression` feature.
    #[cfg(feature = "compression")]
    pub fn get_compression(&self) -> Option<Compression> {
        self.compression
    }

    /// Compress the request message, it is not compressed by default.
    ///
    /// Built-in transports send the encoding in the `grpc-encoding` header.
    /// An empty message is sent uncompressed. A server that does not support
    /// the encoding fails the call with
    /// [`GrpcStatus::Unimplemented`](crate::GrpcStatus::Unimplemented).
    ///
    /// Requires the `compression` feature.
    #[cfg(feature = "compression")]
    pub fn compression(&mut self, compression: Compression) -> &mut Self {
        self.compression = Some(compression);
        self
    }

    /// Disconnect connection on request drop
    pub fn disconnect_on_drop(&mut self) -> &mut Self {
        self.flags.insert(Flags::DISCONNECT_ON_DROP);
        self
    }

    /// Set a request header, replacing any existing value for the same name.
    ///
    /// An invalid name or value is not sent; the error is kept and returned
    /// by [`take_error()`](Self::take_error). Values of `-bin` headers are
    /// sent as they are, encode them with
    /// [`encode_binary_header()`](crate::encode_binary_header).
    ///
    /// Headers the client sets itself are ignored:
    /// `content-type`, `user-agent`, `te`, `grpc-encoding`,
    /// `grpc-message-type`, `grpc-message`, `grpc-status` and `grpc-timeout`.
    /// Use [`timeout()`](Self::timeout) for the timeout.
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: TryFrom<K>,
        HeaderValue: TryFrom<V>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        <HeaderValue as TryFrom<V>>::Error: Into<HttpError>,
    {
        if let Some((key, value)) = self.try_header(key, value) {
            self.headers.insert(key, value);
        }
        self
    }

    /// Add a request header, keeping existing values for the same name.
    ///
    /// Errors are handled as in [`header()`](Self::header).
    pub fn append_header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: TryFrom<K>,
        HeaderValue: TryFrom<V>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        <HeaderValue as TryFrom<V>>::Error: Into<HttpError>,
    {
        if let Some((key, value)) = self.try_header(key, value) {
            self.headers.append(key, value);
        }
        self
    }

    fn try_header<K, V>(&mut self, key: K, value: V) -> Option<(HeaderName, HeaderValue)>
    where
        HeaderName: TryFrom<K>,
        HeaderValue: TryFrom<V>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        <HeaderValue as TryFrom<V>>::Error: Into<HttpError>,
    {
        match HeaderName::try_from(key) {
            Ok(key) => match HeaderValue::try_from(value) {
                Ok(_) if is_reserved(&key) => {}
                Ok(value) => return Some((key, value)),
                Err(e) => self.set_error(e),
            },
            Err(e) => self.set_error(e),
        }
        None
    }

    fn set_error<T: Into<HttpError>>(&mut self, err: T) {
        if self.err.is_none() {
            self.err = Some(err.into());
        }
    }

    /// Take the first error recorded while building the request.
    ///
    /// [`header()`](Self::header) does not fail on an invalid name or value, it
    /// remembers the error instead. Built-in transports return it as
    /// [`ClientError::Http`](super::ClientError::Http) before sending anything,
    /// custom transports should do the same.
    pub fn take_error(&mut self) -> Option<HttpError> {
        self.err.take()
    }

    /// Clear existing headers and timeout.
    ///
    /// The timeout is sent as the `grpc-timeout` header, so it is removed too.
    pub fn clear(&mut self) -> &mut Self {
        self.headers.clear();
        self.timeout = None;
        self
    }

    /// Headers set for the call, including `grpc-timeout`.
    ///
    /// A custom transport sends them with the request.
    pub fn headers(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
        self.headers.iter()
    }

    /// Check if the connection must be closed when the request is dropped.
    pub fn get_disconnect_on_drop(&self) -> bool {
        self.flags.contains(Flags::DISCONNECT_ON_DROP)
    }
}

/// A call that is ready to be sent.
///
/// Generated client methods return it. Set headers or a timeout if you need
/// them, then [`send()`](Self::send) it. The request borrows its message, so
/// keep the message in a variable rather than building it inline.
pub struct Request<'a, T, M>
where
    T: Transport<M>,
    T: 'a,
    M: MethodDef,
{
    input: &'a M::Input,
    transport: &'a T,
    ctx: RequestContext,
}

impl<'a, T, M> Request<'a, T, M>
where
    T: Transport<M>,
    M: MethodDef,
{
    /// Create a call that sends `input` through `transport`.
    ///
    /// Generated clients do this for you.
    pub fn new(transport: &'a T, input: &'a M::Input) -> Self {
        Self {
            input,
            transport,
            ctx: RequestContext::new(),
        }
    }

    /// Set a request header, replacing any existing value for the same name.
    ///
    /// An invalid name or value does not panic, [`send()`](Self::send) returns
    /// the error instead. Headers the client sets itself, such as
    /// `content-type` or `grpc-timeout`, are ignored, see
    /// [`RequestContext::header()`].
    ///
    /// ```rust,ignore
    /// // `GreeterClient` and `HelloRequest` are generated from a .proto file
    /// let msg = HelloRequest { name: "world".into(), ..Default::default() };
    ///
    /// let mut req = client.say_hello(&msg);
    /// req.header("x-request-id", "42");
    /// let res = req.send().await?;
    /// ```
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: TryFrom<K>,
        HeaderValue: TryFrom<V>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        <HeaderValue as TryFrom<V>>::Error: Into<HttpError>,
    {
        self.ctx.header(key, value);
        self
    }

    /// Add a request header, keeping existing values for the same name.
    ///
    /// gRPC metadata can have several values per key, they are sent as
    /// separate headers.
    ///
    /// ```rust,ignore
    /// let mut req = client.say_hello(&msg);
    /// req.append_header("x-tag", "a").append_header("x-tag", "b");
    /// // binary metadata is base64 encoded, the key ends with `-bin`
    /// req.header("x-trace-bin", ntex_grpc::encode_binary_header(&[0, 1, 2]));
    /// ```
    pub fn append_header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: TryFrom<K>,
        HeaderValue: TryFrom<V>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        <HeaderValue as TryFrom<V>>::Error: Into<HttpError>,
    {
        self.ctx.append_header(key, value);
        self
    }

    /// Set the max duration the request is allowed to take.
    ///
    /// The duration is sent in the `grpc-timeout` header, formatted according
    /// to [the spec] with the most precise unit that fits, rounded up and at
    /// most `99999999H`. Built-in transports also stop waiting when it runs
    /// out and return [`ClientError::DeadlineExceeded`](super::ClientError::DeadlineExceeded).
    /// A zero timeout fails the call without sending it.
    ///
    /// [the spec]: https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
    pub fn timeout<U>(&mut self, timeout: U) -> &mut Self
    where
        time::Duration: From<U>,
    {
        self.ctx.timeout(timeout);
        self
    }

    /// Set the size limit of the response message, 4 MiB by default.
    ///
    /// Built-in transports reset the stream and return
    /// [`GrpcStatus::ResourceExhausted`](crate::GrpcStatus::ResourceExhausted)
    /// once the server declares a larger message, without reading it.
    pub fn max_message_size(&mut self, size: usize) -> &mut Self {
        self.ctx.max_message_size(size);
        self
    }

    /// Set the size limit of the request message, 2 GiB - 1 by default.
    ///
    /// Built-in transports return
    /// [`GrpcStatus::ResourceExhausted`](crate::GrpcStatus::ResourceExhausted)
    /// for a larger message, without sending it.
    pub fn max_send_message_size(&mut self, size: usize) -> &mut Self {
        self.ctx.max_send_message_size(size);
        self
    }

    /// Compress the request message with gzip or zstd.
    ///
    /// The server must support the encoding, otherwise the call fails with
    /// [`GrpcStatus::Unimplemented`](crate::GrpcStatus::Unimplemented).
    ///
    /// Requires the `compression` feature.
    #[cfg(feature = "compression")]
    pub fn compression(&mut self, compression: Compression) -> &mut Self {
        self.ctx.compression(compression);
        self
    }

    /// Send request
    pub async fn send(self) -> Result<Response<M>, T::Error> {
        let Request {
            input,
            transport,
            mut ctx,
        } = self;

        transport.request(input, &mut ctx).await
    }
}

/// Checks if the client sets the header itself.
fn is_reserved(key: &HeaderName) -> bool {
    matches!(
        key.as_str(),
        "content-type"
            | "user-agent"
            | "te"
            | "grpc-encoding"
            | "grpc-message-type"
            | "grpc-message"
            | "grpc-status"
            | "grpc-timeout"
    )
}

fn duration_to_grpc_timeout(duration: time::Duration) -> String {
    // nanoseconds in each unit, from the most precise
    const UNITS: [(char, u128); 6] = [
        ('n', 1),
        ('u', 1_000),
        ('m', 1_000_000),
        ('S', 1_000_000_000),
        ('M', 60_000_000_000),
        ('H', 3_600_000_000_000),
    ];
    // the spec allows at most 8 digits
    const MAX_VALUE: u128 = 99_999_999;

    let nanos = duration.as_nanos();
    for (unit, size) in UNITS {
        // rounded up, the server must not give up before the client does
        let value = nanos.div_ceil(size);
        if value <= MAX_VALUE {
            return format!("{value}{unit}");
        }
    }
    // more than 11,000 years
    format!("{MAX_VALUE}H")
}

/// Successful reply to a call.
///
/// Derefs to the reply message, so its fields can be used directly.
pub struct Response<T: MethodDef> {
    /// The reply message.
    pub output: T::Output,
    /// Response headers.
    pub headers: HeaderMap,
    /// Trailers sent after the message, `grpc-status` is one of them.
    pub trailers: HeaderMap,
    /// Size of the request body in bytes, with the 5-byte message prefix.
    pub req_size: usize,
    /// Size of the response body in bytes, with the 5-byte message prefix.
    pub res_size: usize,
}

impl<T: MethodDef> Response<T> {
    #[inline]
    /// Response headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    #[inline]
    /// Trailers sent after the message.
    pub fn trailers(&self) -> &HeaderMap {
        &self.trailers
    }

    #[inline]
    /// Take the reply message and drop the headers.
    pub fn into_inner(self) -> T::Output {
        self.output
    }

    #[inline]
    /// Split into the reply message, headers and trailers.
    pub fn into_parts(self) -> (T::Output, HeaderMap, HeaderMap) {
        (self.output, self.headers, self.trailers)
    }
}

impl<T: MethodDef> ops::Deref for Response<T> {
    type Target = T::Output;

    fn deref(&self) -> &Self::Target {
        &self.output
    }
}

impl<T: MethodDef> ops::DerefMut for Response<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.output
    }
}

impl<T: MethodDef> fmt::Debug for Response<T>
where
    T::Output: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(format!("ResponseFor<{}>", T::NAME).as_str())
            .field("output", &self.output)
            .field("headers", &self.headers)
            .field("trailers", &self.trailers)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use ntex_bytes::{ByteString, Bytes};

    use super::*;
    use crate::google_types::BytesValue;

    struct TestMethod;

    impl MethodDef for TestMethod {
        const NAME: &'static str = "TestMethod";
        const PATH: ByteString = ByteString::from_static("/test.Svc/Test");
        type Input = ();
        type Output = BytesValue;
    }

    #[test]
    fn context_header_and_clear() {
        let mut ctx = RequestContext::new();
        ctx.header("x-a", "1").header("x-a", "2");
        let hdrs: Vec<_> = ctx.headers().collect();
        assert_eq!(
            hdrs,
            [(
                &HeaderName::from_static("x-a"),
                &HeaderValue::from_static("2")
            )]
        );

        ctx.append_header("x-a", "3");
        let mut vals: Vec<_> = ctx.headers().map(|(_, v)| v.clone()).collect();
        vals.sort();
        assert_eq!(vals, ["2", "3"]);

        // invalid values are not added
        ctx.append_header("x-a", "\n");
        assert!(ctx.take_error().is_some());
        assert_eq!(ctx.headers().count(), 2);

        ctx.header("x-a", "1");
        ctx.timeout(time::Duration::from_secs(1));
        assert_eq!(ctx.headers().count(), 2);
        assert!(
            ctx.headers()
                .any(|(k, v)| k == consts::GRPC_TIMEOUT && v == "1000000u")
        );
        ctx.clear();
        assert_eq!(ctx.headers().count(), 0);
        assert_eq!(ctx.get_timeout(), None);
    }

    #[test]
    fn reserved_headers() {
        let mut ctx = RequestContext::new();
        for key in [
            "content-type",
            "User-Agent",
            "te",
            "grpc-encoding",
            "grpc-message-type",
            "grpc-message",
            "grpc-status",
            "grpc-timeout",
        ] {
            ctx.header(key, "1").append_header(key, "2");
        }
        assert_eq!(ctx.headers().count(), 0);
        assert!(ctx.take_error().is_none());

        // the value is still checked
        ctx.header("te", "\n");
        assert!(ctx.take_error().is_some());

        // not reserved
        ctx.header("grpc-accept-encoding", "gzip")
            .header("grpc-previous-rpc-attempts", "1")
            .header("x-te", "1");
        assert_eq!(ctx.headers().count(), 3);

        ctx.timeout(time::Duration::from_secs(1));
        ctx.header("grpc-timeout", "1S");
        let timeout: Vec<_> = ctx
            .headers()
            .filter(|(k, _)| *k == consts::GRPC_TIMEOUT)
            .collect();
        assert_eq!(timeout.len(), 1);
        assert_eq!(timeout[0].1, "1000000u");
    }

    #[test]
    fn context_max_message_size() {
        let mut ctx = RequestContext::new();
        assert_eq!(ctx.get_max_message_size(), 4 * 1024 * 1024);
        ctx.max_message_size(10);
        assert_eq!(ctx.get_max_message_size(), 10);

        assert_eq!(ctx.get_max_send_message_size(), 2_147_483_647);
        ctx.max_send_message_size(20);
        assert_eq!(ctx.get_max_send_message_size(), 20);
        assert_eq!(ctx.get_max_message_size(), 10);
    }

    #[cfg(feature = "compression")]
    #[test]
    fn context_compression() {
        let mut ctx = RequestContext::new();
        assert_eq!(ctx.get_compression(), None);
        ctx.compression(Compression::Zstd);
        assert_eq!(ctx.get_compression(), Some(Compression::Zstd));
    }

    #[test]
    fn context_disconnect_on_drop() {
        let mut ctx = RequestContext::new();
        assert!(!ctx.get_disconnect_on_drop());
        ctx.disconnect_on_drop();
        assert!(ctx.get_disconnect_on_drop());
    }

    #[test]
    fn duration_to_grpc_timeout_less_than_second() {
        let timeout = time::Duration::from_millis(500);
        let value = duration_to_grpc_timeout(timeout);
        assert_eq!(value, format!("{}u", timeout.as_micros()));

        let timeout = time::Duration::from_secs(30);
        let value = duration_to_grpc_timeout(timeout);
        assert_eq!(value, format!("{}u", timeout.as_micros()));

        let one_hour = time::Duration::from_hours(1);
        let value = duration_to_grpc_timeout(one_hour);
        assert_eq!(value, format!("{}m", one_hour.as_millis()));

        assert_eq!(duration_to_grpc_timeout(time::Duration::ZERO), "0n");
        assert_eq!(
            duration_to_grpc_timeout(time::Duration::from_nanos(1)),
            "1n"
        );
    }

    #[test]
    fn duration_to_grpc_timeout_units() {
        // each unit is used once the smaller one needs more than 8 digits
        for (secs, expect) in [
            (100_000_u64, "100000S"),
            (100_000_000, "1666667M"),
            (6_000_000_000, "1666667H"),
        ] {
            let value = duration_to_grpc_timeout(time::Duration::from_secs(secs));
            assert_eq!(value, expect, "{secs}");
        }

        // a value that does not fit a unit is rounded up
        for (duration, expect) in [
            (time::Duration::new(100, 1), "100001m"),
            (time::Duration::new(100_000, 1), "100001S"),
            (time::Duration::from_hours(99_999_999), "99999999H"),
            // larger values are capped
            (
                time::Duration::from_hours(99_999_999) + time::Duration::from_nanos(1),
                "99999999H",
            ),
            (time::Duration::MAX, "99999999H"),
        ] {
            assert_eq!(duration_to_grpc_timeout(duration), expect, "{duration:?}");
        }
    }

    #[test]
    fn response_parts() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-a"),
            HeaderValue::from_static("1"),
        );
        let mut trailers = HeaderMap::new();
        trailers.insert(consts::GRPC_STATUS, HeaderValue::from_static("0"));
        let response = || Response::<TestMethod> {
            output: BytesValue {
                value: Bytes::from_static(b"body"),
            },
            headers: headers.clone(),
            trailers: trailers.clone(),
            req_size: 5,
            res_size: 9,
        };

        // derefs to the message
        let mut res = response();
        assert_eq!(res.value, Bytes::from_static(b"body"));
        assert_eq!(res.headers().get("x-a").unwrap(), "1");
        assert_eq!(res.trailers().get(consts::GRPC_STATUS).unwrap(), "0");
        res.value = Bytes::from_static(b"other");
        assert_eq!(res.into_inner().value, Bytes::from_static(b"other"));

        let (output, headers, trailers) = response().into_parts();
        assert_eq!(output.value, Bytes::from_static(b"body"));
        assert_eq!(headers.get("x-a").unwrap(), "1");
        assert_eq!(trailers.get(consts::GRPC_STATUS).unwrap(), "0");
    }

    struct BigMethod;

    impl MethodDef for BigMethod {
        const NAME: &'static str = "BigMethod";
        const PATH: ByteString = ByteString::from_static("/test.Svc/Big");
        type Input = BytesValue;
        type Output = BytesValue;
    }

    /// Sends a message that does not fit the flow control window to a peer
    /// that never replies, so the request runs into its deadline while the
    /// stream is still open.
    async fn aborted_request(disconnect: bool) -> ntex_h2::client::SimpleClient {
        use ntex::{io::Io, service::cfg::SharedCfg, testing::IoTest};

        let (cli, srv) = IoTest::create();
        cli.remote_buffer_cap(64 * 1024);
        srv.remote_buffer_cap(64 * 1024);
        std::mem::forget(srv);

        let client = ntex_h2::client::SimpleClient::new(
            Io::new(cli, SharedCfg::new("CLI").build()),
            false,
            "localhost".into(),
        );
        let input = BytesValue {
            value: Bytes::from(vec![0; 1024 * 1024]),
        };
        let mut ctx = RequestContext::new();
        ctx.timeout(time::Duration::from_millis(50));
        if disconnect {
            ctx.disconnect_on_drop();
        }

        let err = <_ as Transport<BigMethod>>::request(&client, &input, &mut ctx)
            .await
            .unwrap_err();
        assert!(
            matches!(*err, crate::client::ClientError::DeadlineExceeded(_)),
            "{err:?}"
        );
        client
    }

    #[ntex::test]
    async fn disconnect_on_drop() {
        // the unfinished stream takes the connection with it
        let client = aborted_request(true).await;
        assert!(client.is_disconnecting() || client.is_closed());

        // without the flag the connection stays open
        let client = aborted_request(false).await;
        assert!(!client.is_disconnecting() && !client.is_closed());
    }
}
