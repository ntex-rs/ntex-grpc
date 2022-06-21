#![allow(clippy::declare_interior_mutable_const)]
use ntex_http::HeaderValue;

pub(crate) const HDRV_CT_GRPC: HeaderValue = HeaderValue::from_static("application/grpc");
pub(crate) const HDRV_USER_AGENT: HeaderValue = HeaderValue::from_static("ntex-grpc/1.0.0");
pub(crate) const HDRV_TRAILERS: HeaderValue = HeaderValue::from_static("trailers");
