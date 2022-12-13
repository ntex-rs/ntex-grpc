use std::{future::Future, rc::Rc};

use ntex_h2::client;
use ntex_http::error::Error as HttpError;
use ntex_io::OnDisconnect;

mod connector;
mod request;
mod transport;

pub use self::connector::Connector;
pub use self::request::{Request, RequestContext, Response};

use crate::service::MethodDef;

pub trait Transport<T: MethodDef> {
    /// Errors produced by the transport.
    type Error: From<HttpError>;

    /// The transport response value.
    type Future<'f>: Future<Output = Result<Response<T>, Self::Error>>
    where
        Self: 'f,
        T::Input: 'f;

    fn request<'a>(&'a self, args: &'a T::Input, ctx: RequestContext) -> Self::Future<'a>;
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

#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    #[error("{0}")]
    Http(#[from] ntex_h2::client::ClientError),
}

#[derive(Clone)]
pub struct Client(Rc<transport::Inner>);

impl Client {
    #[inline]
    /// Gracefully close connection
    pub fn close(&self) {
        self.0.client.close()
    }

    #[inline]
    /// Check if connection is closed
    pub fn is_closed(&self) -> bool {
        self.0.client.is_closed()
    }

    #[inline]
    /// Notify when connection get closed
    pub fn on_disconnect(&self) -> OnDisconnect {
        self.0.client.on_disconnect()
    }

    #[inline]
    /// Get reference to h2 client
    pub fn get_ref(&self) -> &client::Client {
        &self.0.client
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // one for current client and one for Client::start() call
        if Rc::strong_count(&self.0) <= 2 {
            self.0.client.close()
        }
    }
}
