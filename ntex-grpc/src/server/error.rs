use ntex_http::{HeaderMap, HeaderValue};

use crate::{DecodeError, GrpcStatus};

pub trait GrpcError {
    fn status(&self) -> GrpcStatus;

    fn message(&self) -> HeaderValue;

    fn headers(&self) -> Option<HeaderMap> {
        None
    }
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

pub trait MethodResult<T> {
    fn into(self) -> Result<T, ServerError>;
}

impl<T> MethodResult<T> for T {
    fn into(self) -> Result<T, ServerError> {
        Ok(self)
    }
}

impl<T, E: GrpcError> MethodResult<T> for Result<T, E> {
    fn into(self) -> Result<T, ServerError> {
        match self {
            Ok(res) => Ok(res),
            Err(e) => Err(ServerError::new(e.status(), e.message(), e.headers())),
        }
    }
}
