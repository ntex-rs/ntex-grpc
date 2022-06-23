/// DO NOT MODIFY. Auto-generated file

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
pub mod greeter_client {
    use super::*;
    use ntex_grpc::codegen as __ng;
    #[doc = " The greeting service definition."]
    #[derive(Clone)]
    pub struct Greeter<T>(T);
    impl<T> __ng::ClientInformation<T> for Greeter<T> {
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
    pub struct SayHelloDef;
    impl __ng::MethodDef for SayHelloDef {
        const NAME: &'static str = "SayHello";
        const PATH: __ng::ByteString =
            __ng::ByteString::from_static("/helloworld.Greeter/SayHello");
        type Input = HelloRequest;
        type Output = HelloReply;
    }
    impl<T: __ng::Transport<SayHelloDef>> Greeter<T> {
        #[doc = " Sends a greeting"]
        pub fn say_hello<'a>(
            &'a self,
            req: &'a HelloRequest,
        ) -> __ng::Request<'a, T, SayHelloDef> {
            __ng::Request::new(&self.0, req)
        }
    }
}
