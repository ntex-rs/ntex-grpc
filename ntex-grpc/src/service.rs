use ntex_bytes::{BytePages, ByteString, Bytes};

use crate::{encoding::DecodeError, server::MethodResult, types::Message};

/// A gRPC service, generated for each `service` in a `.proto` file.
pub trait ServiceDef {
    /// Full service name with the package, e.g. `helloworld.Greeter`.
    const NAME: &'static str;

    /// Enum with one variant per method.
    type Methods;

    /// Find a method by the name used in the request path, e.g. `SayHello`.
    fn method_by_name(name: &str) -> Option<Self::Methods>;
}

/// A method of a gRPC service, generated for each `rpc` in a `.proto` file.
pub trait MethodDef {
    /// Method name, e.g. `SayHello`.
    const NAME: &'static str;

    /// Request path, e.g. `/helloworld.Greeter/SayHello`.
    const PATH: ByteString;

    /// Request message.
    type Input: Message;

    /// Reply message.
    type Output: Message;

    #[inline]
    /// Decode a request message.
    fn decode(&self, buf: &mut Bytes) -> Result<Self::Input, DecodeError> {
        Message::read(buf)
    }

    #[inline]
    /// Encode a reply message.
    fn encode(&self, val: Self::Output, buf: &mut BytePages) {
        val.write(buf);
    }

    #[doc(hidden)]
    #[inline]
    fn server_result<T: MethodResult<Self::Output>>(&self, val: T) -> Self::Output {
        val.into()
    }
}
