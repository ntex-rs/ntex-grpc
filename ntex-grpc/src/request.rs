use std::{future::Future, pin::Pin, task::Context, task::Poll};

use ntex::{http::HeaderMap, util::ready};

use crate::service::{MethodDef, Transport};

pub struct Request<'a, T: Transport, M: MethodDef> {
    transport: &'a T,
    state: State<'a, M, T::Error>,
}

enum State<'a, M: MethodDef, E> {
    Request(M::Input),
    Call(Pin<Box<dyn Future<Output = Result<(M::Output, HeaderMap), E>> + 'a>>),
    Done,
}

impl<'a, M: MethodDef, E> Unpin for State<'a, M, E> {}

impl<'a, T, M> Request<'a, T, M>
where
    T: Transport,
    M: MethodDef,
{
    pub fn new(transport: &'a T, input: M::Input) -> Self {
        Self {
            transport,
            state: State::Request(input),
        }
    }
}

impl<'a, T, M: 'a> Future for Request<'a, T, M>
where
    T: Transport,
    M: MethodDef,
{
    type Output = Result<Response<M::Output>, T::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut slf = self.as_mut();

        if let State::Call(ref mut fut) = slf.state {
            let (message, trailers) = ready!(Pin::new(fut).poll(cx))?;
            return Poll::Ready(Ok(Response {
                message,
                metadata: trailers,
            }));
        }
        match std::mem::replace(&mut slf.state, State::Done) {
            State::Request(input) => {
                slf.state = State::Call(slf.transport.request::<M>(input));
                self.poll(cx)
            }
            _ => panic!("Future cannot be polled after completion"),
        }
    }
}

pub struct Response<T> {
    message: T,
    metadata: HeaderMap,
}

impl<T> Response<T> {
    #[inline]
    pub fn into_inner(self) -> T {
        self.message
    }

    #[inline]
    pub fn into_parts(self) -> (T, HeaderMap) {
        (self.message, self.metadata)
    }
}
