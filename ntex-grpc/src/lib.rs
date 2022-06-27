mod consts;
mod error;
mod request;
mod service;

pub mod client;
pub mod types;

pub use crate::error::{DecodeError, ServiceError};
pub use crate::request::{Request, Response};
pub use crate::service::{ClientInformation, MethodDef, Transport};
pub use crate::types::{BytesAdapter, Message};

#[doc(hidden)]
pub mod encoding;

#[doc(hidden)]
pub mod codegen {
    pub use ntex_bytes::{ByteString, BytesMut};
    pub use ntex_service::Service;

    pub use crate::request::{Request, Response};
    pub use crate::service::{ClientInformation, MethodDef, Transport};
    pub use crate::ServiceError;
}

// [1]: https://github.com/serde-rs/serde/blob/v1.0.89/serde/src/lib.rs#L245-L256
#[allow(unused_imports)]
#[macro_use]
extern crate ntex_prost_derive;
#[doc(hidden)]
pub use ntex_prost_derive::*;
