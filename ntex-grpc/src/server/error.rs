use ntex_http::{HeaderMap, HeaderValue};

use crate::{DecodeError, GrpcStatus};

pub trait GrpcError {
    fn status(&self) -> GrpcStatus;

    fn message(&self) -> HeaderValue;

    fn headers(&self) -> HeaderMap;
}

#[derive(thiserror::Error, Clone, Debug)]
#[error("{status:?}: {message:?}")]
pub struct ServerError {
    pub(crate) status: GrpcStatus,
    pub(crate) message: HeaderValue,
    pub(crate) headers: HeaderMap,
}

impl ServerError {
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
