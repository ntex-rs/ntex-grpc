//! Streaming calls.
use std::{fmt, marker::PhantomData, rc::Rc, time::Instant};

use ntex_error::Error;
use ntex_h2::{self as h2, OperationError, frame::Reason};
use ntex_http::{HeaderMap, HeaderName, HeaderValue, Method, error::Error as HttpError};
use ntex_util::time;

use super::request::{RequestContext, RequestContextBuilder, Response};
use super::transport::{self as tr, with_deadline};
use super::{CallFailure, Client, ClientError};
#[cfg(feature = "compression")]
use crate::Compression;
use crate::stream::{ClientRecv, SendConfig, SendShared};
use crate::{GrpcStatus, RecvStream, SendStream, service::MethodDef, utils::Enc};

/// Opens the streams of streaming calls of method `M`.
///
/// Generated clients go through this trait for client-streaming,
/// server-streaming and bidi calls. [`Client`], [`ntex_h2::client::Client`]
/// and [`ntex_h2::client::SimpleClient`] implement it. A custom transport
/// wraps one of them, a [`StreamCall`] is only created by them.
pub trait StreamTransport<M: MethodDef, St = ()> {
    /// Opens the h2 stream of a call.
    ///
    /// `ctx` holds the headers and timeout set on the [`StreamRequest`]. The
    /// timeout covers the whole call, the time spent opening the stream too.
    /// A wrapper must adjust the context before delegating to a built-in
    /// transport, which sends the request headers.
    async fn open(
        &self,
        ctx: &mut RequestContext<'_, St>,
    ) -> Result<StreamCall, Error<ClientError>>;
}

/// The h2 stream of a streaming call.
pub struct StreamCall {
    tx: h2::client::SendStream,
    rx: h2::client::RecvStream,
    deadline: Option<Instant>,
    service: &'static str,
}

impl StreamCall {
    /// The `SendStream` and `RecvStream` of the call.
    fn attach<Out, St>(
        self,
        shared: &Rc<SendShared>,
        ctx: &RequestContext<'_, St>,
    ) -> Result<RecvStream<Out>, Error<ClientError>> {
        let StreamCall {
            tx,
            rx,
            deadline,
            service,
        } = self;
        let cfg = SendConfig {
            max_size: ctx.get_max_send_message_size(),
            enc: send_enc(ctx),
            deadline,
            service,
        };
        shared
            .attach_client(tx, cfg)
            .map_err(|e| Error::<ClientError>::from(e).with_service(service))?;
        Ok(RecvStream::client(
            ClientRecv::new(rx, deadline, service, Some(shared.clone())),
            ctx.get_max_message_size(),
        ))
    }
}

impl fmt::Debug for StreamCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamCall")
            .field("deadline", &self.deadline)
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "compression")]
fn send_enc<St>(ctx: &RequestContext<'_, St>) -> Option<Enc> {
    ctx.get_compression()
}

#[cfg(not(feature = "compression"))]
fn send_enc<St>(_: &RequestContext<'_, St>) -> Option<Enc> {
    None
}

impl<M: MethodDef, St> StreamTransport<M, St> for Client {
    #[inline]
    async fn open(
        &self,
        ctx: &mut RequestContext<'_, St>,
    ) -> Result<StreamCall, Error<ClientError>> {
        ctx.message_size_defaults(self.max_message_size, self.max_send_message_size);
        StreamTransport::<M, St>::open(&self.pool, ctx).await
    }
}

impl<M: MethodDef, St> StreamTransport<M, St> for h2::client::Client {
    /// The timeout covers waiting for a connection too.
    async fn open(
        &self,
        ctx: &mut RequestContext<'_, St>,
    ) -> Result<StreamCall, Error<ClientError>> {
        with_deadline(ctx.get_timeout(), async |deadline| {
            let client = self.client().await.map_err(|e| e.map(tr::connect_error))?;
            open_stream::<M, St>(&client, ctx, deadline).await
        })
        .await
    }
}

impl<M: MethodDef, St> StreamTransport<M, St> for h2::client::SimpleClient {
    async fn open(
        &self,
        ctx: &mut RequestContext<'_, St>,
    ) -> Result<StreamCall, Error<ClientError>> {
        with_deadline(ctx.get_timeout(), async |deadline| {
            open_stream::<M, St>(self, ctx, deadline).await
        })
        .await
        .map_err(|e| e.with_service(self.service()))
    }
}

async fn open_stream<M: MethodDef, St>(
    client: &h2::client::SimpleClient,
    ctx: &mut RequestContext<'_, St>,
    deadline: Option<Instant>,
) -> Result<StreamCall, Error<ClientError>> {
    if let Some(err) = ctx.take_error() {
        return Err(Error::from(tr::request_error(err)).with_service(client.service()));
    }
    let hdrs = tr::request_headers(ctx);
    let (tx, rx) = client
        .send(Method::POST, M::PATH, hdrs, false)
        .await
        .map_err(|e| {
            tr::operation_error(e, false, HeaderMap::new(), deadline)
                .with_service(client.service())
        })?;
    if ctx.get_disconnect_on_drop() {
        tx.disconnect_on_drop();
    }
    Ok(StreamCall {
        tx,
        rx,
        deadline,
        service: client.service(),
    })
}

const STREAM_USED: &str = "SendStream is used by another call already";

/// The kind of a bidi call, see [`StreamRequest::open()`].
#[derive(Debug)]
pub struct Bidi;

/// The kind of a server-streaming call, holds the request message.
pub struct ServerStreaming<'a, In>(&'a In);

impl<In> fmt::Debug for ServerStreaming<'_, In> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerStreaming").finish_non_exhaustive()
    }
}

/// The kind of a client-streaming call, holds the request stream.
pub struct ClientStreaming(Rc<SendShared>);

impl fmt::Debug for ClientStreaming {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientStreaming").finish_non_exhaustive()
    }
}

/// A streaming call that is ready to be opened.
///
/// Generated client methods return it for client-streaming,
/// server-streaming and bidi methods. Set headers or a timeout if you need
/// them, then open the call:
///
/// * bidi: `open()` returns the request and the response streams.
/// * server-streaming: `open()` sends the request message and returns the
///   response stream.
/// * client-streaming: the request stream is passed to the method, `send()`
///   waits for the reply while the caller sends messages on the stream.
///
/// The timeout covers the whole call. Once it runs out, the streams fail
/// with [`ClientError::DeadlineExceeded`] and the call is reset.
pub struct StreamRequest<'a, T, M, K> {
    transport: &'a T,
    kind: K,
    ctx: RequestContextBuilder,
    _m: PhantomData<fn(M)>,
}

impl<'a, T, M, K> StreamRequest<'a, T, M, K> {
    fn with_kind(transport: &'a T, kind: K) -> Self {
        StreamRequest {
            transport,
            kind,
            ctx: RequestContextBuilder::new(),
            _m: PhantomData,
        }
    }

    /// Set a request header, replacing any existing value for the same name.
    ///
    /// See [`Request::header()`](super::Request::header).
    pub fn header<K1, V>(&mut self, key: K1, value: V) -> &mut Self
    where
        HeaderName: TryFrom<K1>,
        HeaderValue: TryFrom<V>,
        <HeaderName as TryFrom<K1>>::Error: Into<HttpError>,
        <HeaderValue as TryFrom<V>>::Error: Into<HttpError>,
    {
        self.ctx.header(key, value);
        self
    }

    /// Add a request header, keeping existing values for the same name.
    ///
    /// See [`Request::append_header()`](super::Request::append_header).
    pub fn append_header<K1, V>(&mut self, key: K1, value: V) -> &mut Self
    where
        HeaderName: TryFrom<K1>,
        HeaderValue: TryFrom<V>,
        <HeaderName as TryFrom<K1>>::Error: Into<HttpError>,
        <HeaderValue as TryFrom<V>>::Error: Into<HttpError>,
    {
        self.ctx.append_header(key, value);
        self
    }

    /// Set the max duration of the whole call.
    ///
    /// The duration is sent in the `grpc-timeout` header. Built-in
    /// transports reset the call once it runs out, the streams fail with
    /// [`ClientError::DeadlineExceeded`]. A zero timeout fails the call
    /// without sending it.
    pub fn timeout<U>(&mut self, timeout: U) -> &mut Self
    where
        std::time::Duration: From<U>,
    {
        self.ctx.timeout(timeout);
        self
    }

    /// Set the size limit of each response message.
    ///
    /// Overrides the [`Client`] default for this call. Without a client
    /// default or an override, the limit is 4 MiB.
    ///
    /// A larger message fails the call with
    /// [`GrpcStatus::ResourceExhausted`].
    pub fn max_message_size(&mut self, size: usize) -> &mut Self {
        self.ctx.max_message_size(size);
        self
    }

    /// Set the size limit of each request message.
    ///
    /// Overrides the [`Client`] default for this call. Without a client
    /// default or an override, the limit is 2 GiB - 1.
    ///
    /// A larger message is not sent, sending it fails with
    /// [`GrpcStatus::ResourceExhausted`].
    pub fn max_send_message_size(&mut self, size: usize) -> &mut Self {
        self.ctx.max_send_message_size(size);
        self
    }

    /// Compress the request messages with gzip or zstd.
    ///
    /// Requires the `compression` feature.
    #[cfg(feature = "compression")]
    pub fn compression(&mut self, compression: Compression) -> &mut Self {
        self.ctx.compression(compression);
        self
    }

    /// Disconnect the connection if the call is dropped before its end.
    pub fn disconnect_on_drop(&mut self) -> &mut Self {
        self.ctx.disconnect_on_drop();
        self
    }
}

impl<'a, T, M: MethodDef> StreamRequest<'a, T, M, Bidi> {
    /// Create a bidi call through `transport`.
    ///
    /// Generated clients do this for you.
    pub fn bidi(transport: &'a T) -> Self {
        Self::with_kind(transport, Bidi)
    }

    /// Open the call, returns the request and the response streams.
    ///
    /// Close the request stream, or drop it, once all messages are sent.
    /// Dropping the response stream before its end cancels the call.
    pub async fn open(
        self,
    ) -> Result<(SendStream<M::Input>, RecvStream<M::Output>), Error<ClientError>>
    where
        T: StreamTransport<M, ()>,
    {
        self.open_with(&()).await
    }

    /// Open the call, carrying `request_state` through to the transport.
    pub async fn open_with<St>(
        self,
        request_state: &St,
    ) -> Result<(SendStream<M::Input>, RecvStream<M::Output>), Error<ClientError>>
    where
        T: StreamTransport<M, St>,
    {
        let mut ctx = self.ctx.build(request_state);
        let call = self.transport.open(&mut ctx).await?;
        let tx = SendStream::new();
        let rx = call.attach::<M::Output, St>(tx.shared(), &ctx)?;
        Ok((tx, rx))
    }
}

impl<'a, T, M: MethodDef> StreamRequest<'a, T, M, ServerStreaming<'a, M::Input>> {
    /// Create a server-streaming call that sends `input` through `transport`.
    ///
    /// Generated clients do this for you.
    pub fn server_streaming(transport: &'a T, input: &'a M::Input) -> Self {
        Self::with_kind(transport, ServerStreaming(input))
    }

    /// Send the request message, returns the response stream.
    ///
    /// The transport's final size limit and compression settings are used to
    /// encode the request. If encoding fails, the opened call is cancelled
    /// without sending its body.
    ///
    /// Dropping the response stream before its end cancels the call.
    pub async fn open(self) -> Result<RecvStream<M::Output>, Error<ClientError>>
    where
        T: StreamTransport<M, ()>,
    {
        self.open_with(&()).await
    }

    /// Send the request message, carrying `request_state` through to the
    /// transport.
    pub async fn open_with<St>(
        self,
        request_state: &St,
    ) -> Result<RecvStream<M::Output>, Error<ClientError>>
    where
        T: StreamTransport<M, St>,
    {
        let mut ctx = self.ctx.build(request_state);
        let StreamCall {
            tx,
            rx,
            deadline,
            service,
        } = self.transport.open(&mut ctx).await?;

        let deadline_exceeded = || {
            tx.reset(Reason::CANCEL);
            tr::deadline_exceeded(tr::TIMEOUT_RAN_OUT, HeaderMap::new()).with_service(service)
        };
        let send = async {
            let buf = tr::encode_message(self.kind.0, &ctx).await.map_err(|err| {
                tx.reset(Reason::CANCEL);
                Error::from(err).with_service(service)
            })?;
            // Encoding can finish without yielding to the timeout.
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return Err(deadline_exceeded());
            }
            if let Err(err) = tx.send_pages(buf, true).await {
                // the server can reply and reset the stream before it reads the
                // whole request, the reply or the reset is in the receive queue
                if !matches!(
                    *err,
                    OperationError::RemoteReset(_) | OperationError::Closed(Some(_))
                ) {
                    return Err(tr::operation_error(err, true, HeaderMap::new(), deadline)
                        .with_service(service));
                }
            }
            Ok(())
        };
        if let Some(deadline) = deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(deadline_exceeded());
            }
            match time::timeout(remaining, send).await {
                Ok(res) => res?,
                Err(()) => return Err(deadline_exceeded()),
            }
        } else {
            send.await?;
        }
        Ok(RecvStream::client(
            ClientRecv::new(rx, deadline, service, None),
            ctx.get_max_message_size(),
        ))
    }
}

impl<'a, T, M: MethodDef> StreamRequest<'a, T, M, ClientStreaming> {
    /// Create a client-streaming call through `transport`, the request
    /// messages are sent on `tx`.
    ///
    /// Generated clients do this for you.
    pub fn client_streaming(transport: &'a T, tx: &SendStream<M::Input>) -> Self {
        Self::with_kind(transport, ClientStreaming(tx.shared().clone()))
    }

    /// Open the call and wait for the reply.
    ///
    /// Send the request messages on the stream passed to the method while
    /// this future runs, then close the stream or drop it. Messages sent
    /// before the call is opened wait for it. Once this future is polled,
    /// the stream belongs to this call, even while opening. Reusing it for
    /// another call fails with [`ClientError::InvalidRequest`].
    ///
    /// An opening error also fails the sender with the same client error.
    /// Dropping this future while opening closes the sender and wakes
    /// waiting sends.
    ///
    /// ```rust,ignore
    /// let tx = ntex_grpc::SendStream::new();
    /// let (res, _) = futures::join!(client.record_route(&tx).send(), async {
    ///     for point in points {
    ///         tx.send(&point).await?;
    ///     }
    ///     tx.close();
    ///     Ok::<_, ntex_grpc::StreamError>(())
    /// });
    /// ```
    pub async fn send(self) -> Result<Response<M>, Error<ClientError>>
    where
        T: StreamTransport<M, ()>,
    {
        self.send_with(&()).await
    }

    /// Open the call and wait for the reply, carrying `request_state`
    /// through to the transport.
    pub async fn send_with<St>(self, request_state: &St) -> Result<Response<M>, Error<ClientError>>
    where
        T: StreamTransport<M, St>,
    {
        let shared = self.kind.0;
        let opening = shared.start_open().ok_or_else(|| {
            Error::from(ClientError::InvalidRequest(CallFailure::new(
                GrpcStatus::Internal,
                STREAM_USED,
            )))
        })?;
        let mut ctx = self.ctx.build(request_state);
        let call = match self.transport.open(&mut ctx).await {
            Ok(call) => call,
            Err(err) => {
                opening.fail(err.clone());
                return Err(err);
            }
        };
        let service = call.service;
        let mut rx = call.attach::<M::Output, St>(&shared, &ctx)?;
        drop(opening);

        let output = match rx.recv().await {
            Some(Ok(output)) => output,
            Some(Err(err)) => return Err(err.into()),
            None => {
                let failure = CallFailure::new(GrpcStatus::Internal, tr::NO_MESSAGE)
                    .with_headers(rx.trailers().clone());
                let err = Error::from(ClientError::InvalidResponse(failure));
                return Err(err.with_service(service));
            }
        };
        match rx.recv().await {
            None => {}
            Some(Err(err)) => return Err(err.into()),
            Some(Ok(_)) => {
                let hdrs = rx.headers().clone();
                let err = tr::invalid_response(GrpcStatus::Internal, tr::EXTRA_DATA, hdrs);
                return Err(err.with_service(service));
            }
        }
        Ok(Response {
            output,
            headers: rx.headers().clone(),
            trailers: rx.trailers().clone(),
            req_size: shared.sent(),
            res_size: rx.res_size(),
        })
    }
}

impl<T, M, K: fmt::Debug> fmt::Debug for StreamRequest<'_, T, M, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamRequest")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
