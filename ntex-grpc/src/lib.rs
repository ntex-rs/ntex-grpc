mod consts;
mod error;
mod request;
mod service;
mod status;
mod utils;

pub mod client;
pub mod server;
pub mod types;

pub use crate::error::{DecodeError, HttpError, ServerError, ServiceError};
pub use crate::request::{Request, RequestContext, Response};
pub use crate::service::{ClientInformation, MethodDef, MethodsDef, ServiceDef, Transport};
pub use crate::status::GrpcStatus;
pub use crate::types::{Message, NativeType};

#[doc(hidden)]
pub mod encoding;
#[doc(hidden)]
pub use self::encoding::WireType;
#[doc(hidden)]
pub use ntex_bytes::{ByteString, Bytes, BytesMut};
#[doc(hidden)]
pub use ntex_util::HashMap;

// [1]: https://github.com/serde-rs/serde/blob/v1.0.89/serde/src/lib.rs#L245-L256
#[allow(unused_imports)]
#[macro_use]
extern crate ntex_grpc_derive;
#[doc(hidden)]
pub use ntex_grpc_derive::*;
