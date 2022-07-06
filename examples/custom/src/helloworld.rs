#![allow(dead_code, clippy::identity_op, clippy::derivable_impls)]
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
pub struct Greeter;
impl ::ntex_grpc::ServiceDef for Greeter {
    const NAME: &'static str = "helloworld.Greeter";
    type Methods = GreeterMethods;
}
pub enum GreeterMethods {
    SayHello(GreeterSayHelloMethod),
}
impl ::ntex_grpc::MethodsDef for GreeterMethods {
    #[inline]
    fn by_name(name: &str) -> Option<Self> {
        use ::ntex_grpc::MethodDef;
        match name {
            GreeterSayHelloMethod::NAME => Some(GreeterMethods::SayHello(GreeterSayHelloMethod)),
            _ => None,
        }
    }
}
#[doc = " The greeting service definition."]
#[derive(Clone)]
pub struct GreeterClient<T>(T);
impl<T> GreeterClient<T> {
    #[inline]
    #[doc = r" Create new client instance"]
    pub fn new(transport: T) -> Self {
        Self(transport)
    }
}
impl<T> ::ntex_grpc::ClientInformation<T> for GreeterClient<T> {
    #[inline]
    #[doc = r" Create new client instance"]
    fn create(transport: T) -> Self {
        Self(transport)
    }
    #[inline]
    #[doc = r" Get referece to underlying transport"]
    fn transport(&self) -> &T {
        &self.0
    }
    #[inline]
    #[doc = r" Get mut referece to underlying transport"]
    fn transport_mut(&mut self) -> &mut T {
        &mut self.0
    }
    #[inline]
    #[doc = r" Consume client and return inner transport"]
    fn into_inner(self) -> T {
        self.0
    }
}
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct GreeterSayHelloMethod;
impl ::ntex_grpc::MethodDef for GreeterSayHelloMethod {
    const NAME: &'static str = "SayHello";
    const PATH: ::ntex_grpc::types::ByteString =
        ::ntex_grpc::types::ByteString::from_static("/helloworld.Greeter/SayHello");
    type Input = HelloRequest;
    type Output = HelloReply;
}
impl<T: ::ntex_grpc::Transport<GreeterSayHelloMethod>> GreeterClient<T> {
    #[doc = " Sends a greeting"]
    pub fn say_hello<'a>(
        &'a self,
        req: &'a HelloRequest,
    ) -> ::ntex_grpc::Request<'a, T, GreeterSayHelloMethod> {
        ::ntex_grpc::Request::new(&self.0, req)
    }
}

impl ::ntex_grpc::Message for HelloRequest {
    fn write(&self, dst: &mut ::ntex_grpc::BytesMut) {
        ::ntex_grpc::NativeType::serialize(&self.name, 1, None, dst);
        ::ntex_grpc::NativeType::serialize(&self.msg_id, 2, None, dst);
    }

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
                2 => ::ntex_grpc::NativeType::deserialize(&mut msg.msg_id, tag, wire_type, src)
                    .map_err(|err| err.push(STRUCT_NAME, "msg_id"))?,
                _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
            }
        }
        Ok(msg)
    }

    #[inline]
    fn encoded_len(&self) -> usize {
        0 + ::ntex_grpc::NativeType::serialized_len(&self.name, 1, None)
            + ::ntex_grpc::NativeType::serialized_len(&self.msg_id, 2, None)
    }
}

impl ::std::default::Default for HelloRequest {
    fn default() -> Self {
        Self {
            name: ::core::default::Default::default(),
            msg_id: ::core::default::Default::default(),
        }
    }
}

impl ::ntex_grpc::Message for HelloReply {
    fn write(&self, dst: &mut ::ntex_grpc::BytesMut) {
        ::ntex_grpc::NativeType::serialize(&self.message, 1, None, dst);
    }

    fn read(
        src: &mut ::ntex_grpc::Bytes,
    ) -> ::std::result::Result<Self, ::ntex_grpc::DecodeError> {
        const STRUCT_NAME: &str = "HelloReply";
        let mut msg = Self::default();
        while !src.is_empty() {
            let (tag, wire_type) = ::ntex_grpc::encoding::decode_key(src)?;
            match tag {
                1 => ::ntex_grpc::NativeType::deserialize(&mut msg.message, tag, wire_type, src)
                    .map_err(|err| err.push(STRUCT_NAME, "message"))?,
                _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
            }
        }
        Ok(msg)
    }

    #[inline]
    fn encoded_len(&self) -> usize {
        0 + ::ntex_grpc::NativeType::serialized_len(&self.message, 1, None)
    }
}

impl ::std::default::Default for HelloReply {
    fn default() -> Self {
        Self {
            message: ::core::default::Default::default(),
        }
    }
}
