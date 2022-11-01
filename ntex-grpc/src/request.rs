use std::task::{Context, Poll};
use std::{convert::TryFrom, fmt, future::Future, ops, pin::Pin, rc::Rc};

use ntex_http::{error::Error as HttpError, HeaderMap, HeaderName, HeaderValue};

use crate::service::{MethodDef, Transport};

pub struct RequestContext(Rc<RequestContextInner>);

struct RequestContextInner {
    err: Option<HttpError>,
    headers: Vec<(HeaderName, HeaderValue)>,
}

impl RequestContext {
    /// Create new RequestContext instance
    fn new() -> Self {
        Self(Rc::new(RequestContextInner {
            err: None,
            headers: Vec::new(),
        }))
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
}

impl Clone for RequestContext {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

fn log_error<T: Into<HttpError>>(err: T) -> HttpError {
    let e = err.into();
    log::error!("Error in Grpc Request {}", e);
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
        });
        Some(Rc::get_mut(&mut slf.0).unwrap())
    }
}

pub struct Request<'a, T: Transport<M>, M: MethodDef> {
    transport: &'a T,
    state: State<'a, M, T::Error>,
}

enum State<'a, M: MethodDef, E> {
    Request(&'a M::Input, RequestContext),
    #[allow(clippy::type_complexity)]
    Call(Pin<Box<dyn Future<Output = Result<Response<M>, E>> + 'a>>),
    Done,
}

impl<'a, M: MethodDef, E> Unpin for State<'a, M, E> {}

impl<'a, T, M> Request<'a, T, M>
where
    T: Transport<M>,
    M: MethodDef,
{
    pub fn new(transport: &'a T, input: &'a M::Input) -> Self {
        Self {
            transport,
            state: State::Request(input, RequestContext::new()),
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
}

#[inline]
fn parts<'a, 'b, M: MethodDef, E>(
    parts: &'b mut State<'a, M, E>,
) -> Option<&'b mut RequestContext> {
    if let State::Request(_, ref mut ctx) = parts {
        Some(ctx)
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
        let mut slf = self.as_mut();

        if let State::Call(ref mut fut) = slf.state {
            Pin::new(fut).poll(cx)
        } else {
            match std::mem::replace(&mut slf.state, State::Done) {
                State::Request(input, ctx) => {
                    slf.state = State::Call(slf.transport.request(input, ctx));
                    self.poll(cx)
                }
                _ => panic!("Future cannot be polled after completion"),
            }
        }
    }
}

pub struct Response<T: MethodDef> {
    pub output: T::Output,
    pub headers: HeaderMap,
    pub trailers: HeaderMap,
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
