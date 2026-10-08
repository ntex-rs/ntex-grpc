#![allow(clippy::declare_interior_mutable_const)]
use ntex_http::{HeaderName, HeaderValue};

pub(crate) const HDRV_CT_GRPC: HeaderValue = HeaderValue::from_static("application/grpc");
/// `grpc-<language>-<variant>/<version>`, as the gRPC spec suggests.
pub(crate) const HDRV_USER_AGENT: HeaderValue =
    HeaderValue::from_static(concat!("grpc-rust-ntex/", env!("CARGO_PKG_VERSION")));
pub(crate) const HDRV_TRAILERS: HeaderValue = HeaderValue::from_static("trailers");

pub const GRPC_STATUS: HeaderName = HeaderName::from_static("grpc-status");
pub const GRPC_MESSAGE: HeaderName = HeaderName::from_static("grpc-message");

pub(crate) const GRPC_TIMEOUT: HeaderName = HeaderName::from_static("grpc-timeout");
pub(crate) const GRPC_ENCODING: HeaderName = HeaderName::from_static("grpc-encoding");
pub(crate) const GRPC_ACCEPT_ENCODING: HeaderName =
    HeaderName::from_static("grpc-accept-encoding");
pub(crate) const IDENTITY: HeaderValue = HeaderValue::from_static("identity");
/// The `grpc-accept-encoding` we send, only the encodings we support.
#[cfg(feature = "compression")]
pub(crate) const ACCEPT_ENCODING: HeaderValue = HeaderValue::from_static("gzip,zstd");
#[cfg(not(feature = "compression"))]
pub(crate) const ACCEPT_ENCODING: HeaderValue = IDENTITY;
