#![allow(clippy::declare_interior_mutable_const)]
use ntex_http::{HeaderName, HeaderValue};

pub(crate) const HDRV_CT_GRPC: HeaderValue = HeaderValue::from_static("application/grpc");
pub(crate) const HDRV_USER_AGENT: HeaderValue = HeaderValue::from_static("ntex-grpc/1.0.0");
pub(crate) const HDRV_TRAILERS: HeaderValue = HeaderValue::from_static("trailers");

pub const GRPC_STATUS: HeaderName = HeaderName::from_static("grpc-status");
pub const GRPC_MESSAGE: HeaderName = HeaderName::from_static("grpc-message");

pub(crate) const GRPC_ENCODING: HeaderName = HeaderName::from_static("grpc-encoding");
pub(crate) const GRPC_ACCEPT_ENCODING: HeaderName =
    HeaderName::from_static("grpc-accept-encoding");
pub(crate) const IDENTITY: HeaderValue = HeaderValue::from_static("identity");
