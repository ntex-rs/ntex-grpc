use async_trait::async_trait;
use ntex_bytes::{ByteString, Bytes, BytesMut};

use crate::{error::DecodeError, error::EncodeError, request::Response, types::Message};

/// Trait for service method definition
pub trait ServiceDef {
    const NAME: &'static str;

    type Methods: MethodsDef;

    #[inline]
    fn method_by_name(name: &str) -> Option<Self::Methods> {
        <Self::Methods as MethodsDef>::by_name(name)
    }
}

/// Service methods
pub trait MethodsDef: Sized {
    fn by_name(name: &str) -> Option<Self>;
}

/// Trait for service method definition
pub trait MethodDef {
    const NAME: &'static str;

    const PATH: ByteString;

    type Input: Message;

    type Output: Message;

    #[inline]
    fn decode(&self, buf: &mut Bytes) -> Result<Self::Input, DecodeError> {
        let mut value: Self::Input = Default::default();
        value.merge(buf)?;
        Ok(value)
    }

    #[inline]
    fn encode(&self, val: Self::Output, buf: &mut BytesMut) -> Result<(), EncodeError> {
        val.encode(buf)
    }
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
