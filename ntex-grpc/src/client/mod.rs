use std::future::Future;

use ntex_bytes::Bytes;
use ntex_h2::{client, OperationError, StreamError};
use ntex_http::{error::Error as HttpError, HeaderMap, StatusCode};

mod request;
mod transport;

pub use self::request::{Request, RequestContext, Response};

use crate::{encoding::DecodeError, service::MethodDef, status::GrpcStatus};

pub trait Transport<T: MethodDef> {
    /// Errors produced by the transport.
    type Error: From<HttpError>;

    fn request(
        &self,
        args: &T::Input,
        ctx: RequestContext,
    ) -> impl Future<Output = Result<Response<T>, Self::Error>>;
}

/// Client utils methods
pub trait ClientInformation<T> {
    /// Create new client instance
    fn create(transport: T) -> Self;

    /// Get reference to underlying transport
    fn transport(&self) -> &T;

    /// Get mut referece to underlying transport
    fn transport_mut(&mut self) -> &mut T;

    /// Consume client and return inner transport
    fn into_inner(self) -> T;
}

#[derive(Clone)]
pub struct Client(client::Client);

impl Client {
    #[inline]
    /// Get reference to h2 client
    pub fn new(client: client::Client) -> Self {
        Self(client)
    }

    #[inline]
    /// Get reference to h2 client
    pub fn get_ref(&self) -> &client::Client {
        &self.0
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    #[error("{0}")]
    Client(#[from] client::ClientError),
    #[error("Http error {0:?}")]
    Http(Option<HttpError>),
    #[error("{0}")]
    Decode(#[from] DecodeError),
    #[error("Http operation error: {0}")]
    Operation(#[from] OperationError),
    #[error("Http stream error: {0}")]
    Stream(#[from] StreamError),
    #[error("Http response {0:?}, headers: {1:?}, body: {2:?}")]
    Response(Option<StatusCode>, HeaderMap, Bytes),
    #[error("Got eof without payload with {0:?}, headers: {1:?}")]
    UnexpectedEof(Option<StatusCode>, HeaderMap),
    #[error("Grpc status {0:?}, headers: {1:?}")]
    GrpcStatus(GrpcStatus, HeaderMap),
}

impl From<HttpError> for ClientError {
    fn from(err: HttpError) -> Self {
        Self::Http(Some(err))
    }
}

impl Clone for ClientError {
    fn clone(&self) -> Self {
        match self {
            Self::Client(err) => Self::Client(err.clone()),
            Self::Http(_) => Self::Http(None),
            Self::Decode(err) => Self::Decode(err.clone()),
            Self::Operation(err) => Self::Operation(err.clone()),
            Self::Stream(err) => Self::Stream(*err),
            Self::Response(st, hdrs, payload) => {
                Self::Response(*st, hdrs.clone(), payload.clone())
            }
            Self::UnexpectedEof(st, hdrs) => Self::UnexpectedEof(*st, hdrs.clone()),
            Self::GrpcStatus(st, hdrs) => Self::GrpcStatus(*st, hdrs.clone()),
        }
    }
}
