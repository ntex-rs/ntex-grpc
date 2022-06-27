#![allow(dead_code)]
/// DO NOT MODIFY. Auto-generated file

///  The request message containing the user's name.
#[derive(Clone, PartialEq, ::ntex_grpc::Message)]
pub struct HelloRequest {
    #[prost(bytes, tag = "1")]
    pub name: ::ntex_grpc::types::Bytes,
}
///  The response message containing the greetings
#[derive(Clone, PartialEq, ::ntex_grpc::Message)]
pub struct HelloReply {
    #[prost(string, tag = "1")]
    pub message: ::ntex_grpc::types::ByteString,
}

/// `Greeter` service client definition
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
