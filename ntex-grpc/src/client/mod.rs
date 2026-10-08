#![allow(async_fn_in_trait)]

use std::borrow::Cow;

use ntex_bytes::Bytes;
use ntex_error::ErrorDiagnostic;
use ntex_h2::{OperationError, StreamError, client};
use ntex_http::{HeaderMap, StatusCode, error::Error as HttpError};

mod request;
mod transport;

pub use self::request::{Request, RequestContext, Response};

use crate::{consts, encoding::DecodeError, service::MethodDef, status::GrpcStatus, utils};

/// Sends unary calls of method `T`.
///
/// Generated clients go through this trait for every call. Implement it to
/// put something between the client and the connection, like metrics or
/// retries. [`Client`], [`ntex_h2::client::Client`] and
/// [`ntex_h2::client::SimpleClient`] implement it.
pub trait Transport<T: MethodDef> {
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
        ctx: &mut RequestContext,
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
/// Cloning is cheap and clones share the pool. The pool itself,
/// [`ntex_h2::client::Client`], is a transport as well, this type only gives
/// it a name in this crate.
#[derive(Clone)]
pub struct Client(client::Client);

impl Client {
    #[inline]
    /// Create grpc client transport from h2 client
    pub fn new(client: client::Client) -> Self {
        Self(client)
    }

    #[inline]
    /// Get reference to h2 client
    pub fn get_ref(&self) -> &client::Client {
        &self.0
    }
}

/// Errors from the built-in transports.
///
/// Transports return it wrapped in [`ntex_error::Error`], which derefs to
/// `ClientError`, so match on `&*err`:
///
/// ```rust,ignore
/// match req.send().await {
///     // `req` comes from a generated client, e.g. `client.say_hello(&msg)`
///     Ok(res) => println!("{}", res.message),
///     Err(err) => match &*err {
///         // the server replied, but not with OK
///         ClientError::GrpcStatus(status, _, _) => {
///             println!("{status:?}: {:?}", err.grpc_message());
///         }
///         // anything else is a transport or protocol problem
///         _ => println!("call failed: {err}"),
///     },
/// }
/// ```
#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    /// The connection pool could not provide a connection.
    #[error("HTTP2 Client")]
    Client(
        #[from]
        #[source]
        client::ClientError,
    ),
    /// A request header has an invalid name or value, nothing was sent.
    ///
    /// See [`RequestContext::take_error()`].
    #[error("Http error {0:?}")]
    Http(
        #[from]
        #[source]
        HttpError,
    ),
    /// The reply message could not be decoded.
    #[error("Decode")]
    Decode(
        #[from]
        #[source]
        DecodeError,
    ),
    /// The HTTP/2 connection failed, e.g. it was closed during the call.
    #[error("HTTP2 Operation")]
    Operation(
        #[from]
        #[source]
        OperationError,
    ),
    /// The HTTP/2 stream failed, e.g. the server broke the protocol.
    ///
    /// A reset from the server is reported as [`ClientError::GrpcStatus`].
    #[error("HTTP2 Stream")]
    Stream(
        #[from]
        #[source]
        StreamError,
    ),
    /// The response had no HTTP status.
    ///
    /// Holds the status, the headers and the body received. The status is
    /// `None`, a status other than 200 is reported as [`ClientError::GrpcStatus`].
    #[error("Http response {0:?}, headers: {1:?}, body: {2:?}")]
    Response(Option<StatusCode>, HeaderMap, Bytes),
    /// The response ended before a complete reply message arrived.
    ///
    /// Holds the HTTP status and the headers received so far.
    #[error("Got eof without payload")]
    UnexpectedEof(Option<StatusCode>, HeaderMap),
    /// The request timeout ran out.
    ///
    /// Holds the trailers if the server replied with `DEADLINE_EXCEEDED`.
    /// The headers are empty if the client stopped waiting by itself, the
    /// stream is reset with `CANCEL` then.
    #[error("Deadline exceeded")]
    DeadlineExceeded(HeaderMap),
    /// The server replied with a status other than `OK`.
    ///
    /// Holds the status and the trailers, the error text is in the
    /// `grpc-message` trailer, see [`ClientError::grpc_message()`].
    ///
    /// If the server sent no valid `grpc-status`, the client picks one:
    ///
    /// * an unknown or invalid `grpc-status` gives `UNKNOWN`, a
    ///   `grpc-message` from the server is kept.
    /// * an HTTP status other than 200 is mapped as the gRPC spec says, e.g. 404 to
    ///   `UNIMPLEMENTED` and 503 to `UNAVAILABLE`. The response headers take
    ///   the place of the trailers here.
    /// * a stream reset by the server is mapped as the gRPC spec says, e.g.
    ///   `REFUSED_STREAM` to `UNAVAILABLE` and `CANCEL` to `CANCELLED`. The
    ///   response headers take the place of the trailers here.
    /// * a `content-type` other than `application/grpc` gives `UNKNOWN`.
    /// * trailers without `grpc-status` give `UNKNOWN`.
    /// * a response that ends without trailers gives `INTERNAL`.
    ///
    /// The client adds a `grpc-message` describing the problem then, and
    /// keeps the response body in the third field. The body is `None` if the
    /// status came from the server.
    #[error("Grpc status")]
    GrpcStatus(GrpcStatus, HeaderMap, Option<Bytes>),
}

impl ClientError {
    /// The `grpc-message` sent with a non-OK status, percent-decoded.
    ///
    /// Only [`ClientError::GrpcStatus`] and [`ClientError::DeadlineExceeded`]
    /// carry one. Invalid escapes are kept as they are.
    pub fn grpc_message(&self) -> Option<Cow<'_, str>> {
        match self {
            Self::GrpcStatus(_, hdrs, _) | Self::DeadlineExceeded(hdrs) => {
                hdrs.get(consts::GRPC_MESSAGE).map(utils::percent_decode)
            }
            _ => None,
        }
    }
}

impl Clone for ClientError {
    fn clone(&self) -> Self {
        match self {
            Self::Client(err) => Self::Client(err.clone()),
            Self::Http(err) => Self::Http(*err),
            Self::Decode(err) => Self::Decode(err.clone()),
            Self::Operation(err) => Self::Operation(*err),
            Self::Stream(err) => Self::Stream(*err),
            Self::Response(st, hdrs, payload) => {
                Self::Response(*st, hdrs.clone(), payload.clone())
            }
            Self::UnexpectedEof(st, hdrs) => Self::UnexpectedEof(*st, hdrs.clone()),
            Self::DeadlineExceeded(hdrs) => Self::DeadlineExceeded(hdrs.clone()),
            Self::GrpcStatus(st, hdrs, body) => Self::GrpcStatus(*st, hdrs.clone(), body.clone()),
        }
    }
}

impl ErrorDiagnostic for ClientError {
    fn signature(&self) -> &'static str {
        match self {
            ClientError::Client(err) => err.signature(),
            ClientError::Http(_) => "grpc-Http",
            ClientError::Decode(_) => "grpc-Decode",
            ClientError::Operation(err) => err.signature(),
            ClientError::Stream(err) => err.signature(),
            ClientError::Response(_, _, _) => "grpc-Response",
            ClientError::UnexpectedEof(_, _) => "grpc-UnexpectedEof",
            ClientError::DeadlineExceeded(_) => "grpc-BackendCallTimedout",
            ClientError::GrpcStatus(status, _, _) => status.signature(),
        }
    }
}
