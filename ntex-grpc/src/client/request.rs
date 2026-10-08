use std::{convert::TryFrom, fmt, ops, time};

use ntex_http::{HeaderMap, HeaderName, HeaderValue, error::Error as HttpError};
use ntex_util::HashMap;

use crate::{client::Transport, consts, service::MethodDef};

/// Headers, timeout and flags of a single call.
///
/// [`Request`] collects them and passes them to
/// [`Transport::request()`]. A custom transport that wraps a built-in one
/// passes the context on unchanged.
#[derive(Debug)]
pub struct RequestContext {
    err: Option<HttpError>,
    headers: HashMap<HeaderName, HeaderValue>,
    timeout: Option<time::Duration>,
    flags: Flags,
}

bitflags::bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    struct Flags: u8 {
        const DISCONNECT_ON_DROP = 0b0000_0001;
    }
}

impl RequestContext {
    /// Create new `RequestContext` instance
    fn new() -> Self {
        Self {
            err: None,
            headers: HashMap::default(),
            timeout: None,
            flags: Flags::empty(),
        }
    }

    /// Get request timeout
    pub fn get_timeout(&self) -> Option<time::Duration> {
        self.timeout
    }

    /// Set the max duration the request is allowed to take.
    ///
    /// The duration is sent in the `grpc-timeout` header, formatted according
    /// to [the spec] with the most precise unit that fits.
    ///
    /// [the spec]: https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
    pub fn timeout<U>(&mut self, timeout: U) -> &mut Self
    where
        time::Duration: From<U>,
    {
        let to = timeout.into();
        self.timeout = Some(to);
        self.header(consts::GRPC_TIMEOUT, duration_to_grpc_timeout(to));
        self
    }

    /// Disconnect connection on request drop
    pub fn disconnect_on_drop(&mut self) -> &mut Self {
        self.flags.insert(Flags::DISCONNECT_ON_DROP);
        self
    }

    /// Set a request header, replacing any existing value for the same name.
    ///
    /// An invalid name or value is not sent; the error is kept and returned
    /// by [`take_error()`](Self::take_error).
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: TryFrom<K>,
        HeaderValue: TryFrom<V>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        <HeaderValue as TryFrom<V>>::Error: Into<HttpError>,
    {
        match HeaderName::try_from(key) {
            Ok(key) => match HeaderValue::try_from(value) {
                Ok(value) => {
                    self.headers.insert(key, value);
                }
                Err(e) => self.set_error(e),
            },
            Err(e) => self.set_error(e),
        }
        self
    }

    fn set_error<T: Into<HttpError>>(&mut self, err: T) {
        if self.err.is_none() {
            self.err = Some(err.into());
        }
    }

    /// Take the first error recorded while building the request.
    ///
    /// [`header()`](Self::header) does not fail on an invalid name or value, it
    /// remembers the error instead. Built-in transports return it as
    /// [`ClientError::Http`](super::ClientError::Http) before sending anything,
    /// custom transports should do the same.
    pub fn take_error(&mut self) -> Option<HttpError> {
        self.err.take()
    }

    /// Clear existing headers and timeout.
    ///
    /// The timeout is sent as the `grpc-timeout` header, so it is removed too.
    pub fn clear(&mut self) -> &mut Self {
        self.headers.clear();
        self.timeout = None;
        self
    }

    /// Headers set for the call, including `grpc-timeout`.
    ///
    /// A custom transport sends them with the request.
    pub fn headers(&self) -> impl ExactSizeIterator<Item = (&HeaderName, &HeaderValue)> {
        self.headers.iter()
    }

    /// Check if the connection must be closed when the request is dropped.
    pub fn get_disconnect_on_drop(&self) -> bool {
        self.flags.contains(Flags::DISCONNECT_ON_DROP)
    }
}

/// A call that is ready to be sent.
///
/// Generated client methods return it. Set headers or a timeout if you need
/// them, then [`send()`](Self::send) it. The request borrows its message, so
/// keep the message in a variable rather than building it inline.
pub struct Request<'a, T, M>
where
    T: Transport<M>,
    T: 'a,
    M: MethodDef,
{
    input: &'a M::Input,
    transport: &'a T,
    ctx: RequestContext,
}

impl<'a, T, M> Request<'a, T, M>
where
    T: Transport<M>,
    M: MethodDef,
{
    /// Create a call that sends `input` through `transport`.
    ///
    /// Generated clients do this for you.
    pub fn new(transport: &'a T, input: &'a M::Input) -> Self {
        Self {
            input,
            transport,
            ctx: RequestContext::new(),
        }
    }

    /// Set a request header, replacing any existing value for the same name.
    ///
    /// An invalid name or value does not panic, [`send()`](Self::send) returns
    /// the error instead.
    ///
    /// ```rust,ignore
    /// // `GreeterClient` and `HelloRequest` are generated from a .proto file
    /// let msg = HelloRequest { name: "world".into(), ..Default::default() };
    ///
    /// let mut req = client.say_hello(&msg);
    /// req.header("x-request-id", "42");
    /// let res = req.send().await?;
    /// ```
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: TryFrom<K>,
        HeaderValue: TryFrom<V>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        <HeaderValue as TryFrom<V>>::Error: Into<HttpError>,
    {
        self.ctx.header(key, value);
        self
    }

    /// Set the max duration the request is allowed to take.
    ///
    /// The duration is sent in the `grpc-timeout` header, formatted according
    /// to [the spec] with the most precise unit that fits.
    ///
    /// [the spec]: https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
    pub fn timeout<U>(&mut self, timeout: U) -> &mut Self
    where
        time::Duration: From<U>,
    {
        self.ctx.timeout(timeout);
        self
    }

    /// Send request
    pub async fn send(self) -> Result<Response<M>, T::Error> {
        let Request {
            input,
            transport,
            mut ctx,
        } = self;

        transport.request(input, &mut ctx).await
    }
}

fn duration_to_grpc_timeout(duration: time::Duration) -> String {
    fn try_format<T: Into<u128>>(
        duration: time::Duration,
        unit: char,
        convert: impl FnOnce(time::Duration) -> T,
    ) -> Option<String> {
        // The gRPC spec specifies that the timeout most be at most 8 digits. So this is the largest a
        // value can be before we need to use a bigger unit.
        let max_size: u128 = 99_999_999; // exactly 8 digits

        let value = convert(duration).into();
        if value > max_size {
            None
        } else {
            Some(format!("{value}{unit}"))
        }
    }

    // pick the most precise unit that is less than or equal to 8 digits as per the gRPC spec
    try_format(duration, 'n', |d| d.as_nanos())
        .or_else(|| try_format(duration, 'u', |d| d.as_micros()))
        .or_else(|| try_format(duration, 'm', |d| d.as_millis()))
        .or_else(|| try_format(duration, 'S', |d| d.as_secs()))
        .or_else(|| try_format(duration, 'M', |d| d.as_secs() / 60))
        .or_else(|| {
            try_format(duration, 'H', |d| {
                let minutes = d.as_secs() / 60;
                minutes / 60
            })
        })
        // duration has to be more than 11_415 years for this to happen
        .expect("duration is unrealistically large")
}

/// Successful reply to a call.
///
/// Derefs to the reply message, so its fields can be used directly.
pub struct Response<T: MethodDef> {
    /// The reply message.
    pub output: T::Output,
    /// Response headers.
    pub headers: HeaderMap,
    /// Trailers sent after the message, `grpc-status` is one of them.
    pub trailers: HeaderMap,
    /// Size of the request body in bytes, with the 5-byte message prefix.
    pub req_size: usize,
    /// Size of the response body in bytes, with the 5-byte message prefix.
    pub res_size: usize,
}

impl<T: MethodDef> Response<T> {
    #[inline]
    /// Response headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    #[inline]
    /// Trailers sent after the message.
    pub fn trailers(&self) -> &HeaderMap {
        &self.trailers
    }

    #[inline]
    /// Take the reply message and drop the headers.
    pub fn into_inner(self) -> T::Output {
        self.output
    }

    #[inline]
    /// Split into the reply message, headers and trailers.
    pub fn into_parts(self) -> (T::Output, HeaderMap, HeaderMap) {
        (self.output, self.headers, self.trailers)
    }
}

impl<T: MethodDef> ops::Deref for Response<T> {
    type Target = T::Output;

    fn deref(&self) -> &Self::Target {
        &self.output
    }
}

impl<T: MethodDef> ops::DerefMut for Response<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.output
    }
}

impl<T: MethodDef> fmt::Debug for Response<T>
where
    T::Output: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(format!("ResponseFor<{}>", T::NAME).as_str())
            .field("output", &self.output)
            .field("headers", &self.headers)
            .field("trailers", &self.trailers)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_header_and_clear() {
        let mut ctx = RequestContext::new();
        ctx.header("x-a", "1").header("x-a", "2");
        let hdrs: Vec<_> = ctx.headers().collect();
        assert_eq!(
            hdrs,
            [(
                &HeaderName::from_static("x-a"),
                &HeaderValue::from_static("2")
            )]
        );

        ctx.timeout(time::Duration::from_secs(1));
        assert_eq!(ctx.headers().len(), 2);
        assert!(
            ctx.headers()
                .any(|(k, v)| k == consts::GRPC_TIMEOUT && v == "1000000u")
        );
        ctx.clear();
        assert_eq!(ctx.headers().len(), 0);
        assert_eq!(ctx.get_timeout(), None);
    }

    #[test]
    fn context_disconnect_on_drop() {
        let mut ctx = RequestContext::new();
        assert!(!ctx.get_disconnect_on_drop());
        ctx.disconnect_on_drop();
        assert!(ctx.get_disconnect_on_drop());
    }

    #[test]
    fn duration_to_grpc_timeout_less_than_second() {
        let timeout = time::Duration::from_millis(500);
        let value = duration_to_grpc_timeout(timeout);
        assert_eq!(value, format!("{}u", timeout.as_micros()));

        let timeout = time::Duration::from_secs(30);
        let value = duration_to_grpc_timeout(timeout);
        assert_eq!(value, format!("{}u", timeout.as_micros()));

        let one_hour = time::Duration::from_hours(1);
        let value = duration_to_grpc_timeout(one_hour);
        assert_eq!(value, format!("{}m", one_hour.as_millis()));
    }
}
