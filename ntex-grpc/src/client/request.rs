use std::{convert::TryFrom, fmt, ops, time};

use ntex_http::{HeaderMap, HeaderName, HeaderValue, error::Error as HttpError};
use ntex_util::HashMap;

use crate::{client::Transport, consts, service::MethodDef};

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
    /// The duration will be formatted according to [the spec] and use the most precise
    /// possible.
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

    /// Append a header to existing headers.
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
                Err(e) => self.err = Some(log_error(e)),
            },
            Err(e) => self.err = Some(log_error(e)),
        }
        self
    }

    /// Clear existing headers.
    pub fn clear(&mut self) -> &mut Self {
        self.headers.clear();
        self
    }

    pub(crate) fn headers(&self) -> &HashMap<HeaderName, HeaderValue> {
        &self.headers
    }

    pub(crate) fn get_disconnect_on_drop(&self) -> bool {
        self.flags.contains(Flags::DISCONNECT_ON_DROP)
    }
}

fn log_error<T: Into<HttpError>>(err: T) -> HttpError {
    let e = err.into();
    log::error!("Error in Grpc Request {e}");
    e
}

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
    pub fn new(transport: &'a T, input: &'a M::Input) -> Self {
        Self {
            input,
            transport,
            ctx: RequestContext::new(),
        }
    }

    /// Append a header to existing headers.
    ///
    /// ```rust
    /// use ntex::http::{header, Request, Response};
    ///
    /// fn index(req: Request) -> Response {
    ///     Response::Ok()
    ///         .header("X-TEST", "value")
    ///         .header(header::CONTENT_TYPE, "application/json")
    ///         .finish()
    /// }
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
    /// The duration will be formatted according to [the spec] and use the most precise
    /// possible.
    ///
    /// [the spec]: https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
    pub fn timeout<U>(&mut self, timeout: U) -> &mut Self
    where
        time::Duration: From<U>,
    {
        let to = timeout.into();
        self.ctx.timeout = Some(to);
        self.ctx
            .header(consts::GRPC_TIMEOUT, duration_to_grpc_timeout(to));
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

pub struct Response<T: MethodDef> {
    pub output: T::Output,
    pub headers: HeaderMap,
    pub trailers: HeaderMap,
    pub req_size: usize,
    pub res_size: usize,
}

impl<T: MethodDef> Response<T> {
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    #[inline]
    pub fn trailers(&self) -> &HeaderMap {
        &self.trailers
    }

    #[inline]
    pub fn into_inner(self) -> T::Output {
        self.output
    }

    #[inline]
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
            .field("translers", &self.headers)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_to_grpc_timeout_less_than_second() {
        let timeout = time::Duration::from_millis(500);
        let value = duration_to_grpc_timeout(timeout);
        assert_eq!(value, format!("{}u", timeout.as_micros()));

        let timeout = time::Duration::from_secs(30);
        let value = duration_to_grpc_timeout(timeout);
        assert_eq!(value, format!("{}u", timeout.as_micros()));

        let one_hour = time::Duration::from_secs(60 * 60);
        let value = duration_to_grpc_timeout(one_hour);
        assert_eq!(value, format!("{}m", one_hour.as_millis()));
    }
}
