//! Streams of grpc messages, shared by the client and the server.
use std::cell::{Cell, OnceCell, RefCell};
use std::task::{Context, Poll, Waker};
use std::{collections::VecDeque, fmt, future::Future, future::poll_fn, marker::PhantomData};
use std::{mem, pin::Pin, rc::Rc, time::Instant};

use ntex_bytes::{Buf, BytePages, Bytes};
use ntex_error::Error;
use ntex_h2::{self as h2, Capacity, OperationError, StreamRef, frame::Reason};
use ntex_http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use ntex_util::{Stream, task::LocalWaker, time};

use crate::client::{CallFailure, ClientError, NotGrpc, Status, transport as tr};
use crate::server::ServerError;
use crate::utils::{self, Data, Enc};
use crate::{GrpcStatus, Message, consts};

/// An error of a [`SendStream`] or a [`RecvStream`].
///
/// The client side reports the status of the call as
/// [`StreamError::Client`], the way a unary call reports it.
#[derive(thiserror::Error, Debug, Clone)]
pub enum StreamError {
    /// The stream is closed or cancelled.
    #[error("Stream is closed")]
    Closed,
    /// The peer reset the stream.
    #[error("Stream is reset with {0:?}")]
    Reset(Reason),
    /// The connection is closed.
    #[error("Connection is closed")]
    Disconnected,
    /// A message could not be sent or received, e.g. it is larger than the
    /// limit. Holds the status and the `grpc-message`.
    #[error("Grpc status {0:?}: {1:?}")]
    Status(GrpcStatus, HeaderValue),
    /// The call failed, client side only.
    #[error("{0}")]
    Client(Error<ClientError>),
}

impl StreamError {
    /// The status of the call this error ends.
    pub fn status(&self) -> GrpcStatus {
        match self {
            StreamError::Closed => GrpcStatus::Cancelled,
            StreamError::Reset(r) => GrpcStatus::from(*r),
            StreamError::Disconnected => GrpcStatus::Unavailable,
            StreamError::Status(st, _) => *st,
            StreamError::Client(e) => e.status(),
        }
    }

    fn from_send(err: &Error<OperationError>) -> Self {
        match **err {
            OperationError::RemoteReset(r) | OperationError::Closed(Some(r)) => {
                StreamError::Reset(r)
            }
            OperationError::Connection(_)
            | OperationError::Disconnected
            | OperationError::Disconnecting => StreamError::Disconnected,
            _ => StreamError::Closed,
        }
    }
}

impl From<StreamError> for ServerError {
    fn from(err: StreamError) -> Self {
        let (status, msg) = match err {
            StreamError::Status(st, msg) => (st, msg),
            StreamError::Reset(r) => (
                GrpcStatus::from(r),
                HeaderValue::try_from(format!("Stream reset with {r:?}"))
                    .unwrap_or_else(|_| HeaderValue::from_static("Stream reset")),
            ),
            StreamError::Disconnected => (
                GrpcStatus::Unavailable,
                HeaderValue::from_static("Connection is closed"),
            ),
            StreamError::Closed | StreamError::Client(_) => (
                GrpcStatus::Internal,
                HeaderValue::from_static("Stream is closed"),
            ),
        };
        ServerError::new(status, msg, None)
    }
}

/// Maps an error of the client side of a call.
///
/// A [`StreamError::Status`] is a request message that cannot be sent.
impl From<StreamError> for Error<ClientError> {
    fn from(err: StreamError) -> Self {
        let err = match err {
            StreamError::Client(e) => return e,
            StreamError::Status(st, msg) => {
                ClientError::InvalidRequest(CallFailure::new(st, tr::text(&msg)))
            }
            StreamError::Reset(r) => {
                tr::reset_error(r, HeaderMap::new(), None, h2::StreamError::Reset(r))
            }
            StreamError::Disconnected => ClientError::Connection {
                request_sent: true,
                failure: CallFailure::new(GrpcStatus::Unavailable, "Connection is closed"),
            },
            StreamError::Closed => ClientError::StreamFailed(CallFailure::new(
                GrpcStatus::Cancelled,
                "Stream is closed",
            )),
        };
        Error::from(err)
    }
}

bitflags::bitflags! {
    #[derive(Copy, Clone, Debug)]
    struct Flags: u8 {
        /// No more messages are sent.
        const CLOSED        = 0b0000_0001;
        /// A `send()` is in progress.
        const BUSY          = 0b0000_0010;
        /// A message is being written to the h2 stream.
        const SENDING       = 0b0000_0100;
        /// Half-close once the `send()` in progress completes.
        const CLOSE_PENDING = 0b0000_1000;
        /// The stream is attached to an h2 stream.
        const ATTACHED      = 0b0001_0000;
        /// The h2 stream is reset with `CANCEL`.
        const RESET         = 0b0010_0000;
        /// A client-streaming call has claimed the sender.
        const CLAIMED       = 0b0100_0000;
    }
}

enum Backend {
    Client(h2::client::SendStream),
    Server(StreamRef),
}

impl Backend {
    fn reset(&self, reason: Reason) {
        match self {
            Backend::Client(tx) => {
                tx.reset(reason);
            }
            Backend::Server(stream) => {
                stream.reset(reason);
            }
        }
    }
}

/// How messages are sent, set once the stream is attached.
#[derive(Copy, Clone)]
pub(crate) struct SendConfig {
    pub(crate) max_size: usize,
    pub(crate) enc: Option<Enc>,
    /// The deadline of the call, client side only.
    pub(crate) deadline: Option<Instant>,
    pub(crate) service: &'static str,
}

pub(crate) struct SendShared {
    backend: OnceCell<Backend>,
    flags: Cell<Flags>,
    /// The number of `SendStream` handles, the last one closes the stream.
    handles: Cell<usize>,
    wakers: RefCell<Vec<Waker>>,
    max_size: Cell<usize>,
    enc: Cell<Option<Enc>>,
    deadline: Cell<Option<Instant>>,
    service: Cell<&'static str>,
    /// The response headers, server side only. `None` once sent.
    headers: RefCell<Option<HeaderMap>>,
    trailers: RefCell<HeaderMap>,
    error: RefCell<Option<StreamError>>,
    /// The size of the messages sent, with their prefixes.
    sent: Cell<usize>,
}

impl SendShared {
    fn new() -> Self {
        SendShared {
            backend: OnceCell::new(),
            flags: Cell::new(Flags::empty()),
            handles: Cell::new(0),
            wakers: RefCell::new(Vec::new()),
            max_size: Cell::new(usize::MAX),
            enc: Cell::new(None),
            deadline: Cell::new(None),
            service: Cell::new(""),
            headers: RefCell::new(None),
            trailers: RefCell::new(HeaderMap::new()),
            error: RefCell::new(None),
            sent: Cell::new(0),
        }
    }

    /// The response stream of a server call, attached already.
    pub(crate) fn server(
        stream: StreamRef,
        headers: HeaderMap,
        max_size: usize,
        enc: Option<Enc>,
    ) -> Rc<Self> {
        let shared = SendShared::new();
        *shared.headers.borrow_mut() = Some(headers);
        let _ = shared.attach(
            Backend::Server(stream),
            SendConfig {
                max_size,
                enc,
                deadline: None,
                service: "",
            },
        );
        Rc::new(shared)
    }

    fn insert(&self, f: Flags) {
        let mut flags = self.flags.get();
        flags.insert(f);
        self.flags.set(flags);
    }

    fn remove(&self, f: Flags) {
        let mut flags = self.flags.get();
        flags.remove(f);
        self.flags.set(flags);
    }

    fn wake_all(&self) {
        let wakers = mem::take(&mut *self.wakers.borrow_mut());
        for waker in wakers {
            waker.wake();
        }
    }

    /// Keeps the first error.
    fn set_error(&self, err: StreamError) {
        let mut error = self.error.borrow_mut();
        if error.is_none() {
            *error = Some(err);
        }
    }

    pub(crate) fn is_attached(&self) -> bool {
        self.flags.get().contains(Flags::ATTACHED)
    }

    /// Claims an unattached sender for one client-streaming call.
    pub(crate) fn start_open(&self) -> Option<SendOpenGuard<'_>> {
        if self
            .flags
            .get()
            .intersects(Flags::ATTACHED | Flags::CLAIMED)
        {
            None
        } else {
            self.insert(Flags::CLAIMED);
            Some(SendOpenGuard(self))
        }
    }

    pub(crate) fn sent(&self) -> usize {
        self.sent.get()
    }

    /// Checks if the call is cancelled, the h2 stream is reset by this side.
    pub(crate) fn is_reset(&self) -> bool {
        self.flags.get().contains(Flags::RESET)
    }

    fn reset(&self) {
        if let Some(backend) = self.backend.get() {
            backend.reset(Reason::CANCEL);
            self.insert(Flags::RESET);
        }
    }

    /// Attaches the h2 stream, a stream cancelled before resets it.
    fn attach(&self, backend: Backend, cfg: SendConfig) -> Result<(), StreamError> {
        if let Some(err) = self.error.borrow().clone() {
            backend.reset(Reason::CANCEL);
            return Err(err);
        }
        if self.backend.set(backend).is_err() {
            return Err(StreamError::Closed);
        }
        self.max_size.set(cfg.max_size);
        self.enc.set(cfg.enc);
        self.deadline.set(cfg.deadline);
        self.service.set(cfg.service);
        self.insert(Flags::ATTACHED);
        if self.flags.get().contains(Flags::CLOSED) {
            self.half_close();
        }
        self.wake_all();
        Ok(())
    }

    /// Attaches the request stream of a client call.
    pub(crate) fn attach_client(
        &self,
        tx: h2::client::SendStream,
        cfg: SendConfig,
    ) -> Result<(), StreamError> {
        self.attach(Backend::Client(tx), cfg)
    }

    /// Ends the request stream, the server ends its stream with trailers.
    fn half_close(&self) {
        if let Some(Backend::Client(tx)) = self.backend.get() {
            // empty data with eof is sent without waiting
            let mut cx = Context::from_waker(Waker::noop());
            let fut = std::pin::pin!(tx.send_pages(Bytes::new(), true));
            if let Poll::Ready(Err(err)) = fut.poll(&mut cx) {
                log::trace!("Cannot close grpc stream: {err:?}");
            }
        }
    }

    fn poll_ready(&self, cx: &Context<'_>) -> Poll<Result<(), StreamError>> {
        if let Some(err) = self.error.borrow().as_ref() {
            return Poll::Ready(Err(err.clone()));
        }
        let flags = self.flags.get();
        if flags.contains(Flags::CLOSED) {
            Poll::Ready(Err(StreamError::Closed))
        } else if flags.contains(Flags::ATTACHED) && !flags.contains(Flags::BUSY) {
            Poll::Ready(Ok(()))
        } else {
            let mut wakers = self.wakers.borrow_mut();
            if !wakers.iter().any(|w| w.will_wake(cx.waker())) {
                wakers.push(cx.waker().clone());
            }
            Poll::Pending
        }
    }

    async fn send<T: Message>(&self, msg: &T) -> Result<(), StreamError> {
        poll_fn(|cx| self.poll_ready(cx)).await?;
        self.insert(Flags::BUSY);
        let _guard = SendGuard(self);

        let mut buf = BytePages::default();
        msg.write(&mut buf);
        let buf = utils::frame_message(buf, self.max_size.get(), self.enc.get())
            .await
            .map_err(|(st, msg)| StreamError::Status(st, msg))?;
        let len = buf.len();

        let Some(backend) = self.backend.get() else {
            return Err(StreamError::Closed);
        };
        let res = match backend {
            Backend::Client(tx) => self.send_client(tx, buf).await,
            Backend::Server(stream) => {
                let headers = self.headers.borrow_mut().take();
                if let Some(hdrs) = headers
                    && let Err(err) = send_response_headers(stream, hdrs)
                {
                    Err(StreamError::from_send(&err))
                } else {
                    self.insert(Flags::SENDING);
                    let res = stream.send_pages(buf, false).await;
                    self.remove(Flags::SENDING);
                    res.map_err(|e| StreamError::from_send(&e))
                }
            }
        };
        match res {
            Ok(()) => {
                self.sent.set(self.sent.get() + len);
                Ok(())
            }
            Err(err) => {
                self.set_error(err.clone());
                Err(err)
            }
        }
    }

    async fn send_client(
        &self,
        tx: &h2::client::SendStream,
        buf: BytePages,
    ) -> Result<(), StreamError> {
        let deadline_exceeded = || {
            tx.reset(Reason::CANCEL);
            let err = tr::deadline_exceeded(tr::TIMEOUT_RAN_OUT, HeaderMap::new());
            StreamError::Client(err.with_service(self.service.get()))
        };
        self.insert(Flags::SENDING);
        let res = if let Some(deadline) = self.deadline.get() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                Err(deadline_exceeded())
            } else {
                match time::timeout(remaining, tx.send_pages(buf, false)).await {
                    Ok(res) => res.map_err(|e| StreamError::from_send(&e)),
                    Err(()) => Err(deadline_exceeded()),
                }
            }
        } else {
            tx.send_pages(buf, false)
                .await
                .map_err(|e| StreamError::from_send(&e))
        };
        self.remove(Flags::SENDING);
        res
    }

    fn close(&self) {
        let flags = self.flags.get();
        if flags.contains(Flags::CLOSED) {
            return;
        }
        self.insert(Flags::CLOSED);
        if flags.contains(Flags::ATTACHED) {
            if flags.contains(Flags::BUSY) {
                self.insert(Flags::CLOSE_PENDING);
            } else {
                self.half_close();
            }
        }
        self.wake_all();
    }

    fn cancel(&self) {
        self.insert(Flags::CLOSED);
        self.set_error(StreamError::Closed);
        self.reset();
        self.wake_all();
    }

    /// Ends a server call, returns the response headers if no message was
    /// sent, and the trailers set by the handler.
    ///
    /// Returns `None` if the stream failed, or if a message is still being
    /// written, the stream is reset then.
    pub(crate) fn finish(&self) -> Option<(Option<HeaderMap>, HeaderMap)> {
        let flags = self.flags.get();
        self.insert(Flags::CLOSED);
        if flags.contains(Flags::SENDING) {
            self.set_error(StreamError::Closed);
            self.reset();
        }
        self.wake_all();
        if self.error.borrow().is_some() {
            return None;
        }
        Some((
            self.headers.borrow_mut().take(),
            mem::take(&mut *self.trailers.borrow_mut()),
        ))
    }
}

/// Starts a streaming response, resetting the stream if its headers cannot be sent.
pub(crate) fn send_response_headers(
    stream: &StreamRef,
    headers: HeaderMap,
) -> Result<(), Error<OperationError>> {
    stream
        .send_response(StatusCode::OK, headers, false)
        .inspect_err(|err| {
            log::debug!("{}: Cannot send response headers: {err:?}", stream.tag());
            stream.reset(Reason::INTERNAL_ERROR);
        })
}

/// Cancels an unattached sender when opening fails or is dropped.
pub(crate) struct SendOpenGuard<'a>(&'a SendShared);

impl SendOpenGuard<'_> {
    pub(crate) fn fail(self, err: Error<ClientError>) {
        self.0.set_error(StreamError::Client(err));
    }
}

impl Drop for SendOpenGuard<'_> {
    fn drop(&mut self) {
        if !self.0.is_attached() {
            self.0.cancel();
        }
    }
}

/// Clears the busy state when `send()` completes or is dropped.
struct SendGuard<'a>(&'a SendShared);

impl Drop for SendGuard<'_> {
    fn drop(&mut self) {
        let shared = self.0;
        let flags = shared.flags.get();
        if flags.contains(Flags::SENDING) {
            // dropped while writing, part of the message may be sent
            shared.reset();
            shared.set_error(StreamError::Closed);
            shared.insert(Flags::CLOSED);
        }
        shared.remove(Flags::BUSY | Flags::SENDING);
        if flags.contains(Flags::CLOSE_PENDING) {
            shared.remove(Flags::CLOSE_PENDING);
            if !flags.contains(Flags::SENDING) {
                shared.half_close();
            }
        }
        shared.wake_all();
    }
}

/// A stream of messages to the peer.
///
/// On the client, the request messages of a client-streaming or a bidi
/// call. On the server, the response messages of a server-streaming or a
/// bidi call.
///
/// The stream is shared, clones send on the same stream. Messages are sent
/// one at a time, `send()` waits for the one in progress. Dropping the last
/// clone or calling [`close()`](Self::close) ends the stream, the client
/// half-closes its request then. The server ends the stream when the
/// handler returns.
pub struct SendStream<T> {
    shared: Rc<SendShared>,
    _t: PhantomData<fn(&T)>,
}

impl<T> SendStream<T> {
    /// Creates a stream for a client-streaming call.
    ///
    /// Pass it to the call, messages are sent once the call is opened,
    /// `send()` waits until then.
    pub fn new() -> Self {
        Self::from_shared(Rc::new(SendShared::new()))
    }

    pub(crate) fn from_shared(shared: Rc<SendShared>) -> Self {
        shared.handles.set(shared.handles.get() + 1);
        SendStream {
            shared,
            _t: PhantomData,
        }
    }

    pub(crate) fn shared(&self) -> &Rc<SendShared> {
        &self.shared
    }

    /// Checks if a message can be sent.
    ///
    /// Ready once the stream is open and no message is being sent, fails if
    /// the stream is closed. Does not wait for flow-control capacity,
    /// `send()` does.
    pub fn poll_ready(&self, cx: &Context<'_>) -> Poll<Result<(), StreamError>> {
        self.shared.poll_ready(cx)
    }

    /// Waits until a message can be sent, see [`poll_ready()`](Self::poll_ready).
    pub async fn ready(&self) -> Result<(), StreamError> {
        poll_fn(|cx| self.shared.poll_ready(cx)).await
    }

    /// Ends the stream, no more messages are sent.
    ///
    /// A message being sent is completed first. On the server the stream
    /// ends when the handler returns.
    pub fn close(&self) {
        self.shared.close();
    }

    /// Cancels the call, the h2 stream is reset with `CANCEL`.
    ///
    /// On the client the response stream, or the reply of a
    /// client-streaming call, fails with [`ClientError::StreamFailed`] and
    /// `CANCELLED`.
    pub fn cancel(&self) {
        self.shared.cancel();
    }

    /// Checks if the stream is closed, no more messages can be sent.
    pub fn is_closed(&self) -> bool {
        self.shared.flags.get().contains(Flags::CLOSED) || self.shared.error.borrow().is_some()
    }

    /// Adds a response header, server side only.
    ///
    /// Returns false if the headers are sent already, with the first
    /// message, or on the client.
    ///
    /// Headers exceeding the peer's limit reset the call with
    /// `INTERNAL_ERROR` when the response starts.
    pub fn header(&self, name: HeaderName, value: HeaderValue) -> bool {
        if let Some(hdrs) = self.shared.headers.borrow_mut().as_mut() {
            hdrs.append(name, value);
            true
        } else {
            false
        }
    }

    /// Adds a response trailer, server side only.
    ///
    /// The trailers are sent when the handler returns.
    pub fn trailer(&self, name: HeaderName, value: HeaderValue) {
        self.shared.trailers.borrow_mut().append(name, value);
    }
}

impl<T: Message> SendStream<T> {
    /// Sends a message.
    ///
    /// Waits until the stream is open, until a message being sent is
    /// completed, and for flow-control capacity. A message over the size
    /// limit fails with [`StreamError::Status`] and is not sent, the stream
    /// stays usable. Other errors end the stream.
    ///
    /// Dropping the future while the message is written cancels the call.
    pub async fn send(&self, msg: &T) -> Result<(), StreamError> {
        self.shared.send(msg).await
    }
}

impl<T> Default for SendStream<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for SendStream<T> {
    fn clone(&self) -> Self {
        Self::from_shared(self.shared.clone())
    }
}

impl<T> Drop for SendStream<T> {
    fn drop(&mut self) {
        let handles = self.shared.handles.get() - 1;
        self.shared.handles.set(handles);
        if handles == 0 {
            self.shared.close();
        }
    }
}

impl<T> fmt::Debug for SendStream<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SendStream")
            .field("flags", &self.shared.flags.get())
            .field("sent", &self.shared.sent.get())
            .finish()
    }
}

/// Splits the received data into messages.
///
/// The receive window of the data is held while a complete message waits
/// to be read, so a reader that falls behind stops the peer. It is released
/// while only a part of a message is buffered, a message larger than the
/// window must not stall.
pub(crate) struct Decoder {
    data: Data,
    caps: VecDeque<Capacity>,
    max_size: usize,
    enc: Option<HeaderValue>,
    /// The status of a message in an encoding we do not support.
    unsupported: GrpcStatus,
}

impl Decoder {
    pub(crate) fn new(max_size: usize, enc: Option<HeaderValue>, unsupported: GrpcStatus) -> Self {
        Decoder {
            data: Data::Empty,
            caps: VecDeque::new(),
            max_size,
            enc,
            unsupported,
        }
    }

    fn push(&mut self, data: Bytes, cap: Capacity) {
        self.data.push(data, self.max_size);
        self.caps.push_back(cap);
        if !self.has_message() {
            self.caps.clear();
        }
    }

    /// The length of the next message, if its prefix has arrived.
    fn next_len(&self) -> Option<usize> {
        let data = self.data.as_slice();
        Some(u32::from_be_bytes(data.get(1..5)?.try_into().ok()?) as usize)
    }

    fn has_message(&self) -> bool {
        self.next_len()
            .is_some_and(|len| self.data.as_slice().len() - 5 >= len)
    }

    fn has_data(&self) -> bool {
        !self.data.as_slice().is_empty()
    }

    /// Returns the next complete message and its compressed flag, or the
    /// length of a message over the size limit.
    fn next(&mut self) -> Result<Option<(u8, Bytes)>, usize> {
        let Some(len) = self.next_len() else {
            return Ok(None);
        };
        if len > self.max_size {
            return Err(len);
        }
        if self.data.as_slice().len() - 5 < len {
            return Ok(None);
        }
        let mut rest = self.data.get();
        let mut msg = rest.split_to(5 + len);
        let flag = msg[0];
        msg.advance(5);
        if !rest.is_empty() {
            self.data = Data::Chunk(rest);
        }
        if !self.has_message() {
            self.caps.clear();
        }
        Ok(Some((flag, msg)))
    }
}

struct ServerRecvInner {
    chunks: VecDeque<(Bytes, Capacity)>,
    /// The request trailers, or the error that ended the request.
    end: Option<Result<HeaderMap, StreamError>>,
    /// The client ended its stream.
    ended: bool,
    /// The handler stopped reading, data is dropped.
    closed: bool,
}

/// The request messages of a server call, the dispatcher pushes them.
pub(crate) struct ServerRecv {
    inner: RefCell<ServerRecvInner>,
    waker: LocalWaker,
}

impl ServerRecv {
    pub(crate) fn new(eof: bool) -> Rc<Self> {
        Rc::new(ServerRecv {
            inner: RefCell::new(ServerRecvInner {
                chunks: VecDeque::new(),
                end: eof.then(|| Ok(HeaderMap::new())),
                ended: eof,
                closed: false,
            }),
            waker: LocalWaker::new(),
        })
    }

    pub(crate) fn push_data(&self, data: Bytes, cap: Capacity) {
        let mut inner = self.inner.borrow_mut();
        if !inner.closed && !inner.ended {
            inner.chunks.push_back((data, cap));
            self.waker.wake();
        }
    }

    /// Ends the request with its trailers or an error.
    pub(crate) fn push_end(&self, end: Result<HeaderMap, StreamError>) {
        let mut inner = self.inner.borrow_mut();
        if !inner.ended {
            inner.ended = true;
            if !inner.closed {
                inner.end = Some(end);
            }
            self.waker.wake();
        }
    }

    /// The client ended its stream.
    pub(crate) fn is_ended(&self) -> bool {
        self.inner.borrow().ended
    }

    fn close(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.closed = true;
        inner.chunks.clear();
        inner.end = None;
    }

    fn poll(&self, trailers: &mut HeaderMap, cx: &Context<'_>) -> Poll<Item> {
        let mut inner = self.inner.borrow_mut();
        if let Some((data, cap)) = inner.chunks.pop_front() {
            Poll::Ready(Item::Data(data, cap))
        } else if let Some(end) = inner.end.take() {
            Poll::Ready(Item::End(end.map(|t| *trailers = t)))
        } else if inner.closed {
            Poll::Ready(Item::End(Err(StreamError::Closed)))
        } else {
            self.waker.register(cx.waker());
            Poll::Pending
        }
    }
}

/// The next event of the stream a `RecvStream` reads.
enum Item {
    Data(Bytes, Capacity),
    End(Result<(), StreamError>),
}

type ClientResult = Result<Option<(Bytes, Capacity)>, Error<ClientError>>;

/// The response messages of a client call.
pub(crate) struct ClientRecv {
    /// Dropping the h2 stream before its end resets it with `CANCEL`.
    rx: Option<h2::client::RecvStream>,
    deadline: Option<Instant>,
    sleep: Option<time::Sleep>,
    status: Option<StatusCode>,
    /// The body of a response that is not a grpc one, kept for the error.
    other: Data,
    /// The error to report once the last message is read.
    pending_end: Option<Error<ClientError>>,
    service: &'static str,
    /// The request stream of a client-streaming or a bidi call.
    send: Option<Rc<SendShared>>,
}

impl ClientRecv {
    pub(crate) fn new(
        rx: h2::client::RecvStream,
        deadline: Option<Instant>,
        service: &'static str,
        send: Option<Rc<SendShared>>,
    ) -> Self {
        let sleep = deadline.map(|d| {
            time::sleep(time::Millis::from(
                d.saturating_duration_since(Instant::now()),
            ))
        });
        ClientRecv {
            rx: Some(rx),
            deadline,
            sleep,
            status: None,
            other: Data::Empty,
            pending_end: None,
            service,
            send,
        }
    }

    fn close(&mut self) {
        self.rx = None;
        self.sleep = None;
    }

    fn poll(
        &mut self,
        headers: &mut HeaderMap,
        trailers: &mut HeaderMap,
        cx: &mut Context<'_>,
    ) -> Poll<Item> {
        let res = match self.poll_inner(headers, trailers, cx) {
            Poll::Ready(res) => res,
            Poll::Pending => return Poll::Pending,
        };
        Poll::Ready(match res {
            Ok(Some((data, cap))) => Item::Data(data, cap),
            Ok(None) => {
                self.close();
                Item::End(Ok(()))
            }
            Err(err) => {
                self.close();
                Item::End(Err(StreamError::Client(err.with_service(self.service))))
            }
        })
    }

    /// The result of the call once the trailers have arrived.
    fn end(
        &mut self,
        hdrs: HeaderMap,
        headers: &HeaderMap,
        trailers: &mut HeaderMap,
    ) -> ClientResult {
        *trailers = hdrs.clone();
        match tr::check_grpc_status(&hdrs) {
            Some(Ok(GrpcStatus::Ok)) | None if !tr::is_grpc(self.status, headers) => {
                Err(self.other_error(headers))
            }
            Some(Ok(GrpcStatus::Ok)) => Ok(None),
            None => Err(tr::invalid_response(
                GrpcStatus::Unknown,
                tr::NO_GRPC_STATUS,
                hdrs,
            )),
            Some(Ok(st)) => Err(Error::from(ClientError::Status(Status::new(st, hdrs)))),
            Some(Err(msg)) => Err(tr::invalid_response(GrpcStatus::Unknown, msg, hdrs)),
        }
    }

    fn poll_inner(
        &mut self,
        headers: &mut HeaderMap,
        trailers: &mut HeaderMap,
        cx: &mut Context<'_>,
    ) -> Poll<ClientResult> {
        if let Some(err) = self.pending_end.take() {
            return Poll::Ready(Err(err));
        }
        loop {
            let Some(rx) = self.rx.as_ref() else {
                return Poll::Ready(Err(StreamError::Closed.into()));
            };
            if self
                .sleep
                .as_ref()
                .is_some_and(|s| s.poll_elapsed(cx).is_ready())
            {
                return Poll::Ready(Err(tr::deadline_exceeded(
                    tr::TIMEOUT_RAN_OUT,
                    headers.clone(),
                )));
            }
            let Poll::Ready(msg) = rx.poll_recv(cx) else {
                return Poll::Pending;
            };
            let Some(msg) = msg else {
                if self.deadline.is_some_and(|d| Instant::now() >= d) {
                    return Poll::Ready(Err(tr::deadline_exceeded(
                        tr::TIMEOUT_RAN_OUT,
                        headers.clone(),
                    )));
                }
                if self.send.as_ref().is_some_and(|s| s.is_reset()) {
                    let failure = CallFailure::new(GrpcStatus::Cancelled, tr::CANCELLED)
                        .with_headers(headers.clone());
                    return Poll::Ready(Err(Error::from(ClientError::StreamFailed(failure))));
                }
                let failure = CallFailure::new(GrpcStatus::Unavailable, tr::STREAM_CLOSED)
                    .with_headers(headers.clone());
                return Poll::Ready(Err(Error::from(ClientError::Connection {
                    request_sent: true,
                    failure,
                })));
            };
            match msg.kind {
                h2::MessageKind::Headers {
                    headers: hdrs,
                    pseudo,
                    eof,
                } => {
                    if eof {
                        *trailers = hdrs.clone();
                        self.status = pseudo.status;
                        return Poll::Ready(trailers_only(pseudo.status, hdrs));
                    }
                    *headers = hdrs;
                    self.status = pseudo.status;
                }
                h2::MessageKind::Data(data, cap) => {
                    if tr::is_grpc(self.status, headers) {
                        return Poll::Ready(Ok(Some((data, cap))));
                    }
                    self.other.push(data, 0);
                    if tr::limit_other_body(self.status, headers, &mut self.other) {
                        return Poll::Ready(Err(self.other_error(headers)));
                    }
                }
                h2::MessageKind::Eof(h2::StreamEof::Data(data, cap)) => {
                    if tr::is_grpc(self.status, headers) {
                        self.pending_end = Some(tr::invalid_response(
                            GrpcStatus::Internal,
                            tr::NO_TRAILERS,
                            headers.clone(),
                        ));
                        return Poll::Ready(Ok(Some((data, cap))));
                    }
                    self.other.push(data, 0);
                    tr::limit_other_body(self.status, headers, &mut self.other);
                    return Poll::Ready(Err(self.other_error(headers)));
                }
                h2::MessageKind::Eof(h2::StreamEof::Trailers(hdrs)) => {
                    return Poll::Ready(self.end(hdrs, headers, trailers));
                }
                h2::MessageKind::Eof(h2::StreamEof::Error(err)) => {
                    return Poll::Ready(Err(
                        err.map(|e| tr::stream_error(e, headers.clone(), self.deadline))
                    ));
                }
                h2::MessageKind::Disconnect(err) => {
                    return Poll::Ready(Err(tr::operation_error(
                        err,
                        true,
                        headers.clone(),
                        self.deadline,
                    )));
                }
            }
        }
    }

    /// The error of a response that is not a grpc one.
    fn other_error(&mut self, headers: &HeaderMap) -> Error<ClientError> {
        let body = self.other.get();
        Error::from(ClientError::NotGrpc(NotGrpc::new(
            self.status,
            headers.clone(),
            body,
        )))
    }
}

/// The result of a response that ends with its headers, a grpc one with
/// `OK` is a stream without messages.
fn trailers_only(status: Option<StatusCode>, hdrs: HeaderMap) -> ClientResult {
    if tr::is_grpc(status, &hdrs)
        && matches!(tr::check_grpc_status(&hdrs), Some(Ok(GrpcStatus::Ok)))
    {
        Ok(None)
    } else {
        Err(Error::from(tr::trailers_only(status, hdrs)))
    }
}

const ERR_NO_MESSAGE: HeaderValue = HeaderValue::from_static("grpc: request without a message");
const ERR_TRUNCATED: HeaderValue = HeaderValue::from_static("grpc: request message is truncated");
const ERR_EXTRA_DATA: HeaderValue =
    HeaderValue::from_static("grpc: received data after the request message");
const ERR_DECODE: HeaderValue = HeaderValue::from_static("Cannot decode grpc message");

enum Source {
    Server(Rc<ServerRecv>),
    Client(Box<ClientRecv>),
}

type Decompress = Pin<Box<dyn Future<Output = Result<Bytes, (GrpcStatus, HeaderValue)>>>>;

/// A stream of messages from the peer.
///
/// On the client, the response messages of a server-streaming or a bidi
/// call. On the server, the request messages of a client-streaming or a
/// bidi call.
///
/// Messages are read with [`recv()`](Self::recv) or as a [`Stream`]. The
/// stream ends with `None` once the peer ends it, or with an error. The
/// messages received before an error are read first.
///
/// A decoding error discards the remaining buffered data and releases its
/// flow-control capacity.
///
/// The client reports the status of the call as [`StreamError::Client`].
/// Dropping the stream on the client before its end cancels the call.
pub struct RecvStream<T> {
    source: Source,
    decoder: Decoder,
    decompress: Option<Decompress>,
    headers: HeaderMap,
    trailers: HeaderMap,
    end: Option<Result<(), StreamError>>,
    done: bool,
    res_size: usize,
    _t: PhantomData<fn() -> T>,
}

impl<T> RecvStream<T> {
    pub(crate) fn server(
        recv: Rc<ServerRecv>,
        headers: HeaderMap,
        max_size: usize,
        enc: Option<HeaderValue>,
    ) -> Self {
        Self::with_source(
            Source::Server(recv),
            Decoder::new(max_size, enc, GrpcStatus::Unimplemented),
            headers,
        )
    }

    pub(crate) fn client(recv: ClientRecv, max_size: usize) -> Self {
        Self::with_source(
            Source::Client(Box::new(recv)),
            Decoder::new(max_size, None, GrpcStatus::Internal),
            HeaderMap::new(),
        )
    }

    fn with_source(source: Source, decoder: Decoder, headers: HeaderMap) -> Self {
        RecvStream {
            source,
            decoder,
            decompress: None,
            headers,
            trailers: HeaderMap::new(),
            end: None,
            done: false,
            res_size: 0,
            _t: PhantomData,
        }
    }

    /// The headers of the peer.
    ///
    /// On the client, the response headers, available once the first
    /// message or the end of the stream is received. On the server, the
    /// request headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// The trailers of the peer, available once the stream has ended.
    pub fn trailers(&self) -> &HeaderMap {
        &self.trailers
    }

    /// The status the stream ended with, `None` until it has ended.
    pub fn status(&self) -> Option<GrpcStatus> {
        if !self.done {
            return None;
        }
        Some(match &self.end {
            Some(Err(err)) => err.status(),
            Some(Ok(())) | None => GrpcStatus::Ok,
        })
    }

    /// The size of the messages received, with their prefixes.
    pub(crate) fn res_size(&self) -> usize {
        self.res_size
    }

    fn is_client(&self) -> bool {
        matches!(self.source, Source::Client(_))
    }

    fn service(&self) -> &'static str {
        match &self.source {
            Source::Client(recv) => recv.service,
            Source::Server(_) => "",
        }
    }

    fn close_source(&mut self) {
        match &mut self.source {
            Source::Server(recv) => recv.close(),
            Source::Client(recv) => recv.close(),
        }
    }

    /// Ends the stream with an error.
    fn fail(&mut self, err: StreamError) -> StreamError {
        self.done = true;
        self.decompress = None;
        self.end = Some(Err(err.clone()));
        self.close_source();
        self.decoder.data = Data::Empty;
        self.decoder.caps.clear();
        err
    }

    fn status_error(&self, st: GrpcStatus, msg: HeaderValue) -> StreamError {
        if self.is_client() {
            let err = tr::invalid_response(st, tr::text(&msg), self.headers.clone());
            StreamError::Client(err.with_service(self.service()))
        } else {
            StreamError::Status(st, msg)
        }
    }

    /// The error of a message over the size limit.
    fn too_large(&self, len: usize) -> StreamError {
        let max_size = self.decoder.max_size;
        if self.is_client() {
            let msg = tr::too_large(len, max_size);
            let err =
                tr::invalid_response(GrpcStatus::ResourceExhausted, msg, self.headers.clone());
            StreamError::Client(err.with_service(self.service()))
        } else {
            let msg = format!("grpc: received message larger than max ({len} vs. {max_size})");
            let msg = HeaderValue::try_from(msg).unwrap_or_else(|_| {
                HeaderValue::from_static("grpc: received message larger than max")
            });
            StreamError::Status(GrpcStatus::ResourceExhausted, msg)
        }
    }

    /// The error of a stream that ends inside a message. On the client, the
    /// error of a response without a message too.
    pub(crate) fn truncated(&self) -> StreamError {
        if self.is_client() {
            let err =
                tr::invalid_response(GrpcStatus::Internal, tr::TRUNCATED, self.trailers.clone());
            StreamError::Client(err.with_service(self.service()))
        } else {
            StreamError::Status(GrpcStatus::InvalidArgument, ERR_TRUNCATED)
        }
    }

    fn poll_source(&mut self, cx: &mut Context<'_>) -> Poll<Item> {
        match &mut self.source {
            Source::Server(recv) => recv.poll(&mut self.trailers, cx),
            Source::Client(recv) => recv.poll(&mut self.headers, &mut self.trailers, cx),
        }
    }
}

impl<T: Message> RecvStream<T> {
    fn decode(&mut self, mut msg: Bytes) -> Result<T, StreamError> {
        T::read(&mut msg).map_err(|e| {
            let err = if self.is_client() {
                let failure = CallFailure::new(GrpcStatus::Internal, e.to_string())
                    .with_headers(self.headers.clone())
                    .with_cause(e);
                let err = Error::from(ClientError::InvalidResponse(failure));
                StreamError::Client(err.with_service(self.service()))
            } else {
                StreamError::Status(GrpcStatus::InvalidArgument, ERR_DECODE)
            };
            self.fail(err)
        })
    }

    /// Polls for the next message.
    ///
    /// Returns `None` once the peer has ended the stream. After an error
    /// the stream returns `None`.
    pub fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<T, StreamError>>> {
        loop {
            if let Some(fut) = self.decompress.as_mut() {
                let res = match fut.as_mut().poll(cx) {
                    Poll::Ready(res) => res,
                    Poll::Pending => return Poll::Pending,
                };
                self.decompress = None;
                return Poll::Ready(Some(match res {
                    Ok(msg) => self.decode(msg),
                    Err((st, msg)) => Err(self.fail(self.status_error(st, msg))),
                }));
            }
            if self.done {
                return Poll::Ready(None);
            }
            match self.decoder.next() {
                Ok(Some((flag, msg))) => {
                    self.res_size += msg.len() + 5;
                    if flag == 0 {
                        return Poll::Ready(Some(self.decode(msg)));
                    }
                    self.decompress = Some(Box::pin(utils::read_message(
                        flag,
                        self.decoder.enc.clone(),
                        msg,
                        self.decoder.max_size,
                        self.decoder.unsupported,
                    )));
                    continue;
                }
                Ok(None) => {}
                Err(len) => return Poll::Ready(Some(Err(self.fail(self.too_large(len))))),
            }
            if let Some(end) = &self.end {
                let end = end.clone();
                return Poll::Ready(match end {
                    Ok(()) if self.decoder.has_data() => Some(Err(self.fail(self.truncated()))),
                    Ok(()) => {
                        self.done = true;
                        None
                    }
                    Err(err) => {
                        self.done = true;
                        Some(Err(err))
                    }
                });
            }
            match self.poll_source(cx) {
                Poll::Ready(Item::Data(data, cap)) => {
                    // the client gets the encoding with the response headers
                    if self.is_client() && self.decoder.enc.is_none() {
                        self.decoder.enc = self.headers.get(consts::GRPC_ENCODING).cloned();
                    }
                    self.decoder.push(data, cap);
                }
                Poll::Ready(Item::End(res)) => self.end = Some(res),
                Poll::Pending => return Poll::Pending,
            }
        }
    }

    /// Receives the next message, `None` once the peer has ended the stream.
    pub async fn recv(&mut self) -> Option<Result<T, StreamError>> {
        poll_fn(|cx| self.poll_recv(cx)).await
    }

    /// Reads the only request message of a server-streaming call.
    #[doc(hidden)]
    pub async fn single(mut self) -> Result<T, ServerError> {
        let msg = match self.recv().await {
            Some(Ok(msg)) => msg,
            Some(Err(err)) => return Err(err.into()),
            None => {
                return Err(ServerError::new(
                    GrpcStatus::InvalidArgument,
                    ERR_NO_MESSAGE,
                    None,
                ));
            }
        };
        match self.recv().await {
            None => Ok(msg),
            Some(Err(err)) => Err(err.into()),
            Some(Ok(_)) => Err(ServerError::new(GrpcStatus::Internal, ERR_EXTRA_DATA, None)),
        }
    }
}

impl<T: Message> Stream for RecvStream<T> {
    type Item = Result<T, StreamError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().poll_recv(cx)
    }
}

impl<T> Drop for RecvStream<T> {
    fn drop(&mut self) {
        if let Source::Server(recv) = &self.source {
            recv.close();
        }
    }
}

impl<T> fmt::Debug for RecvStream<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecvStream")
            .field("headers", &self.headers)
            .field("trailers", &self.trailers)
            .field("end", &self.end)
            .field("done", &self.done)
            .finish()
    }
}

/// The streams of a streaming call on the server.
#[doc(hidden)]
pub struct CallStreams {
    recv: Option<(Rc<ServerRecv>, HeaderMap, usize, Option<HeaderValue>)>,
    send: Option<Rc<SendShared>>,
}

impl CallStreams {
    pub(crate) fn new(
        recv: Rc<ServerRecv>,
        headers: HeaderMap,
        max_size: usize,
        enc: Option<HeaderValue>,
        send: Rc<SendShared>,
    ) -> Self {
        CallStreams {
            recv: Some((recv, headers, max_size, enc)),
            send: Some(send),
        }
    }

    /// Takes the request stream.
    ///
    /// # Panics
    ///
    /// Panics if the stream is taken already.
    pub fn recv<T>(&mut self) -> RecvStream<T> {
        let (recv, headers, max_size, enc) = self
            .recv
            .take()
            .expect("The request stream is taken already");
        RecvStream::server(recv, headers, max_size, enc)
    }

    /// Takes the response stream.
    ///
    /// # Panics
    ///
    /// Panics if the stream is taken already.
    pub fn send<T>(&mut self) -> SendStream<T> {
        SendStream::from_shared(
            self.send
                .take()
                .expect("The response stream is taken already"),
        )
    }
}

impl Drop for CallStreams {
    fn drop(&mut self) {
        // the request is not read, its data is dropped
        if let Some((recv, ..)) = self.recv.take() {
            recv.close();
        }
    }
}

impl fmt::Debug for CallStreams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallStreams")
            .field("recv", &self.recv.is_some())
            .field("send", &self.send.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(flag: u8, payload: &[u8]) -> Vec<u8> {
        let mut buf = vec![flag];
        buf.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        buf.extend_from_slice(payload);
        buf
    }

    fn decoder(max_size: usize) -> Decoder {
        Decoder::new(max_size, None, GrpcStatus::Unimplemented)
    }

    #[test]
    fn decoder_split_and_coalesced() {
        let mut dec = decoder(1024);
        let mut data = frame(0, b"hello");
        data.extend(frame(1, b""));
        data.extend(frame(0, b"world"));

        // the prefix and the payload arrive in pieces
        for chunk in [&data[..3], &data[3..8]] {
            dec.data.push(Bytes::copy_from_slice(chunk), dec.max_size);
            assert!(!dec.has_message());
            assert_eq!(dec.next(), Ok(None));
        }
        dec.data
            .push(Bytes::copy_from_slice(&data[8..]), dec.max_size);
        assert!(dec.has_message());
        assert_eq!(dec.next(), Ok(Some((0, Bytes::from_static(b"hello")))));
        assert_eq!(dec.next(), Ok(Some((1, Bytes::new()))));
        assert_eq!(dec.next(), Ok(Some((0, Bytes::from_static(b"world")))));
        assert_eq!(dec.next(), Ok(None));
        assert!(!dec.has_data());
    }

    #[test]
    fn decoder_partial_tail() {
        let mut dec = decoder(1024);
        let mut data = frame(0, b"one");
        data.extend(&frame(0, b"two")[..6]);
        dec.data.push(Bytes::from(data), dec.max_size);
        assert_eq!(dec.next(), Ok(Some((0, Bytes::from_static(b"one")))));
        assert_eq!(dec.next(), Ok(None));
        assert!(dec.has_data());
        dec.data.push(Bytes::from_static(b"wo"), dec.max_size);
        assert_eq!(dec.next(), Ok(Some((0, Bytes::from_static(b"two")))));
    }

    #[test]
    fn decoder_max_size() {
        let mut dec = decoder(4);
        // the size is checked as soon as the prefix has arrived
        dec.data
            .push(Bytes::from(frame(0, b"hello")[..5].to_vec()), dec.max_size);
        assert_eq!(dec.next(), Err(5));

        let mut dec = decoder(5);
        dec.data.push(Bytes::from(frame(0, b"hello")), dec.max_size);
        assert_eq!(dec.next(), Ok(Some((0, Bytes::from_static(b"hello")))));
    }

    #[test]
    fn server_error_from_stream_error() {
        let cases = [
            (
                StreamError::Closed,
                GrpcStatus::Internal,
                "Stream is closed",
            ),
            (
                StreamError::Reset(Reason::ENHANCE_YOUR_CALM),
                GrpcStatus::ResourceExhausted,
                "Stream reset with ENHANCE_YOUR_CALM",
            ),
            (
                StreamError::Disconnected,
                GrpcStatus::Unavailable,
                "Connection is closed",
            ),
            (
                StreamError::Status(GrpcStatus::NotFound, HeaderValue::from_static("gone")),
                GrpcStatus::NotFound,
                "gone",
            ),
        ];
        for (err, st, msg) in cases {
            let err = ServerError::from(err);
            assert_eq!((err.status, err.message.to_str().unwrap()), (st, msg));
        }
    }

    #[test]
    fn client_error_from_stream_error() {
        let err = Error::<ClientError>::from(StreamError::Status(
            GrpcStatus::ResourceExhausted,
            HeaderValue::from_static("too big"),
        ));
        assert!(matches!(*err, ClientError::InvalidRequest(_)));
        assert_eq!(err.status(), GrpcStatus::ResourceExhausted);
        assert_eq!(err.message(), "too big");

        // a refused stream is not processed by the server, it can be retried
        let err = Error::<ClientError>::from(StreamError::Reset(Reason::REFUSED_STREAM));
        assert!(matches!(*err, ClientError::NotProcessed(_)));
        assert_eq!(err.status(), GrpcStatus::Unavailable);

        let err = Error::<ClientError>::from(StreamError::Reset(Reason::CANCEL));
        assert!(matches!(*err, ClientError::StreamFailed(_)));
        assert_eq!(err.status(), GrpcStatus::Cancelled);

        let err = Error::<ClientError>::from(StreamError::Disconnected);
        assert!(matches!(
            *err,
            ClientError::Connection {
                request_sent: true,
                ..
            }
        ));
        assert_eq!(err.status(), GrpcStatus::Unavailable);

        let err = Error::<ClientError>::from(StreamError::Closed);
        assert!(matches!(*err, ClientError::StreamFailed(_)));
        assert_eq!(err.status(), GrpcStatus::Cancelled);

        // the status of the error matches the status of the call it ends
        for err in [
            StreamError::Closed,
            StreamError::Reset(Reason::REFUSED_STREAM),
            StreamError::Disconnected,
        ] {
            let st = err.status();
            assert_eq!(Error::<ClientError>::from(err).status(), st);
        }
    }

    #[test]
    fn unattached_send_stream() {
        let tx = SendStream::<()>::new();
        let cx = Context::from_waker(Waker::noop());
        // messages wait until the call is opened
        assert!(tx.poll_ready(&cx).is_pending());
        assert_eq!(tx.shared.wakers.borrow().len(), 1);
        assert!(!tx.is_closed());
        // response headers are server side only
        assert!(!tx.header(consts::GRPC_STATUS, HeaderValue::from_static("0")));

        let tx2 = tx.clone();
        drop(tx);
        assert!(!tx2.is_closed());
        drop(tx2.clone());
        assert!(!tx2.is_closed());

        // the last clone closes the stream
        let shared = tx2.shared.clone();
        drop(tx2);
        assert!(shared.flags.get().contains(Flags::CLOSED));
        assert!(shared.wakers.borrow().is_empty());
        assert!(matches!(
            shared.poll_ready(&cx),
            Poll::Ready(Err(StreamError::Closed))
        ));
    }

    #[ntex::test]
    async fn cancelled_send_stream() {
        let tx = SendStream::<()>::default();
        tx.cancel();
        assert!(tx.is_closed());
        assert!(matches!(tx.send(&()).await, Err(StreamError::Closed)));
        assert!(matches!(tx.ready().await, Err(StreamError::Closed)));

        let tx = SendStream::<()>::new();
        tx.close();
        tx.close();
        assert!(tx.is_closed());
        assert!(matches!(tx.ready().await, Err(StreamError::Closed)));
    }

    #[ntex::test]
    async fn server_recv_end() {
        // the request ended with its headers
        let mut rx = RecvStream::<()>::server(ServerRecv::new(true), HeaderMap::new(), 64, None);
        assert_eq!(rx.status(), None);
        assert!(rx.recv().await.is_none());
        assert_eq!(rx.status(), Some(GrpcStatus::Ok));
        assert!(rx.recv().await.is_none());

        // the end of a reset request is reported once
        let recv = ServerRecv::new(false);
        let mut rx = RecvStream::<()>::server(recv.clone(), HeaderMap::new(), 64, None);
        recv.push_end(Err(StreamError::Reset(Reason::CANCEL)));
        recv.push_end(Ok(HeaderMap::new()));
        assert!(recv.is_ended());
        assert!(matches!(
            rx.recv().await,
            Some(Err(StreamError::Reset(Reason::CANCEL)))
        ));
        assert_eq!(rx.status(), Some(GrpcStatus::Cancelled));
        assert!(rx.recv().await.is_none());

        let mut trailers = HeaderMap::new();
        trailers.insert(consts::GRPC_STATUS, HeaderValue::from_static("0"));
        let recv = ServerRecv::new(false);
        let mut rx = RecvStream::<()>::server(recv.clone(), HeaderMap::new(), 64, None);
        recv.push_end(Ok(trailers.clone()));
        assert!(rx.recv().await.is_none());
        assert_eq!(rx.trailers(), &trailers);
    }

    #[ntex::test]
    async fn server_recv_failure_clears_buffered_data() {
        use crate::google_types::BytesValue;

        for (invalid, status) in [
            (frame(2, b""), GrpcStatus::Internal),
            (frame(0, b"\xff"), GrpcStatus::InvalidArgument),
            (frame(0, &[0; 65]), GrpcStatus::ResourceExhausted),
        ] {
            let mut data = frame(0, b"");
            data.extend(invalid);
            data.extend(frame(0, b""));
            let mut rx = RecvStream::<BytesValue>::server(
                ServerRecv::new(true),
                HeaderMap::new(),
                64,
                None,
            );
            rx.decoder.data.push(Bytes::from(data), 64);

            assert_eq!(rx.recv().await.unwrap().unwrap(), BytesValue::default());
            let err = rx.recv().await.unwrap().unwrap_err();
            assert_eq!(err.status(), status);
            assert_eq!(rx.status(), Some(status));
            assert!(!rx.decoder.has_data());
            assert!(rx.recv().await.is_none());
        }
    }

    #[ntex::test]
    async fn server_recv_single() {
        let rx = RecvStream::<()>::server(ServerRecv::new(true), HeaderMap::new(), 64, None);
        let err = rx.single().await.unwrap_err();
        assert_eq!(err.status, GrpcStatus::InvalidArgument);

        let recv = ServerRecv::new(false);
        let rx = RecvStream::<()>::server(recv.clone(), HeaderMap::new(), 64, None);
        recv.push_end(Err(StreamError::Disconnected));
        let err = rx.single().await.unwrap_err();
        assert_eq!(err.status, GrpcStatus::Unavailable);
    }

    #[test]
    fn call_streams_drop_closes_recv() {
        let recv = ServerRecv::new(false);
        let streams = CallStreams::new(
            recv.clone(),
            HeaderMap::new(),
            64,
            None,
            Rc::new(SendShared::new()),
        );
        drop(streams);

        // the handler does not read the request, its end is dropped
        recv.push_end(Ok(HeaderMap::new()));
        assert!(recv.is_ended());
        let mut trailers = HeaderMap::new();
        let cx = Context::from_waker(Waker::noop());
        assert!(matches!(
            recv.poll(&mut trailers, &cx),
            Poll::Ready(Item::End(Err(StreamError::Closed)))
        ));
    }

    #[test]
    fn call_streams_take() {
        let recv = ServerRecv::new(false);
        let mut streams = CallStreams::new(
            recv.clone(),
            HeaderMap::new(),
            64,
            None,
            Rc::new(SendShared::new()),
        );
        let rx = streams.recv::<()>();
        let _tx = streams.send::<()>();
        drop(streams);
        // the taken stream is still read
        recv.push_end(Ok(HeaderMap::new()));
        let cx = Context::from_waker(Waker::noop());
        let mut trailers = HeaderMap::new();
        assert!(matches!(
            recv.poll(&mut trailers, &cx),
            Poll::Ready(Item::End(Ok(())))
        ));
        drop(rx);
    }
}
