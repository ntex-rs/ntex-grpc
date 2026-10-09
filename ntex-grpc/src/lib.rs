//! gRPC client and server for [ntex](https://docs.rs/ntex).
//!
//! Messages, clients and service definitions are generated from `.proto`
//! files with the `ntex-grpc-codegen` tool. This crate is the runtime part
//! that the generated code builds on:
//!
//! - [`client`] sends calls over an `ntex-h2` connection,
//! - [`server`](mod@server) serves a service, usually one written with the
//!   `#[server]` attribute,
//! - [`types`] has the protobuf encoding traits that generated messages
//!   implement,
//! - [`google_types`] has the protobuf well-known types.
//!
//! Only unary calls are supported, streaming methods are not generated.
#![deny(clippy::pedantic)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::missing_errors_doc,
    clippy::missing_fields_in_debug,
    clippy::must_use_candidate,
    clippy::unused_async_trait_impl
)]
#[cfg(feature = "compression")]
mod compression;
mod consts;
mod format;
mod service;
mod status;
mod utils;

/// gRPC client.
///
/// Generated clients, like `GreeterClient<T>`, are generic over a
/// [`Transport`](client::Transport). [`Client`](client::Client) and the
/// `ntex-h2` clients implement it, so in most cases you build an h2 client
/// and pass it to the generated client's `new()`.
pub mod client;
/// gRPC server.
///
/// [`GrpcServer`](server::GrpcServer) accepts HTTP/2 connections and passes
/// each call to a service. The `#[server]` attribute writes that service for
/// you from a plain `impl` block, the rest of this module is what the
/// generated code works with.
pub mod server;
/// Protobuf encoding traits used by generated code.
pub mod types;

#[cfg(feature = "compression")]
pub use crate::compression::Compression;
pub use crate::encoding::DecodeError;
pub use crate::service::{MethodDef, ServiceDef};
pub use crate::status::GrpcStatus;
pub use crate::types::{Message, NativeType};
pub use crate::utils::{decode_binary_header, decode_binary_header_values, encode_binary_header};

/// Protobuf well-known types.
///
/// Generated code uses these for `google.protobuf.*` fields, unless the
/// codegen runs with `--well-known-types`.
pub mod google_types;

#[doc(hidden)]
pub mod encoding;
#[doc(hidden)]
pub use self::encoding::WireType;
#[doc(hidden)]
pub use ntex_bytes::{BytePages, ByteString, Bytes, BytesMut};
#[doc(hidden)]
pub use ntex_http::HeaderValue;
#[doc(hidden)]
pub use ntex_service::{Ctx, Service, ServiceFactory};
#[doc(hidden)]
pub use ntex_util::HashMap;

// [1]: https://github.com/serde-rs/serde/blob/v1.0.89/serde/src/lib.rs#L245-L256
#[allow(unused_imports)]
#[macro_use]
extern crate ntex_grpc_derive;
#[doc(hidden)]
pub use ntex_grpc_derive::*;
