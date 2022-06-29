use ntex_h2::{OperationError, StreamError};
use ntex_http::{HeaderMap, StatusCode};
pub use prost::DecodeError;

use crate::status::GrpcStatus;

#[derive(thiserror::Error, Clone, Debug)]
pub enum ServiceError {
    #[error("Canceled")]
    Canceled,
    #[error("{0}")]
    ProstEncoder(#[from] prost::EncodeError),
    #[error("Http operation error: {0}")]
    Operation(#[from] OperationError),
    #[error("Http stream error: {0}")]
    Stream(#[from] StreamError),
    #[error("Http response {0}, headers: {1:?}")]
    Response(StatusCode, HeaderMap),
    #[error("Unknown response status, headers: {0:?}")]
    UnknownResponseStatus(HeaderMap),
    #[error("Unexpected disconnect with {0}, headers: {1:?}")]
    UnexpectedDisconnect(StatusCode, HeaderMap),
    #[error("Grpc status {0:?}, headers: {1:?}")]
    GrpcStatus(GrpcStatus, HeaderMap),
}
