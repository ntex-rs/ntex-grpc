/// DO NOT MODIFY. Auto-generated file
use ntex_grpc::codegen as ngrpc;

/// The request message containing the user's name.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct HelloRequest {
    #[prost(string, tag = "1")]
    pub name: ::prost::alloc::string::String,
}
/// The response message containing the greetings
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct HelloReply {
    #[prost(string, tag = "1")]
    pub message: ::prost::alloc::string::String,
}

/// `Greeter` service client definition
#[doc = " The greeting service definition."]
#[derive(Clone)]
pub struct GreeterClient<T>(T);
impl<T> ngrpc::ClientInformation<T> for GreeterClient<T> {
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
impl ngrpc::MethodDef for GreeterSayHelloMethod {
    const NAME: &'static str = "SayHello";
    const PATH: ngrpc::ByteString = ngrpc::ByteString::from_static("/helloworld.Greeter/SayHello");
    type Input = HelloRequest;
    type Output = HelloReply;
}
impl<T: ngrpc::Transport<GreeterSayHelloMethod>> GreeterClient<T> {
    #[doc = " Sends a greeting"]
    pub fn say_hello<'a>(
        &'a self,
        req: &'a HelloRequest,
    ) -> ngrpc::Request<'a, T, GreeterSayHelloMethod> {
        ngrpc::Request::new(&self.0, req)
    }
}
