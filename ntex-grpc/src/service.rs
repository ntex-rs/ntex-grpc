use ntex_bytes::{ByteString, Bytes, BytesMut};

use crate::{encoding::DecodeError, server::MethodResult, types::Message};

/// Trait for service method definition
pub trait ServiceDef {
    const NAME: &'static str;

    type Methods;

    fn method_by_name(name: &str) -> Option<Self::Methods>;
}

/// Trait for service method definition
pub trait MethodDef {
    const NAME: &'static str;

    const PATH: ByteString;

    type Input: Message;

    type Output: Message;

    #[inline]
    fn decode(&self, buf: &mut Bytes) -> Result<Self::Input, DecodeError> {
        Message::read(buf)
    }

    #[inline]
    fn encode(&self, val: Self::Output, buf: &mut BytesMut) {
        val.write(buf);
    }

    #[doc(hidden)]
    #[inline]
    fn server_result<T: MethodResult<Self::Output>>(&self, val: T) -> Self::Output {
        val.into()
    }
}
