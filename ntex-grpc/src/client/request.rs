use std::task::{Context, Poll};
use std::{cell::Cell, convert::TryFrom, fmt, future::Future, mem, ops, pin::Pin, rc::Rc, time};

use ntex_http::{HeaderMap, HeaderName, HeaderValue, error::Error as HttpError};
use ntex_util::future::BoxFuture;

use crate::{client::Transport, consts, service::MethodDef};

pub struct RequestContext(Rc<RequestContextInner>);

bitflags::bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    struct Flags: u8 {
        const DISCONNECT_ON_DROP = 0b0000_0001;
    }
}

struct RequestContextInner {
    err: Option<HttpError>,
    headers: Vec<(HeaderName, HeaderValue)>,
    timeout: Cell<Option<time::Duration>>,
    flags: Cell<Flags>,
}

impl RequestContext {
    /// Create new RequestContext instance
    fn new() -> Self {
        Self(Rc::new(RequestContextInner {
            err: None,
            headers: Vec::new(),
            timeout: Cell::new(None),
            flags: Cell::new(Flags::empty()),
        }))
    }

    /// Get request timeout
    pub fn get_timeout(&self) -> Option<time::Duration> {
        self.0.timeout.get()
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
        self.0.timeout.set(Some(to));
        self.header(consts::GRPC_TIMEOUT, duration_to_grpc_timeout(to));
        self
    }

    /// Disconnect connection on request drop
    pub fn disconnect_on_drop(&mut self) -> &mut Self {
        let mut flags = self.0.flags.get();
        flags.insert(Flags::DISCONNECT_ON_DROP);
        self.0.flags.set(flags);
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
        if let Some(ctx) = ctx(self) {
            match HeaderName::try_from(key) {
                Ok(key) => match HeaderValue::try_from(value) {
                    Ok(value) => ctx.headers.push((key, value)),
                    Err(e) => ctx.err = Some(log_error(e)),
                },
                Err(e) => ctx.err = Some(log_error(e)),
            };
        }
        self
    }

    pub(crate) fn headers(&self) -> &[(HeaderName, HeaderValue)] {
        &self.0.headers
    }

    pub(crate) fn get_disconnect_on_drop(&self) -> bool {
        self.0.flags.get().contains(Flags::DISCONNECT_ON_DROP)
    }
}

impl Clone for RequestContext {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

fn log_error<T: Into<HttpError>>(err: T) -> HttpError {
    let e = err.into();
    log::error!("Error in Grpc Request {e}");
    e
}

fn ctx(slf: &mut RequestContext) -> Option<&mut RequestContextInner> {
    if slf.0.err.is_some() {
        return None;
    }

    if Rc::get_mut(&mut slf.0).is_some() {
        Rc::get_mut(&mut slf.0)
    } else {
        slf.0 = Rc::new(RequestContextInner {
            err: None,
            headers: slf.0.headers.clone(),
            timeout: slf.0.timeout.clone(),
            flags: slf.0.flags.clone(),
        });
        Some(Rc::get_mut(&mut slf.0).unwrap())
    }
}

pin_project_lite::pin_project! {
    pub struct Request<'a, T, M>
    where T: Transport<M>,
          T: 'a,
          M: MethodDef
    {
        transport: &'a T,
        #[pin]
        state: State<'a, T, M>,
    }
}

enum State<'a, T, M>
where
    T: Transport<M> + 'a,
    M: MethodDef,
{
    Call {
        fut: BoxFuture<'a, Result<Response<M>, T::Error>>,
    },
    Request {
        input: &'a M::Input,
        ctx: Option<RequestContext>,
    },
    None,
}

impl<'a, T, M> Request<'a, T, M>
where
    T: Transport<M>,
    M: MethodDef,
{
    pub fn new(transport: &'a T, input: &'a M::Input) -> Self {
        Self {
            transport,
            state: State::Request {
                input,
                ctx: Some(RequestContext::new()),
            },
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
        if let Some(ctx) = parts(&mut self.state) {
            ctx.header(key, value);
        }
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
        if let Some(ctx) = parts(&mut self.state) {
            let to = timeout.into();
            ctx.0.timeout.set(Some(to));
            ctx.header(consts::GRPC_TIMEOUT, duration_to_grpc_timeout(to));
        }
        self
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

#[inline]
fn parts<'a, 'b, T: Transport<M> + 'a, M: MethodDef>(
    parts: &'b mut State<'a, T, M>,
) -> Option<&'b mut RequestContext> {
    if let State::Request { ctx, .. } = parts {
        ctx.as_mut()
    } else {
        None
    }
}

impl<'a, T, M: 'a> Future for Request<'a, T, M>
where
    T: Transport<M>,
    M: MethodDef,
{
    type Output = Result<Response<M>, T::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            if let State::Call { ref mut fut } = self.state {
                return Pin::new(fut).poll(cx);
            }

            if let State::Request { input, ref mut ctx } =
                mem::replace(&mut self.state, State::None)
            {
                self.state = State::Call {
                    fut: Box::pin(self.transport.request(input, ctx.take().unwrap())),
                };
            }
        }
    }
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
        let timeout = Duration::from_millis(500);
        let value = duration_to_grpc_timeout(timeout);
        assert_eq!(value, format!("{}u", timeout.as_micros()));

        let timeout = Duration::from_secs(30);
        let value = duration_to_grpc_timeout(timeout);
        assert_eq!(value, format!("{}u", timeout.as_micros()));

        let one_hour = Duration::from_secs(60 * 60);
        let value = duration_to_grpc_timeout(one_hour);
        assert_eq!(value, format!("{}m", one_hour.as_millis()));
    }
}
