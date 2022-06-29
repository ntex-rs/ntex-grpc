use async_trait::async_trait;
use ntex_bytes::ByteString;

use crate::{request::Response, types::Message};

/// Trait for service method definition
pub trait MethodDef {
    const NAME: &'static str;

    const PATH: ByteString;

    type Input: Message;

    type Output: Message;
}

#[async_trait(?Send)]
pub trait Transport<T: MethodDef> {
    /// Errors produced by the service.
    type Error;

    async fn request(&self, args: &T::Input) -> Result<Response<T>, Self::Error>;
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
