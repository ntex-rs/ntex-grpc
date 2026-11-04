use ntex_http::{HeaderMap, HeaderValue};

use super::Response;
use crate::{DecodeError, GrpcStatus};

/// Error answer to a call.
///
/// It is sent as trailers: `grpc-status`, `grpc-message` and any extra
/// headers. Hand-written services return it directly. With `#[server]`, a
/// method returns it to fail the call (see [`MethodResult`] and
/// [`StreamResult`]); it is also produced for unknown methods and requests
/// that cannot be decoded.
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

/// Return types a unary or client-streaming `#[server]` method can have.
///
/// - `Reply`, the reply message.
/// - `Result<Reply, E>` where `E: Into<Reply>`. The error is turned into a
///   reply message and sent like any other reply, with status `OK`.
/// - [`Response<Reply>`](super::Response), the reply message plus extra
///   trailers.
/// - `Result<Response<Reply>, E>` where `E: Into<ServerError>`. The error
///   ends the call with its status, `grpc-message` and trailers.
pub trait MethodResult<T> {
    /// Turn into the reply, or the error the call ends with.
    ///
    /// # Errors
    ///
    /// Returns the error the call ends with.
    fn into_response(self) -> Result<Response<T>, ServerError>;
}

impl<T> MethodResult<T> for T {
    #[inline]
    fn into_response(self) -> Result<Response<T>, ServerError> {
        Ok(Response::new(self))
    }
}

impl<T, E: Into<T>> MethodResult<T> for Result<T, E> {
    #[inline]
    fn into_response(self) -> Result<Response<T>, ServerError> {
        match self {
            Ok(res) => Ok(Response::new(res)),
            Err(e) => Ok(Response::new(e.into())),
        }
    }
}

impl<T> MethodResult<T> for Response<T> {
    #[inline]
    fn into_response(self) -> Result<Response<T>, ServerError> {
        Ok(self)
    }
}

impl<T, E: Into<ServerError>> MethodResult<T> for Result<Response<T>, E> {
    #[inline]
    fn into_response(self) -> Result<Response<T>, ServerError> {
        self.map_err(Into::into)
    }
}

/// Return types a `#[server]` streaming method can have.
///
/// A bidirectional or server-streaming method returns `()`, or
/// `Result<(), E>` where `E: Into<ServerError>`. The error is sent as the
/// status of the call.
pub trait StreamResult {
    /// Turn into the result of the call.
    ///
    /// # Errors
    ///
    /// Returns the error the call ends with.
    fn into_result(self) -> Result<(), ServerError>;
}

impl StreamResult for () {
    #[inline]
    fn into_result(self) -> Result<(), ServerError> {
        Ok(())
    }
}

impl<E: Into<ServerError>> StreamResult for Result<(), E> {
    #[inline]
    fn into_result(self) -> Result<(), ServerError> {
        self.map_err(Into::into)
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

    /// STATEMENT: Every `MethodResult` return type turns into the reply or
    /// the error the call ends with.
    ///
    /// HOW:
    /// - Convert a plain reply and both arms of `Result<Reply, E: Into<Reply>>`,
    ///   and confirm each is an `OK` reply with no trailers.
    /// - Convert a `Response` with a trailer and confirm the trailer is kept.
    /// - Convert both arms of `Result<Response<Reply>, ServerError>` and
    ///   confirm the error keeps its status.
    #[test]
    fn method_result_conversion() {
        fn reply<R: MethodResult<u32>>(res: R) -> u32 {
            let res = res.into_response().unwrap();
            assert!(res.headers.is_empty());
            res.message
        }

        assert_eq!(reply(5), 5);
        assert_eq!(reply(Ok::<u32, u8>(7)), 7);
        assert_eq!(reply(Err::<u32, u8>(3)), 3);

        let res = Response {
            message: 9_u32,
            headers: vec![(
                HeaderName::from_static("x-test"),
                HeaderValue::from_static("1"),
            )],
        };
        let res = MethodResult::<u32>::into_response(res).unwrap();
        assert_eq!(res.message, 9);
        assert_eq!(res.headers.len(), 1);

        let ok: Result<Response<u32>, ServerError> = Ok(Response::new(4));
        assert_eq!(MethodResult::<u32>::into_response(ok).unwrap().message, 4);

        let err: Result<Response<u32>, ServerError> = Err(ServerError::new(
            GrpcStatus::NotFound,
            HeaderValue::from_static("nope"),
            None,
        ));
        let err = MethodResult::<u32>::into_response(err).err().unwrap();
        assert_eq!(err.status, GrpcStatus::NotFound);
    }
}
