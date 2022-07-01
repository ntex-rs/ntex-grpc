#![allow(dead_code)]
/// DO NOT MODIFY. Auto-generated file

///  The request message containing the user's name.
#[derive(Clone, PartialEq, ::ntex_grpc::Message)]
pub struct HelloRequest {
    #[prost(string, tag = "1")]
    pub name: ::ntex_grpc::types::ByteString,
}
///  The response message containing the greetings
#[derive(Clone, PartialEq, ::ntex_grpc::Message)]
pub struct HelloReply {
    #[prost(int64, tag = "4")]
    pub metadata: i64,
    #[prost(oneof = "hello_reply::Result", tags = "1, 2, 3")]
    pub result: ::core::option::Option<hello_reply::Result>,
}
/// Nested message and enum types in `HelloReply`.
pub mod hello_reply {
    #[derive(Clone, PartialEq, ::ntex_grpc::Oneof)]
    pub enum Result {
        #[prost(message, tag = "1")]
        Success(super::ResponseResult),
        #[prost(int64, tag = "2")]
        ServiceError(i64),
        #[prost(int64, tag = "3")]
        InvalidRequest(i64),
    }
}
#[derive(Clone, PartialEq, ::ntex_grpc::Message)]
pub struct ResponseResult {
    #[prost(string, tag = "1")]
    pub message: ::ntex_grpc::types::ByteString,
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
