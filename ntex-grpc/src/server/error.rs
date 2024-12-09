use ntex_http::{HeaderMap, HeaderValue};

use crate::{DecodeError, GrpcStatus};

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
