#![allow(
    dead_code,
    unused_mut,
    unused_variables,
    clippy::identity_op,
    clippy::derivable_impls,
    clippy::unit_arg,
    clippy::derive_partial_eq_without_eq,
    clippy::manual_range_patterns
)]
/// DO NOT MODIFY. Auto-generated file

///  The request message containing the user's name.
#[derive(Clone, PartialEq, Debug)]
pub struct HelloRequest {
    pub name: ::ntex_grpc::ByteString,
    pub data1: Vec<i64>,
    pub data2: Vec<DocumentType>,
}

///  The response message containing the greetings
#[derive(Clone, PartialEq, Debug)]
pub struct HelloReply {
    pub data5: Vec<DocumentType>,
    pub data6: Vec<i64>,
    pub message: ::ntex_grpc::ByteString,
    pub tag: u32,
    pub data1: Vec<u32>,
    pub data2: Vec<::ntex_grpc::ByteString>,
    pub data3: ::ntex_grpc::HashMap<::ntex_grpc::ByteString, u32>,
    pub data4: DocumentType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(i32)]
pub enum DocumentType {
    Broker = 0,
    Namespace = 1,
    TestDoc = 2,
    Quota = 6,
}

impl DocumentType {
    /// String value of the enum field names used in the ProtoBuf definition with stripped prefix.
    pub fn to_str_name(self) -> &'static str {
        match self {
            DocumentType::Broker => "BROKER",
            DocumentType::Namespace => "NAMESPACE",
            DocumentType::TestDoc => "TEST_DOC",
            DocumentType::Quota => "QUOTA",
        }
    }

    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn to_origin_name(self) -> &'static str {
        match self {
            DocumentType::Broker => "BROKER",
            DocumentType::Namespace => "NAMESPACE",
            DocumentType::TestDoc => "TEST_DOC",
            DocumentType::Quota => "QUOTA",
        }
    }

    pub fn from_i32(value: i32) -> ::std::option::Option<Self> {
        match value {
            0 => Some(DocumentType::Broker),
            1 => Some(DocumentType::Namespace),
            2 => Some(DocumentType::TestDoc),
            6 => Some(DocumentType::Quota),
            _ => ::std::option::Option::None,
        }
    }
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

mod _priv_impl_helloworld {
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
                &self.data1,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.data2,
                3,
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
                    2 => ::ntex_grpc::NativeType::deserialize(&mut msg.data1, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "data1"))?,
                    3 => ::ntex_grpc::NativeType::deserialize(&mut msg.data2, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "data2"))?,
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
                &self.data1,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.data2,
                3,
                ::ntex_grpc::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for HelloRequest {
        #[inline]
        fn default() -> Self {
            Self {
                name: ::core::default::Default::default(),
                data1: ::core::default::Default::default(),
                data2: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::Message for HelloReply {
        #[inline]
        fn write(&self, dst: &mut ::ntex_grpc::BytesMut) {
            ::ntex_grpc::NativeType::serialize(
                &self.data5,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.data6,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.message,
                3,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.tag,
                4,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.data1,
                5,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.data2,
                6,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.data3,
                7,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.data4,
                8,
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
                    1 => ::ntex_grpc::NativeType::deserialize(&mut msg.data5, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "data5"))?,
                    2 => ::ntex_grpc::NativeType::deserialize(&mut msg.data6, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "data6"))?,
                    3 => {
                        ::ntex_grpc::NativeType::deserialize(&mut msg.message, tag, wire_type, src)
                            .map_err(|err| err.push(STRUCT_NAME, "message"))?
                    }
                    4 => ::ntex_grpc::NativeType::deserialize(&mut msg.tag, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "tag"))?,
                    5 => ::ntex_grpc::NativeType::deserialize(&mut msg.data1, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "data1"))?,
                    6 => ::ntex_grpc::NativeType::deserialize(&mut msg.data2, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "data2"))?,
                    7 => ::ntex_grpc::NativeType::deserialize(&mut msg.data3, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "data3"))?,
                    8 => ::ntex_grpc::NativeType::deserialize(&mut msg.data4, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "data4"))?,
                    _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + ::ntex_grpc::NativeType::serialized_len(
                &self.data5,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.data6,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.message,
                3,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.tag,
                4,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.data1,
                5,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.data2,
                6,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.data3,
                7,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.data4,
                8,
                ::ntex_grpc::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for HelloReply {
        #[inline]
        fn default() -> Self {
            Self {
                data5: ::core::default::Default::default(),
                data6: ::core::default::Default::default(),
                message: ::core::default::Default::default(),
                tag: ::core::default::Default::default(),
                data1: ::core::default::Default::default(),
                data2: ::core::default::Default::default(),
                data3: ::core::default::Default::default(),
                data4: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::NativeType for DocumentType {
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
        fn value_len(&self) -> usize {
            ::ntex_grpc::encoding::encoded_len_varint(*self as i32 as u64)
        }

        #[inline]
        fn is_default(&self) -> bool {
            self == &DocumentType::Broker
        }
    }

    impl ::std::default::Default for DocumentType {
        #[inline]
        fn default() -> Self {
            DocumentType::Broker
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
