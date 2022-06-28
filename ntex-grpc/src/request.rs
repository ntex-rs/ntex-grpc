use std::{fmt, future::Future, ops, pin::Pin, task::Context, task::Poll};

use ntex_http::HeaderMap;
use ntex_util::ready;

use crate::service::{MethodDef, Response as TransportResponse, Transport};

pub struct Request<'a, T: Transport<M>, M: MethodDef> {
    transport: &'a T,
    state: State<'a, M, T::Error>,
}

enum State<'a, M: MethodDef, E> {
    Request(&'a M::Input),
    #[allow(clippy::type_complexity)]
    Call(Pin<Box<dyn Future<Output = Result<TransportResponse<M>, E>> + 'a>>),
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
    type Output = Result<Response<M::Output>, T::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut slf = self.as_mut();

        if let State::Call(ref mut fut) = slf.state {
            let response = ready!(Pin::new(fut).poll(cx))?;
            return Poll::Ready(Ok(Response {
                message: response.data,
                headers: response.headers,
                trailers: response.trailers,
            }));
        }
        match std::mem::replace(&mut slf.state, State::Done) {
            State::Request(input) => {
                slf.state = State::Call(slf.transport.request(input));
                self.poll(cx)
            }
            _ => panic!("Future cannot be polled after completion"),
        }
    }
}

pub struct Response<T> {
    message: T,
    headers: HeaderMap,
    trailers: HeaderMap,
}

impl<T> Response<T> {
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    #[inline]
    pub fn trailers(&self) -> &HeaderMap {
        &self.trailers
    }

    #[inline]
    pub fn into_inner(self) -> T {
        self.message
    }

    #[inline]
    pub fn into_parts(self) -> (T, HeaderMap, HeaderMap) {
        (self.message, self.headers, self.trailers)
    }
}

impl<T> ops::Deref for Response<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

impl<T> ops::DerefMut for Response<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.message
    }
}

impl<T: fmt::Debug> fmt::Debug for Response<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}
