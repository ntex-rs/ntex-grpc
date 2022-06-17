mod consts;
mod error;
mod request;
mod service;

pub mod client;

pub use crate::error::ServiceError;
pub use crate::request::{Request, Response};
pub use crate::service::{Client, MethodDef, Transport};

#[doc(hidden)]
pub mod codegen {
    pub use ntex::service::Service;
    pub use ntex::util::ByteString;

    pub use crate::request::{Request, Response};
    pub use crate::service::{Client, MethodDef, Transport};
    pub use crate::ServiceError;
}
