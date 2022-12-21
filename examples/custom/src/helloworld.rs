#![allow(
    dead_code,
    unused_mut,
    unused_variables,
    clippy::identity_op,
    clippy::derivable_impls,
    clippy::unit_arg
)]
/// DO NOT MODIFY. Auto-generated file

///  The request message containing the user's name.
#[derive(Clone, PartialEq, Debug)]
pub struct HelloRequest {
    pub name: ::ntex_grpc::ByteString,
    pub msg_id: crate::unique_id::UniqueId,
}

///  The response message containing the greetings
#[derive(Clone, PartialEq, Debug)]
pub struct HelloReply {
    pub message: ::ntex_grpc::ByteString,
}

/// `Greeter` service definition
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Greeter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GreeterMethods {
    SayHello(GreeterSayHelloMethod),
}

/// The greeting service definition.
#[derive(Debug, Clone)]
pub struct GreeterClient<T>(T);

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct GreeterSayHelloMethod;

impl ::ntex_grpc::MethodDef for GreeterSayHelloMethod {
    const NAME: &'static str = "SayHello";
    const PATH: ::ntex_grpc::ByteString =
        ::ntex_grpc::ByteString::from_static("/helloworld.Greeter/SayHello");
    type Input = HelloRequest;
    type Output = HelloReply;
}

mod _priv_impl {
    use super::*;

    impl ::ntex_grpc::Message for HelloRequest {
        #[inline]
        fn write(&self, dst: &mut ::ntex_grpc::BytesMut) {
            ::ntex_grpc::NativeType::serialize(
                &self.name,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.msg_id,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
        }

        #[inline]
        fn read(
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<Self, ::ntex_grpc::DecodeError> {
            const STRUCT_NAME: &str = "HelloRequest";
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = ::ntex_grpc::encoding::decode_key(src)?;
                match tag {
                    1 => ::ntex_grpc::NativeType::deserialize(&mut msg.name, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "name"))?,
                    2 => {
                        ::ntex_grpc::NativeType::deserialize(&mut msg.msg_id, tag, wire_type, src)
                            .map_err(|err| err.push(STRUCT_NAME, "msg_id"))?
                    }
                    _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + ::ntex_grpc::NativeType::serialized_len(
                &self.name,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.msg_id,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for HelloRequest {
        #[inline]
        fn default() -> Self {
            Self {
                name: ::core::default::Default::default(),
                msg_id: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::Message for HelloReply {
        #[inline]
        fn write(&self, dst: &mut ::ntex_grpc::BytesMut) {
            ::ntex_grpc::NativeType::serialize(
                &self.message,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
        }

        #[inline]
        fn read(
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<Self, ::ntex_grpc::DecodeError> {
            const STRUCT_NAME: &str = "HelloReply";
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = ::ntex_grpc::encoding::decode_key(src)?;
                match tag {
                    1 => {
                        ::ntex_grpc::NativeType::deserialize(&mut msg.message, tag, wire_type, src)
                            .map_err(|err| err.push(STRUCT_NAME, "message"))?
                    }
                    _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + ::ntex_grpc::NativeType::serialized_len(
                &self.message,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for HelloReply {
        #[inline]
        fn default() -> Self {
            Self {
                message: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::ServiceDef for Greeter {
        const NAME: &'static str = "helloworld.Greeter";
        type Methods = GreeterMethods;

        #[inline]
        fn method_by_name(name: &str) -> Option<Self::Methods> {
            use ::ntex_grpc::MethodDef;
            match name {
                GreeterSayHelloMethod::NAME => {
                    Some(GreeterMethods::SayHello(GreeterSayHelloMethod))
                }
                _ => None,
            }
        }
    }

    impl<T> GreeterClient<T> {
        #[inline]
        /// Create new client instance
        pub fn new(transport: T) -> Self {
            Self(transport)
        }
    }

    impl<T> ::ntex_grpc::client::ClientInformation<T> for GreeterClient<T> {
        #[inline]
        /// Create new client instance
        fn create(transport: T) -> Self {
            Self(transport)
        }

        #[inline]
        /// Get referece to underlying transport
        fn transport(&self) -> &T {
            &self.0
        }

        #[inline]
        /// Get mut referece to underlying transport
        fn transport_mut(&mut self) -> &mut T {
            &mut self.0
        }

        #[inline]
        /// Consume client and return inner transport
        fn into_inner(self) -> T {
            self.0
        }
    }

    impl<T: ::ntex_grpc::client::Transport<GreeterSayHelloMethod>> GreeterClient<T> {
        /// Sends a greeting
        pub fn say_hello<'a>(
            &'a self,
            req: &'a super::HelloRequest,
        ) -> ::ntex_grpc::client::Request<'a, T, GreeterSayHelloMethod> {
            ::ntex_grpc::client::Request::new(&self.0, req)
        }
    }
}
