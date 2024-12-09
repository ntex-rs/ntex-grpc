use std::ops;

use ntex_bytes::{ByteString, Bytes};
use ntex_http::{HeaderMap, HeaderName, HeaderValue};

mod error;
mod service;

pub use self::error::{GrpcError, ServerError};
pub use self::service::{GrpcServer, GrpcService};

#[derive(Debug)]
pub struct ServerRequest {
    pub name: ByteString,
    pub payload: Bytes,
    pub headers: HeaderMap,
}

#[derive(Debug)]
pub struct ServerResponse {
    pub payload: Bytes,
    pub headers: Vec<(HeaderName, HeaderValue)>,
}

impl ServerResponse {
    #[inline]
    pub fn new(payload: Bytes) -> ServerResponse {
        ServerResponse::with_headers(payload, Vec::new())
    }

    #[inline]
    pub fn with_headers(
        payload: Bytes,
        headers: Vec<(HeaderName, HeaderValue)>,
    ) -> ServerResponse {
        ServerResponse { payload, headers }
    }
}

pub trait FromRequest<T> {
    fn from(input: Request<T>) -> Self;
}

pub struct Request<T> {
    pub name: ByteString,
    pub headers: HeaderMap,
    pub message: T,
}

impl<T> FromRequest<T> for T {
    #[inline]
    fn from(input: Request<T>) -> T {
        input.message
    }
}

impl<T> FromRequest<T> for Request<T> {
    #[inline]
    fn from(input: Request<T>) -> Request<T> {
        input
    }
}

impl<T> Request<T> {
    pub fn into_inner(self) -> T {
        self.message
    }
}

impl<T> ops::Deref for Request<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.message
    }
}

impl<T> ops::DerefMut for Request<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.message
    }
}

pub struct Response<T> {
    pub message: T,
    pub headers: Vec<(HeaderName, HeaderValue)>,
}

impl<T> Response<T> {
    pub fn new(message: T) -> Self {
        Self {
            message,
            headers: Vec::new(),
        }
    }
}

impl<T> From<T> for Response<T> {
    fn from(message: T) -> Self {
        Response {
            message,
            headers: Vec::new(),
        }
    }
}
