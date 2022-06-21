use async_trait::async_trait;
use ntex_bytes::ByteString;
use ntex_http::HeaderMap;

/// Trait for service method definition
pub trait MethodDef {
    const NAME: &'static str;

    const PATH: ByteString;

    type Input: prost::Message;

    type Output: prost::Message + Default;
}

#[async_trait(?Send)]
pub trait Transport {
    /// Errors produced by the service.
    type Error;

    async fn request<T: MethodDef>(
        &self,
        args: T::Input,
    ) -> Result<(T::Output, HeaderMap), Self::Error>;
}

/// Client utils methods
pub trait Client<T> {
    fn transport(&self) -> &T;

    /// Get mut referece to underlying transport
    fn transport_mut(&mut self) -> &mut T;

    /// Consume client and return inner transport
    fn into_inner(self) -> T;
}
