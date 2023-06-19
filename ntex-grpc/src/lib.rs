mod consts;
mod error;
mod service;
mod status;
mod utils;

pub mod client;
pub mod server;
pub mod types;

pub use crate::encoding::DecodeError;
pub use crate::error::ServiceError;
pub use ntex_http::Error as HttpError;

pub use crate::service::{MethodDef, ServiceDef};
pub use crate::status::GrpcStatus;
pub use crate::types::{Message, NativeType};

#[doc(hidden)]
pub mod google_types;

#[doc(hidden)]
pub mod encoding;
#[doc(hidden)]
pub use self::encoding::WireType;
#[doc(hidden)]
pub use ntex_bytes::{ByteString, Bytes, BytesMut};
#[doc(hidden)]
pub use ntex_service::{Service, ServiceCtx, ServiceFactory};
#[doc(hidden)]
pub use ntex_util::{future::BoxFuture, HashMap};

// [1]: https://github.com/serde-rs/serde/blob/v1.0.89/serde/src/lib.rs#L245-L256
#[allow(unused_imports)]
#[macro_use]
extern crate ntex_grpc_derive;
#[doc(hidden)]
pub use ntex_grpc_derive::*;
