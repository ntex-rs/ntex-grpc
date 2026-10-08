use ntex_http::{HeaderMap, HeaderValue};

use crate::{DecodeError, GrpcStatus};

/// Error answer to a call.
///
/// It is sent as trailers: `grpc-status`, `grpc-message` and any extra
/// headers. Hand-written services return it directly. With `#[server]` it is
/// produced for unknown methods and requests that cannot be decoded.
#[derive(thiserror::Error, Clone, Debug)]
#[error("{status:?}: {message:?}")]
pub struct ServerError {
    pub(crate) status: GrpcStatus,
    pub(crate) message: HeaderValue,
    pub(crate) headers: HeaderMap,
}

impl ServerError {
    /// Create an error from a status, a `grpc-message` text and extra
    /// trailers.
    ///
    /// The message is sent as is. The gRPC spec expects it to be
    /// percent-encoded if it contains anything other than printable ASCII.
    pub fn new(status: GrpcStatus, message: HeaderValue, headers: Option<HeaderMap>) -> Self {
        Self {
            status,
            message,
            headers: headers.unwrap_or_default(),
        }
    }
}

impl From<DecodeError> for ServerError {
    fn from(_: DecodeError) -> Self {
        Self::new(
            GrpcStatus::InvalidArgument,
            HeaderValue::from_static("Cannot decode grpc message"),
            None,
        )
    }
}

/// Return types a `#[server]` method can have.
///
/// A method returns its reply message, or `Result<Reply, E>` where
/// `E: Into<Reply>`. The error is turned into a reply message and sent like
/// any other reply, with status `OK`.
pub trait MethodResult<T> {
    /// Turn into the reply message.
    fn into(self) -> T;
}

impl<T> MethodResult<T> for T {
    #[inline]
    fn into(self) -> T {
        self
    }
}

impl<T, E: Into<T>> MethodResult<T> for Result<T, E> {
    #[inline]
    fn into(self) -> T {
        match self {
            Ok(res) => res,
            Err(e) => e.into(),
        }
    }
}
