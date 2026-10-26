#![allow(async_fn_in_trait)]

use std::{borrow::Cow, fmt};

use ntex_bytes::{ByteString, Bytes};
use ntex_error::ErrorDiagnostic;
use ntex_h2::{OperationError, StreamError, client};
use ntex_http::{HeaderMap, StatusCode, error::Error as HttpError, header};

pub(crate) mod request;
mod stream;
pub(crate) mod transport;

pub use self::request::{Request, RequestContext, Response};
pub use self::stream::{
    Bidi, ClientStreaming, ServerStreaming, StreamCall, StreamRequest, StreamTransport,
};

use crate::{consts, encoding::DecodeError, service::MethodDef, status::GrpcStatus, utils};

/// Sends unary calls of method `T`.
///
/// Generated clients go through this trait for every call. Implement it to
/// put something between the client and the connection, like metrics or
/// retries. [`Client`], [`ntex_h2::client::Client`] and
/// [`ntex_h2::client::SimpleClient`] implement it.
pub trait Transport<T: MethodDef, St = ()> {
    /// Errors produced by the transport.
    type Error;

    /// Send `args` as the request message and wait for the reply.
    ///
    /// `ctx` holds the headers and timeout set on the [`Request`]. If
    /// [`RequestContext::take_error()`] returns an error, return it without
    /// sending anything.
    async fn request(
        &self,
        args: &T::Input,
        ctx: &mut RequestContext<St>,
    ) -> Result<Response<T>, Self::Error>;
}

/// Client utils methods
pub trait ClientInformation<T> {
    /// Create new client instance
    fn create(transport: T) -> Self;

    /// Get reference to underlying transport
    fn transport(&self) -> &T;

    /// Get mut reference to underlying transport
    fn transport_mut(&mut self) -> &mut T;

    /// Consume client and return inner transport
    fn into_inner(self) -> T;
}

/// [`Transport`] on top of an `ntex-h2` connection pool.
///
/// Cloning is cheap and clones share the pool, copying the message-size
/// defaults. [`max_message_size()`](Self::max_message_size) and
/// [`max_send_message_size()`](Self::max_send_message_size) set defaults for
/// unary and streaming calls; explicit per-request limits take precedence.
/// The pool itself, [`ntex_h2::client::Client`], is a transport as well, but
/// does not carry these gRPC defaults.
#[derive(Clone)]
pub struct Client {
    pool: client::Client,
    max_message_size: usize,
    max_send_message_size: usize,
}

impl Client {
    #[inline]
    /// Create grpc client transport from h2 client
    pub fn new(client: client::Client) -> Self {
        Self {
            pool: client,
            max_message_size: request::DEFAULT_MAX_MESSAGE_SIZE,
            max_send_message_size: request::DEFAULT_MAX_SEND_MESSAGE_SIZE,
        }
    }

    #[must_use]
    /// Set the default size limit of each response message, in bytes.
    ///
    /// The default is 4 MiB. Both the received and decompressed message
    /// must fit. A larger message fails the call with
    /// [`GrpcStatus::ResourceExhausted`].
    ///
    /// [`Request::max_message_size()`] and
    /// [`StreamRequest::max_message_size()`] override this for one call.
    pub fn max_message_size(mut self, size: usize) -> Self {
        self.max_message_size = size;
        self
    }

    #[must_use]
    /// Set the default size limit of each request message, in bytes.
    ///
    /// The default is 2 GiB - 1. The limit applies after compression. A
    /// larger message is not sent and fails with
    /// [`GrpcStatus::ResourceExhausted`]. Messages of 4 GiB or larger cannot
    /// be sent, regardless of this limit.
    ///
    /// [`Request::max_send_message_size()`] and
    /// [`StreamRequest::max_send_message_size()`] override this for one call.
    pub fn max_send_message_size(mut self, size: usize) -> Self {
        self.max_send_message_size = size;
        self
    }

    #[inline]
    /// Get reference to h2 client
    pub fn get_ref(&self) -> &client::Client {
        &self.pool
    }
}

/// Why a call failed.
///
/// Built-in transports return it wrapped in [`ntex_error::Error`], which
/// derefs to `ClientError`, so match on `&*err`.
///
/// Each variant calls for a different reaction from a caller that retries
/// calls or tracks the health of endpoints. Within a variant,
/// [`message()`](Self::message) describes the cause, and
/// [`status()`](Self::status) gives the gRPC code of every variant.
///
/// | Variant | Server saw the request? | What a caller can do |
/// | --- | --- | --- |
/// | [`Status`](Self::Status) | yes | apply the rules for the code |
/// | [`InvalidRequest`](Self::InvalidRequest) | no | fail the call, retrying fails the same way |
/// | [`NotProcessed`](Self::NotProcessed) | no | retry at once, for any method |
/// | [`Connection`](Self::Connection) | maybe, see `request_sent` | retry elsewhere |
/// | [`StreamFailed`](Self::StreamFailed) | maybe | retry if the method is idempotent |
/// | [`NotGrpc`](Self::NotGrpc) | not by a grpc server | check the address, or back off on 429, 502, 503 and 504 |
/// | [`InvalidResponse`](Self::InvalidResponse) | yes | fail the call, asking again gets the same answer |
/// | [`DeadlineExceeded`](Self::DeadlineExceeded) | maybe | the time for the call is up |
///
/// ```rust,ignore
/// match req.send().await {
///     // `req` comes from a generated client, e.g. `client.say_hello(&msg)`
///     Ok(res) => println!("{}", res.message),
///     Err(err) => match &*err {
///         // the server replied, but not with OK
///         ClientError::Status(st) => println!("{:?}: {}", st.code, err.message()),
///         // safe to retry, the server did not process the request
///         ClientError::NotProcessed(_) => println!("retry"),
///         _ => println!("call failed with {:?}: {err}", err.status()),
///     },
/// }
/// ```
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ClientError {
    /// The server replied with a `grpc-status` other than `OK`.
    ///
    /// The only variant that holds a status the server sent, including
    /// `DEADLINE_EXCEEDED`.
    Status(Status),
    /// The request is not valid, nothing was sent and sending it again fails
    /// the same way.
    ///
    /// A header with an invalid name or value (see
    /// [`RequestContext::take_error()`]), headers over the server's limit, a
    /// message over [`Request::max_send_message_size()`] or a message that
    /// does not compress.
    InvalidRequest(CallFailure),
    /// The server did not process the request, retrying is safe for any
    /// method.
    ///
    /// The connection was closing or out of stream identifiers when the call
    /// started, the server refused the stream with `REFUSED_STREAM`, or the
    /// server's GOAWAY did not cover the stream.
    NotProcessed(CallFailure),
    /// The connection could not be established, or it failed or closed
    /// before the call ended.
    Connection {
        /// Whether the request was handed to the connection.
        ///
        /// If false the server did not see the request and retrying is safe
        /// for any method, otherwise the server may have processed it.
        request_sent: bool,
        /// The failure, its source is the `ntex-h2` error.
        failure: CallFailure,
    },
    /// The stream of the call failed while the connection stayed up.
    ///
    /// The server reset it, or broke the protocol on it, or the client reset
    /// it because it could not send. The server may have processed the
    /// request. A reset is mapped to a code as the gRPC spec says, e.g.
    /// `CANCEL` to `CANCELLED` and `ENHANCE_YOUR_CALM` to
    /// `RESOURCE_EXHAUSTED`.
    StreamFailed(CallFailure),
    /// Something other than a grpc server replied: the HTTP status is not 200
    /// or the `content-type` is not `application/grpc`.
    NotGrpc(NotGrpc),
    /// The server replied with a grpc response the client cannot accept.
    ///
    /// The response has an invalid or no `grpc-status`, ends without
    /// trailers or a complete message, has data after the message, a message
    /// over [`Request::max_message_size()`], in an unknown encoding or that
    /// does not decode.
    InvalidResponse(CallFailure),
    /// The request timeout ran out before the server replied.
    ///
    /// The client stopped waiting, the stream is reset with `CANCEL` then,
    /// or the server reset the stream with `CANCEL` once the timeout had run
    /// out. A `DEADLINE_EXCEEDED` status from the server is a
    /// [`Status`](Self::Status).
    DeadlineExceeded(CallFailure),
}

/// A status other than `OK` that the server sent.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Status {
    /// The status code.
    pub code: GrpcStatus,
    /// The trailers, or the headers of a trailers-only response.
    ///
    /// They hold `grpc-message`, `grpc-status-details-bin` and the other
    /// metadata from the server.
    pub metadata: HeaderMap,
}

/// A failure the client detected.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CallFailure {
    /// The gRPC code of the failure, as the gRPC spec maps it.
    ///
    /// E.g. `UNAVAILABLE` for a lost connection or a refused stream,
    /// `RESOURCE_EXHAUSTED` for a message over a limit.
    pub code: GrpcStatus,
    /// What happened, e.g. "Connection is disconnecting" or "Response ended
    /// without trailers".
    pub message: ByteString,
    /// The response headers received before the failure, or the trailers
    /// if the response ended. Empty if none arrived.
    pub headers: HeaderMap,
    /// The error behind the failure, also returned by
    /// [`source()`](std::error::Error::source).
    pub cause: Option<Cause>,
}

/// A response from something other than a grpc server.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct NotGrpc {
    /// The HTTP status of the response, if it had one.
    pub http_status: Option<StatusCode>,
    /// The response headers.
    pub headers: HeaderMap,
    /// Up to 64 KiB of the response body, e.g. an error page from a proxy.
    pub body: Bytes,
}

/// The error behind a [`CallFailure`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Cause {
    /// The connection pool could not provide a connection.
    Client(client::ClientError),
    /// The HTTP/2 connection or stream failed.
    Operation(OperationError),
    /// The HTTP/2 stream failed.
    Stream(StreamError),
    /// The request could not be built, e.g. an invalid header.
    Http(HttpError),
    /// The response message could not be decoded.
    Decode(DecodeError),
}

impl ClientError {
    /// The gRPC code of the failure.
    ///
    /// The server's code for [`Status`](Self::Status), the code the HTTP
    /// status maps to for [`NotGrpc`](Self::NotGrpc) and
    /// [`CallFailure::code`] otherwise.
    pub fn status(&self) -> GrpcStatus {
        match self {
            Self::Status(st) => st.code,
            Self::NotGrpc(resp) => resp.status(),
            Self::InvalidRequest(f)
            | Self::NotProcessed(f)
            | Self::Connection { failure: f, .. }
            | Self::StreamFailed(f)
            | Self::InvalidResponse(f)
            | Self::DeadlineExceeded(f) => f.code,
        }
    }

    /// What went wrong.
    ///
    /// The percent-decoded `grpc-message` of the server for
    /// [`Status`](Self::Status), empty if it sent none. A description of the
    /// failure otherwise.
    pub fn message(&self) -> Cow<'_, str> {
        match self {
            Self::Status(st) => st.message(),
            Self::NotGrpc(resp) => resp.message(),
            Self::InvalidRequest(f)
            | Self::NotProcessed(f)
            | Self::Connection { failure: f, .. }
            | Self::StreamFailed(f)
            | Self::InvalidResponse(f)
            | Self::DeadlineExceeded(f) => Cow::Borrowed(&f.message),
        }
    }
}

impl Status {
    /// Create a status, `metadata` holds the trailers.
    pub fn new(code: GrpcStatus, metadata: HeaderMap) -> Self {
        Self { code, metadata }
    }

    /// The percent-decoded `grpc-message`, empty if the server sent none.
    ///
    /// Invalid escapes are kept as they are.
    pub fn message(&self) -> Cow<'_, str> {
        self.metadata
            .get(consts::GRPC_MESSAGE)
            .map_or(Cow::Borrowed(""), utils::percent_decode)
    }
}

impl CallFailure {
    /// Create a failure with no headers.
    pub fn new(code: GrpcStatus, message: impl Into<ByteString>) -> Self {
        Self {
            code,
            message: message.into(),
            headers: HeaderMap::new(),
            cause: None,
        }
    }

    /// Set the response headers received before the failure.
    #[must_use]
    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }

    /// Set the error behind the failure.
    #[must_use]
    pub fn with_cause(mut self, cause: impl Into<Cause>) -> Self {
        self.cause = Some(cause.into());
        self
    }
}

impl NotGrpc {
    /// Create a response from something other than a grpc server.
    pub fn new(http_status: Option<StatusCode>, headers: HeaderMap, body: Bytes) -> Self {
        Self {
            http_status,
            headers,
            body,
        }
    }

    /// The gRPC code the HTTP status maps to, as the gRPC spec says.
    ///
    /// E.g. 404 is `UNIMPLEMENTED` and 503 is `UNAVAILABLE`. A 200 response
    /// whose `content-type` is not grpc, or a response without status, is
    /// `UNKNOWN`.
    pub fn status(&self) -> GrpcStatus {
        match self.http_status.map(|st| st.as_u16()) {
            Some(400) => GrpcStatus::Internal,
            Some(401) => GrpcStatus::Unauthenticated,
            Some(403) => GrpcStatus::PermissionDenied,
            Some(404) => GrpcStatus::Unimplemented,
            Some(429 | 502 | 503 | 504) => GrpcStatus::Unavailable,
            _ => GrpcStatus::Unknown,
        }
    }

    /// Describes the response, e.g. "HTTP status 503".
    pub fn message(&self) -> Cow<'_, str> {
        match self.http_status {
            None => Cow::Borrowed("Response has no HTTP status"),
            Some(st) if st != StatusCode::OK => Cow::Owned(format!("HTTP status {}", st.as_u16())),
            Some(_) => match self.headers.get(header::CONTENT_TYPE) {
                None => Cow::Borrowed("Response has no content-type"),
                Some(val) => Cow::Owned(format!(
                    "Invalid content-type: {}",
                    String::from_utf8_lossy(val.as_bytes())
                )),
            },
        }
    }
}

macro_rules! cause_from {
    ($($var:ident($ty:ty)),+) => {
        $(impl From<$ty> for Cause {
            fn from(err: $ty) -> Self {
                Cause::$var(err)
            }
        })+
    };
}

cause_from!(
    Client(client::ClientError),
    Operation(OperationError),
    Stream(StreamError),
    Http(HttpError),
    Decode(DecodeError)
);

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(st) => write!(f, "Server replied with {:?}: {}", st.code, st.message()),
            Self::InvalidRequest(e) => write!(f, "Invalid request: {e}"),
            Self::NotProcessed(e) => write!(f, "Request is not processed: {e}"),
            Self::Connection { failure, .. } => write!(f, "Connection failed: {failure}"),
            Self::StreamFailed(e) => write!(f, "Stream failed: {e}"),
            Self::NotGrpc(resp) => write!(f, "Not a grpc response: {}", resp.message()),
            Self::InvalidResponse(e) => write!(f, "Invalid response: {e}"),
            Self::DeadlineExceeded(e) => write!(f, "Deadline exceeded: {e}"),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Status(_) | Self::NotGrpc(_) => None,
            Self::InvalidRequest(f)
            | Self::NotProcessed(f)
            | Self::Connection { failure: f, .. }
            | Self::StreamFailed(f)
            | Self::InvalidResponse(f)
            | Self::DeadlineExceeded(f) => f.source(),
        }
    }
}

impl fmt::Display for CallFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CallFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self.cause.as_ref()? {
            Cause::Client(err) => Some(err),
            Cause::Operation(err) => Some(err),
            Cause::Stream(err) => Some(err),
            Cause::Http(err) => Some(err),
            Cause::Decode(err) => Some(err),
        }
    }
}

impl ErrorDiagnostic for ClientError {
    fn signature(&self) -> &'static str {
        match self {
            Self::Status(st) => st.code.signature(),
            Self::InvalidRequest(_) => "grpc-InvalidRequest",
            Self::NotProcessed(_) => "grpc-NotProcessed",
            Self::Connection { .. } => "grpc-Connection",
            Self::StreamFailed(_) => "grpc-StreamFailed",
            Self::NotGrpc(_) => "grpc-NotGrpc",
            Self::InvalidResponse(_) => "grpc-InvalidResponse",
            Self::DeadlineExceeded(_) => "grpc-BackendCallTimedout",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use ntex_http::HeaderValue;

    use super::*;

    #[test]
    fn errors() {
        let mut hdrs = HeaderMap::new();
        hdrs.insert(consts::GRPC_MESSAGE, HeaderValue::from_static("a%20b"));
        let failure = |code| CallFailure::new(code, "msg").with_headers(hdrs.clone());

        let errors = [
            (
                ClientError::Status(Status::new(GrpcStatus::NotFound, hdrs.clone())),
                "grpc-status-NotFound",
                GrpcStatus::NotFound,
                "Server replied with NotFound: a b",
            ),
            (
                ClientError::InvalidRequest(
                    failure(GrpcStatus::Internal)
                        .with_cause(HttpError::from(HeaderValue::from_str("\n").unwrap_err())),
                ),
                "grpc-InvalidRequest",
                GrpcStatus::Internal,
                "Invalid request: msg",
            ),
            (
                ClientError::NotProcessed(failure(GrpcStatus::Unavailable)),
                "grpc-NotProcessed",
                GrpcStatus::Unavailable,
                "Request is not processed: msg",
            ),
            (
                ClientError::Connection {
                    request_sent: false,
                    failure: failure(GrpcStatus::Unavailable)
                        .with_cause(client::ClientError::HandshakeTimeout),
                },
                "grpc-Connection",
                GrpcStatus::Unavailable,
                "Connection failed: msg",
            ),
            (
                ClientError::StreamFailed(
                    failure(GrpcStatus::Cancelled).with_cause(StreamError::Closed),
                ),
                "grpc-StreamFailed",
                GrpcStatus::Cancelled,
                "Stream failed: msg",
            ),
            (
                ClientError::NotGrpc(NotGrpc::new(
                    Some(StatusCode::BAD_GATEWAY),
                    hdrs.clone(),
                    Bytes::from_static(b"body"),
                )),
                "grpc-NotGrpc",
                GrpcStatus::Unavailable,
                "Not a grpc response: HTTP status 502",
            ),
            (
                ClientError::InvalidResponse(
                    failure(GrpcStatus::Internal).with_cause(DecodeError::new("bad")),
                ),
                "grpc-InvalidResponse",
                GrpcStatus::Internal,
                "Invalid response: msg",
            ),
            (
                ClientError::DeadlineExceeded(
                    failure(GrpcStatus::DeadlineExceeded).with_cause(OperationError::Disconnected),
                ),
                "grpc-BackendCallTimedout",
                GrpcStatus::DeadlineExceeded,
                "Deadline exceeded: msg",
            ),
        ];

        for (err, signature, status, display) in errors {
            let copy = err.clone();
            assert_eq!(copy.signature(), signature);
            assert_eq!(copy.status(), status);
            assert_eq!(copy.to_string(), display);
            assert_eq!(format!("{copy:?}"), format!("{err:?}"));
            assert_eq!(copy.message(), err.message());
            // failures with a cause expose it
            let has_source = !matches!(
                err,
                ClientError::Status(_) | ClientError::NotGrpc(_) | ClientError::NotProcessed(_)
            );
            assert_eq!(copy.source().is_some(), has_source, "{err:?}");
        }

        // only a server status has the server's message
        let st = ClientError::Status(Status::new(GrpcStatus::Internal, hdrs.clone()));
        assert_eq!(st.message(), "a b");
        let st = ClientError::Status(Status::new(GrpcStatus::Internal, HeaderMap::new()));
        assert_eq!(st.message(), "");
        let err = ClientError::InvalidResponse(failure(GrpcStatus::Internal));
        assert_eq!(err.message(), "msg");
    }

    #[test]
    fn not_grpc() {
        for (code, status) in [
            (400, GrpcStatus::Internal),
            (401, GrpcStatus::Unauthenticated),
            (403, GrpcStatus::PermissionDenied),
            (404, GrpcStatus::Unimplemented),
            (429, GrpcStatus::Unavailable),
            (502, GrpcStatus::Unavailable),
            (503, GrpcStatus::Unavailable),
            (504, GrpcStatus::Unavailable),
            (500, GrpcStatus::Unknown),
            (302, GrpcStatus::Unknown),
            (204, GrpcStatus::Unknown),
        ] {
            let st = StatusCode::from_u16(code).unwrap();
            let resp = NotGrpc::new(Some(st), HeaderMap::new(), Bytes::new());
            assert_eq!(resp.status(), status, "{code}");
            assert_eq!(resp.message(), format!("HTTP status {code}"));
        }

        let mut resp = NotGrpc::new(Some(StatusCode::OK), HeaderMap::new(), Bytes::new());
        assert_eq!(resp.status(), GrpcStatus::Unknown);
        assert_eq!(resp.message(), "Response has no content-type");
        resp.headers
            .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html"));
        assert_eq!(resp.message(), "Invalid content-type: text/html");

        let resp = NotGrpc::new(None, HeaderMap::new(), Bytes::new());
        assert_eq!(resp.status(), GrpcStatus::Unknown);
        assert_eq!(resp.message(), "Response has no HTTP status");
    }
}
