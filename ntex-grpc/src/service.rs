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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding;
    use crate::types::{DefaultValue, NativeType};

    #[derive(Debug, Default, Clone, PartialEq)]
    struct Msg {
        value: u32,
    }

    impl Message for Msg {
        fn read(src: &mut Bytes) -> Result<Self, DecodeError> {
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = encoding::decode_key(src)?;
                match tag {
                    1 => NativeType::deserialize(&mut msg.value, tag, wire_type, src)?,
                    _ => encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        fn write(&self, dst: &mut BytePages) {
            NativeType::serialize(&self.value, 1, DefaultValue::Default, dst);
        }

        fn encoded_len(&self) -> usize {
            NativeType::serialized_len(&self.value, 1, DefaultValue::Default)
        }
    }

    #[derive(Debug)]
    struct Failure;

    impl From<Failure> for Msg {
        fn from(_: Failure) -> Msg {
            Msg { value: 0xff }
        }
    }

    struct Ping;

    impl MethodDef for Ping {
        const NAME: &'static str = "Ping";
        const PATH: ByteString = ByteString::from_static("/test.Service/Ping");

        type Input = Msg;
        type Output = Msg;
    }

    struct TestService;

    impl ServiceDef for TestService {
        const NAME: &'static str = "test.Service";

        type Methods = Ping;

        fn method_by_name(name: &str) -> Option<Ping> {
            if name == Ping::NAME { Some(Ping) } else { None }
        }
    }

    #[test]
    fn service_def_lookup() {
        assert_eq!(TestService::NAME, "test.Service");
        assert_eq!(Ping::PATH, "/test.Service/Ping");
        assert!(TestService::method_by_name("Ping").is_some());
        assert!(TestService::method_by_name("Pong").is_none());
    }

    #[test]
    fn default_encode_decode() {
        let mut buf = BytePages::default();
        Ping.encode(Msg { value: 150 }, &mut buf);

        let mut bytes = buf.freeze();
        assert_eq!(bytes.as_ref(), &[0x08, 0x96, 0x01]);
        assert_eq!(Ping.decode(&mut bytes).unwrap(), Msg { value: 150 });
        assert!(bytes.is_empty());

        // truncated varint
        assert!(Ping.decode(&mut Bytes::from_static(&[0x08])).is_err());

        // unknown fields are skipped
        let mut bytes = Bytes::from_static(&[0x10, 0x07, 0x08, 0x96, 0x01]);
        assert_eq!(Ping.decode(&mut bytes).unwrap(), Msg { value: 150 });
        assert_eq!(Message::encoded_len(&Msg { value: 150 }), 3);
        assert_eq!(Message::encoded_len(&Msg::default()), 0);
    }

    #[test]
    fn server_result_conversion() {
        assert_eq!(Ping.server_result(Msg { value: 1 }), Msg { value: 1 });

        let ok: Result<Msg, Failure> = Ok(Msg { value: 2 });
        assert_eq!(Ping.server_result(ok), Msg { value: 2 });

        let err: Result<Msg, Failure> = Err(Failure);
        assert_eq!(Ping.server_result(err), Msg { value: 0xff });
    }
}
