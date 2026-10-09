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
    /// percent-encoded, build it with
    /// [`encode_grpc_message()`](crate::encode_grpc_message).
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

#[cfg(test)]
mod tests {
    use ntex_http::HeaderName;

    use super::*;

    #[test]
    fn server_error() {
        let err = ServerError::new(GrpcStatus::NotFound, HeaderValue::from_static("nope"), None);
        assert_eq!(err.status, GrpcStatus::NotFound);
        assert_eq!(err.message, HeaderValue::from_static("nope"));
        assert!(err.headers.is_empty());

        let text = err.to_string();
        assert!(text.contains("NotFound"), "{text}");
        assert!(text.contains("nope"), "{text}");
        assert!(format!("{err:?}").contains("NotFound"));
    }

    #[test]
    fn server_error_with_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-test"),
            HeaderValue::from_static("1"),
        );

        let err = ServerError::new(
            GrpcStatus::Internal,
            HeaderValue::from_static("boom"),
            Some(headers),
        );
        let err = err.clone();
        assert_eq!(err.status, GrpcStatus::Internal);
        assert_eq!(
            err.headers.get("x-test").unwrap(),
            &HeaderValue::from_static("1")
        );
    }

    #[test]
    fn from_decode_error() {
        let err = ServerError::from(DecodeError::new("bad data"));
        assert_eq!(err.status, GrpcStatus::InvalidArgument);
        assert_eq!(
            err.message,
            HeaderValue::from_static("Cannot decode grpc message")
        );
        assert!(err.headers.is_empty());
    }

    #[test]
    fn method_result_conversion() {
        assert_eq!(<u32 as MethodResult<u32>>::into(5), 5);

        let ok: Result<u32, u8> = Ok(7);
        assert_eq!(<Result<u32, u8> as MethodResult<u32>>::into(ok), 7);

        let err: Result<u32, u8> = Err(3);
        assert_eq!(<Result<u32, u8> as MethodResult<u32>>::into(err), 3);
    }
}
