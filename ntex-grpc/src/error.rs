use ntex_bytes::ByteString;
use ntex_h2::{OperationError, StreamError};
use ntex_http::{HeaderMap, StatusCode};
pub use prost::{DecodeError, EncodeError};

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
    #[error("Got eof without payload with {0}, headers: {1:?}")]
    UnexpectedEof(StatusCode, HeaderMap),
    #[error("Grpc status {0:?}, headers: {1:?}")]
    GrpcStatus(GrpcStatus, HeaderMap),
}

#[derive(thiserror::Error, Clone, Debug)]
pub enum ServerError {
    #[error("{0}")]
    Encode(#[from] EncodeError),
    #[error("{0}")]
    Decode(#[from] DecodeError),
    #[error("Service method is not found: {0}")]
    NotFound(ByteString),
    #[error("Service method is not implemented: {0}")]
    NotImplemented(ByteString),
}
