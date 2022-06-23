use ntex_h2::{OperationError, StreamError};

#[derive(thiserror::Error, Clone, Debug)]
pub enum ServiceError {
    #[error("Canceled")]
    Canceled,
    #[error("{0}")]
    ProstEncoder(#[from] prost::EncodeError),
    #[error("Http2 operation error: {0}")]
    Operation(#[from] OperationError),
    #[error("Http2 stream error: {0}")]
    Stream(#[from] StreamError),
}
