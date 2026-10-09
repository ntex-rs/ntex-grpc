use std::ops;

use ntex_bytes::{BytePages, ByteString, Bytes};
use ntex_http::{HeaderMap, HeaderName, HeaderValue};

mod error;
mod service;

pub use self::error::{MethodResult, ServerError};
pub use self::service::GrpcServer;
pub use crate::GrpcStatus;

/// A call as [`GrpcServer`] receives it, before the message is decoded.
///
/// The service given to `GrpcServer` handles these and answers with a
/// [`ServerResponse`] or a [`ServerError`]. The `#[server]` attribute
/// writes that service for you; implement it by hand only when you need
/// the raw bytes.
#[derive(Debug)]
pub struct ServerRequest {
    /// Method name without the service, e.g. `SayHello`.
    pub name: ByteString,
    /// The encoded request message, without the 5-byte message prefix.
    pub payload: Bytes,
    /// Request headers, plus the trailers if the client sent any.
    pub headers: HeaderMap,
}

/// Successful answer to a [`ServerRequest`].
#[derive(Debug)]
pub struct ServerResponse {
    /// The encoded reply message, the server adds the message prefix.
    pub payload: BytePages,
    /// Extra trailers, sent after `grpc-status`.
    pub headers: Vec<(HeaderName, HeaderValue)>,
}

impl ServerResponse {
    #[inline]
    /// Answer with an encoded message and no extra trailers.
    pub fn new(payload: BytePages) -> ServerResponse {
        ServerResponse::with_headers(payload, Vec::new())
    }

    #[inline]
    /// Answer with an encoded message and extra trailers.
    pub fn with_headers(
        payload: BytePages,
        headers: Vec<(HeaderName, HeaderValue)>,
    ) -> ServerResponse {
        ServerResponse { payload, headers }
    }
}

/// Argument types a `#[server]` method can take.
///
/// A method takes either the request message itself, or [`Request<T>`]
/// when it also needs the headers:
///
/// ```rust,ignore
/// #[server(helloworld::Greeter)]
/// impl GreeterServer {
///     // `req: HelloRequest` works too, if the headers are not needed
///     #[method(SayHello)]
///     async fn say_hello(&self, req: Request<HelloRequest>) -> HelloReply {
///         let id = req.headers.get("x-request-id");
///         // `req.name` is the method name, the message is in `req.message`
///         HelloReply { message: format!("Hello {} ({id:?})", req.message.name).into() }
///     }
/// }
/// ```
pub trait FromRequest<T> {
    /// Build the argument from the decoded request.
    fn from(input: Request<T>) -> Self;
}

/// Decoded request message, with the method name and headers.
///
/// Derefs to the message. Message fields called `name`, `headers` or
/// `message` are hidden behind the fields below, reach them through
/// `req.message`.
pub struct Request<T> {
    /// Method name, e.g. `SayHello`.
    pub name: ByteString,
    /// Request headers.
    pub headers: HeaderMap,
    /// The request message.
    pub message: T,
}

impl<T> FromRequest<T> for T {
    #[inline]
    fn from(input: Request<T>) -> T {
        input.message
    }
}

impl<T> FromRequest<T> for Request<T> {
    #[inline]
    fn from(input: Request<T>) -> Request<T> {
        input
    }
}

impl<T> Request<T> {
    /// Take the request message and drop the headers.
    pub fn into_inner(self) -> T {
        self.message
    }
}

impl<T> ops::Deref for Request<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.message
    }
}

impl<T> ops::DerefMut for Request<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.message
    }
}

/// Reply message plus extra trailers.
///
/// The `#[server]` attribute wraps each method's return value in one and
/// sends `headers` after `grpc-status`. Methods return the plain message,
/// so for now `headers` is always empty there.
pub struct Response<T> {
    /// The reply message.
    pub message: T,
    /// Extra trailers, sent after `grpc-status`.
    pub headers: Vec<(HeaderName, HeaderValue)>,
}

impl<T> Response<T> {
    /// Wrap a message, with no extra trailers.
    pub fn new(message: T) -> Self {
        Self {
            message,
            headers: Vec::new(),
        }
    }
}

impl<T> From<T> for Response<T> {
    fn from(message: T) -> Self {
        Response {
            message,
            headers: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(data: &[u8]) -> BytePages {
        let mut buf = BytePages::default();
        buf.extend_from_slice(data);
        buf
    }

    #[test]
    fn server_response() {
        let res = ServerResponse::new(pages(b"hi"));
        assert_eq!(res.payload.len(), 2);
        assert!(res.headers.is_empty());

        let res = ServerResponse::with_headers(
            pages(b"hi"),
            vec![(
                HeaderName::from_static("x-a"),
                HeaderValue::from_static("1"),
            )],
        );
        assert_eq!(res.headers.len(), 1);
        assert_eq!(res.headers[0].1, HeaderValue::from_static("1"));
        assert!(format!("{res:?}").contains("ServerResponse"));
    }

    #[test]
    fn server_request_debug() {
        let req = ServerRequest {
            name: ByteString::from_static("SayHello"),
            payload: Bytes::from_static(b"x"),
            headers: HeaderMap::new(),
        };
        assert!(format!("{req:?}").contains("SayHello"));
    }

    fn request(message: u32) -> Request<u32> {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-a"),
            HeaderValue::from_static("1"),
        );
        Request {
            name: ByteString::from_static("SayHello"),
            headers,
            message,
        }
    }

    #[test]
    fn from_request_message() {
        // a method that only needs the message gets the message
        assert_eq!(<u32 as FromRequest<u32>>::from(request(5)), 5);
    }

    #[test]
    fn from_request_full() {
        let req = <Request<u32> as FromRequest<u32>>::from(request(6));
        assert_eq!(req.name, "SayHello");
        assert_eq!(
            req.headers.get("x-a").unwrap(),
            &HeaderValue::from_static("1")
        );
        assert_eq!(*req, 6);
        assert_eq!(req.into_inner(), 6);
    }

    #[test]
    fn request_deref_mut() {
        let mut req = request(7);
        *req = 8;
        assert_eq!(req.message, 8);
    }

    #[test]
    fn response_wrapper() {
        let res = Response::new(1_u32);
        assert_eq!(res.message, 1);
        assert!(res.headers.is_empty());

        let res = <Response<u32> as From<u32>>::from(2_u32);
        assert_eq!(res.message, 2);
        assert!(res.headers.is_empty());
    }
}
