#![allow(
    dead_code,
    unused_mut,
    unused_variables,
    clippy::identity_op,
    clippy::derivable_impls,
    clippy::unit_arg,
    clippy::derive_partial_eq_without_eq
)]
/// DO NOT MODIFY. Auto-generated file

///  The request message containing the user's name.
#[derive(Clone, PartialEq, Debug)]
pub struct HelloRequest {
    pub name: ::ntex_grpc::ByteString,
}

///  The response message containing the greetings
#[derive(Clone, PartialEq, Debug)]
pub struct HelloReply {
    pub metadata: i64,
    pub reply_type: hello_reply::Type,
    pub result: Option<hello_reply::Result>,
}

/// Nested message and enum types in `HelloReply`.
pub mod hello_reply {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
    #[repr(i32)]
    pub enum Type {
        Universal = 0,
        Web = 1,
        Images = 2,
        Local = 3,
        News = 4,
        Products = 5,
        Video = 6,
    }

    impl Type {
        /// String value of the enum field names used in the ProtoBuf definition.
        ///
        /// The values are not transformed in any way and thus are considered stable
        /// (if the ProtoBuf definition does not change) and safe for programmatic use.
        pub fn to_str_name(self) -> &'static str {
            match self {
                Type::Universal => "UNIVERSAL",
                Type::Web => "WEB",
                Type::Images => "IMAGES",
                Type::Local => "LOCAL",
                Type::News => "NEWS",
                Type::Products => "PRODUCTS",
                Type::Video => "VIDEO",
            }
        }
        pub fn from_i32(value: i32) -> ::std::option::Option<Self> {
            match value {
                0 => Some(Type::Universal),
                1 => Some(Type::Web),
                2 => Some(Type::Images),
                3 => Some(Type::Local),
                4 => Some(Type::News),
                5 => Some(Type::Products),
                6 => Some(Type::Video),
                _ => ::std::option::Option::None,
            }
        }
    }

    #[derive(Clone, PartialEq, Debug)]
    pub enum Result {
        Success(super::ResponseResult),
        ServiceError(i64),
        InvalidRequest(i64),
        ErrorMessage(::ntex_grpc::ByteString),
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct ResponseResult {
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
            )
        }
    }

    impl ::std::default::Default for HelloRequest {
        #[inline]
        fn default() -> Self {
            Self {
                name: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::Message for HelloReply {
        #[inline]
        fn write(&self, dst: &mut ::ntex_grpc::BytesMut) {
            ::ntex_grpc::NativeType::serialize(
                &self.metadata,
                5,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.reply_type,
                6,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.result,
                0,
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
                    5 => ::ntex_grpc::NativeType::deserialize(
                        &mut msg.metadata,
                        tag,
                        wire_type,
                        src,
                    )
                    .map_err(|err| err.push(STRUCT_NAME, "metadata"))?,
                    6 => ::ntex_grpc::NativeType::deserialize(
                        &mut msg.reply_type,
                        tag,
                        wire_type,
                        src,
                    )
                    .map_err(|err| err.push(STRUCT_NAME, "reply_type"))?,
                    1 | 2 | 3 | 4 => {
                        ::ntex_grpc::NativeType::deserialize(&mut msg.result, tag, wire_type, src)
                            .map_err(|err| err.push(STRUCT_NAME, "result"))?
                    }
                    _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + ::ntex_grpc::NativeType::serialized_len(
                &self.metadata,
                5,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.reply_type,
                6,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.result,
                0,
                ::ntex_grpc::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for HelloReply {
        #[inline]
        fn default() -> Self {
            Self {
                metadata: ::core::default::Default::default(),
                reply_type: ::core::default::Default::default(),
                result: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::NativeType for hello_reply::Type {
        const TYPE: ::ntex_grpc::WireType = ::ntex_grpc::WireType::Varint;

        #[inline]
        fn merge(
            &mut self,
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<(), ::ntex_grpc::DecodeError> {
            *self = ::ntex_grpc::encoding::decode_varint(src)
                .map(|val| Self::from_i32(val as i32).unwrap_or_default())?;
            Ok(())
        }

        #[inline]
        fn encode_value(&self, dst: &mut ::ntex_grpc::BytesMut) {
            ::ntex_grpc::encoding::encode_varint(*self as i32 as u64, dst);
        }

        #[inline]
        fn encoded_len(&self, tag: u32) -> usize {
            ::ntex_grpc::encoding::key_len(tag)
                + ::ntex_grpc::encoding::encoded_len_varint(*self as i32 as u64)
        }

        #[inline]
        fn is_default(&self) -> bool {
            self == &hello_reply::Type::Universal
        }
    }

    impl ::std::default::Default for hello_reply::Type {
        #[inline]
        fn default() -> Self {
            hello_reply::Type::Universal
        }
    }

    impl ::ntex_grpc::NativeType for hello_reply::Result {
        const TYPE: ::ntex_grpc::WireType = ::ntex_grpc::WireType::LengthDelimited;

        fn merge(
            &mut self,
            _: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<(), ::ntex_grpc::DecodeError> {
            panic!("Not supported")
        }

        fn encode_value(&self, _: &mut ::ntex_grpc::BytesMut) {
            panic!("Not supported")
        }

        #[inline]
        /// Encodes the message to a buffer.
        fn serialize(
            &self,
            _: u32,
            _: ::ntex_grpc::types::DefaultValue<&Self>,
            dst: &mut ::ntex_grpc::BytesMut,
        ) {
            match *self {
                hello_reply::Result::Success(ref value) => ::ntex_grpc::NativeType::serialize(
                    value,
                    1,
                    ::ntex_grpc::types::DefaultValue::Unknown,
                    dst,
                ),
                hello_reply::Result::ServiceError(ref value) => {
                    ::ntex_grpc::NativeType::serialize(
                        value,
                        2,
                        ::ntex_grpc::types::DefaultValue::Unknown,
                        dst,
                    )
                }
                hello_reply::Result::InvalidRequest(ref value) => {
                    ::ntex_grpc::NativeType::serialize(
                        value,
                        3,
                        ::ntex_grpc::types::DefaultValue::Unknown,
                        dst,
                    )
                }
                hello_reply::Result::ErrorMessage(ref value) => {
                    ::ntex_grpc::NativeType::serialize(
                        value,
                        4,
                        ::ntex_grpc::types::DefaultValue::Unknown,
                        dst,
                    )
                }
            }
        }

        #[inline]
        /// Decodes an instance of the message from a buffer, and merges it into self.
        fn deserialize(
            &mut self,
            tag: u32,
            wire_type: ::ntex_grpc::WireType,
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<(), ::ntex_grpc::DecodeError> {
            *self = match tag {
                1 => hello_reply::Result::Success(::ntex_grpc::NativeType::deserialize_default(
                    1, wire_type, src,
                )?),
                2 => hello_reply::Result::ServiceError(
                    ::ntex_grpc::NativeType::deserialize_default(2, wire_type, src)?,
                ),
                3 => hello_reply::Result::InvalidRequest(
                    ::ntex_grpc::NativeType::deserialize_default(3, wire_type, src)?,
                ),
                4 => hello_reply::Result::ErrorMessage(
                    ::ntex_grpc::NativeType::deserialize_default(4, wire_type, src)?,
                ),
                _ => unreachable!("invalid Result, tag: {}", tag),
            };
            Ok(())
        }

        #[inline]
        /// Returns the encoded length of the message without a length delimiter.
        fn serialized_len(&self, _: u32, _: ::ntex_grpc::types::DefaultValue<&Self>) -> usize {
            match *self {
                hello_reply::Result::Success(ref value) => {
                    ::ntex_grpc::NativeType::serialized_len(
                        value,
                        1,
                        ::ntex_grpc::types::DefaultValue::Default,
                    )
                }
                hello_reply::Result::ServiceError(ref value) => {
                    ::ntex_grpc::NativeType::serialized_len(
                        value,
                        2,
                        ::ntex_grpc::types::DefaultValue::Default,
                    )
                }
                hello_reply::Result::InvalidRequest(ref value) => {
                    ::ntex_grpc::NativeType::serialized_len(
                        value,
                        3,
                        ::ntex_grpc::types::DefaultValue::Default,
                    )
                }
                hello_reply::Result::ErrorMessage(ref value) => {
                    ::ntex_grpc::NativeType::serialized_len(
                        value,
                        4,
                        ::ntex_grpc::types::DefaultValue::Default,
                    )
                }
            }
        }
    }

    impl ::std::default::Default for hello_reply::Result {
        #[inline]
        fn default() -> Self {
            hello_reply::Result::Success(::std::default::Default::default())
        }
    }

    impl ::ntex_grpc::Message for ResponseResult {
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
            const STRUCT_NAME: &str = "ResponseResult";
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

    impl ::std::default::Default for ResponseResult {
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
