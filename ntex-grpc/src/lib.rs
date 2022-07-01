mod consts;
mod error;
mod request;
mod service;
mod status;
mod utils;

pub mod client;
pub mod server;
pub mod types;

pub use crate::error::{DecodeError, ServerError, ServiceError};
pub use crate::request::{Request, Response};
pub use crate::service::{ClientInformation, MethodDef, MethodsDef, ServiceDef, Transport};
pub use crate::status::GrpcStatus;
pub use crate::types::{Message, NativeType};

#[doc(hidden)]
pub mod encoding;

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
