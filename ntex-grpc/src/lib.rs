mod consts;
mod error;
mod request;
mod status;
mod transport;

pub mod client;
pub mod types;

pub use crate::error::{DecodeError, ServiceError};
pub use crate::request::{Request, Response};
pub use crate::status::GrpcStatus;
pub use crate::transport::{ClientInformation, MethodDef, Transport};
pub use crate::types::{BytesAdapter, Message};

#[doc(hidden)]
pub mod encoding;

#[doc(hidden)]
pub mod prost {
    pub use prost::*;
}

use ntex_http::HeaderName;

#[allow(clippy::declare_interior_mutable_const)]
pub const GRPC_STATUS: HeaderName = HeaderName::from_static("grpc-status");
#[allow(clippy::declare_interior_mutable_const)]
pub const GRPC_MESSAGE: HeaderName = HeaderName::from_static("grpc-message");

// [1]: https://github.com/serde-rs/serde/blob/v1.0.89/serde/src/lib.rs#L245-L256
#[allow(unused_imports)]
#[macro_use]
extern crate ntex_prost_derive;
#[doc(hidden)]
pub use ntex_prost_derive::*;
