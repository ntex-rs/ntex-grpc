use ntex_bytes::{ByteString, Bytes};
use ntex_h2::{OperationError, StreamError};
use ntex_http::{HeaderMap, StatusCode};

pub use ntex_http::error::Error as HttpError;

pub use crate::encoding::DecodeError;
use crate::status::GrpcStatus;

#[derive(thiserror::Error, Debug)]
pub enum ServiceError {
    #[error("Canceled")]
    Canceled,
    #[error("Http error {0:?}")]
    Http(Option<HttpError>),
    #[error("{0}")]
    Decode(#[from] DecodeError),
    #[error("Http operation error: {0}")]
    Operation(#[from] OperationError),
    #[error("Http stream error: {0}")]
    Stream(#[from] StreamError),
    #[error("Http response {0:?}, headers: {1:?}, body: {2:?}")]
    Response(Option<StatusCode>, HeaderMap, Bytes),
    #[error("Got eof without payload with {0:?}, headers: {1:?}")]
    UnexpectedEof(Option<StatusCode>, HeaderMap),
    #[error("Grpc status {0:?}, headers: {1:?}")]
    GrpcStatus(GrpcStatus, HeaderMap),
}

#[derive(thiserror::Error, Clone, Debug)]
pub enum ServerError {
    #[error("{0}")]
    Decode(#[from] DecodeError),
    #[error("Service method is not found: {0}")]
    NotFound(ByteString),
    #[error("Service method is not implemented: {0}")]
    NotImplemented(ByteString),
}

impl From<HttpError> for ServiceError {
    fn from(err: HttpError) -> Self {
        Self::Http(Some(err))
    }
}

impl Clone for ServiceError {
    fn clone(&self) -> Self {
        match self {
            ServiceError::Canceled => ServiceError::Canceled,
            ServiceError::Http(_) => ServiceError::Http(None),
            ServiceError::Decode(err) => ServiceError::Decode(err.clone()),
            ServiceError::Operation(err) => ServiceError::Operation(err.clone()),
            ServiceError::Stream(err) => ServiceError::Stream(*err),
            ServiceError::Response(st, hdrs, payload) => {
                ServiceError::Response(*st, hdrs.clone(), payload.clone())
            }
            ServiceError::UnexpectedEof(st, hdrs) => {
                ServiceError::UnexpectedEof(*st, hdrs.clone())
            }
            ServiceError::GrpcStatus(st, hdrs) => ServiceError::GrpcStatus(*st, hdrs.clone()),
        }
    }
}
