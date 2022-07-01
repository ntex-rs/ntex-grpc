use std::{fmt, future::Future, ops, pin::Pin, task::Context, task::Poll};

use ntex_http::HeaderMap;

use crate::service::{MethodDef, Transport};

pub struct Request<'a, T: Transport<M>, M: MethodDef> {
    transport: &'a T,
    state: State<'a, M, T::Error>,
}

enum State<'a, M: MethodDef, E> {
    Request(&'a M::Input),
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
            state: State::Request(input),
        }
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
                State::Request(input) => {
                    slf.state = State::Call(slf.transport.request(input));
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
