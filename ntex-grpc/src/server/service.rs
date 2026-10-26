use std::{cell::RefCell, error::Error, hash::Hash, rc::Rc};

use ntex_bytes::{Buf, ByteString, Bytes};
use ntex_h2::{self as h2, StreamRef, frame::Reason, frame::StreamId};
use ntex_http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header::CONTENT_TYPE};
use ntex_io::{Filter, Io, IoBoxed};
use ntex_service::boxed::{self, BoxService, BoxServiceFactory};
use ntex_service::{Ctx, Pipeline, Service, ServiceFactory, cfg::SharedCfg};
use ntex_util::{HashMap, future::join_all, time::Millis, time::timeout_checked};

use crate::stream::{CallStreams, SendShared, ServerRecv, StreamError, send_response_headers};
use crate::utils::{self, Data, Enc};
use crate::{consts, encode_grpc_message, status::GrpcStatus};

use super::{NamedService, ServerError, ServerRequest, ServerResponse};

/// The length prefix of an uncompressed empty message.
const EMPTY_MESSAGE: &[u8] = &[0; 5];
const ERR_NO_MESSAGE: HeaderValue = HeaderValue::from_static("grpc: request without a message");
const ERR_TRUNCATED: HeaderValue = HeaderValue::from_static("grpc: request message is truncated");
const ERR_DECODE_TIMEOUT: HeaderValue =
    HeaderValue::from_static("Cannot decode grpc-timeout header");
const ERR_DEADLINE: HeaderValue = HeaderValue::from_static("Deadline exceeded");
const ERR_EXTRA_DATA: HeaderValue =
    HeaderValue::from_static("grpc: received data after the request message");
const ERR_CONTENT_TYPE: &str = "grpc: invalid request content-type";
const HDR_APP_GRPC: HeaderValue = HeaderValue::from_static("application/grpc");

/// The default limit of a request message.
const DEFAULT_MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

/// The default limit of a response message.
const DEFAULT_MAX_SEND_MESSAGE_SIZE: usize = i32::MAX as usize;

const MILLIS_IN_HOUR: u64 = 60 * 60 * 1000;
const MILLIS_IN_MINUTE: u64 = 60 * 1000;

type Factory = BoxServiceFactory<(), ServerRequest, ServerResponse, ServerError, Box<dyn Error>>;
type BoxedService = BoxService<(), ServerRequest, ServerResponse, ServerError>;
/// Tells if a method of a service is a streaming one.
type IsStreaming = fn(&str) -> bool;

fn not_streaming(_: &str) -> bool {
    false
}

/// Grpc server
///
/// Routes each call by the service name in the request path,
/// `/helloworld.Greeter/SayHello` goes to the service registered for
/// `helloworld.Greeter`. A call to a service that is not registered goes to
/// the [default service](GrpcServer::default_service), or fails with
/// `UNIMPLEMENTED` if there is none.
///
/// A service instance of each registered service is created for every
/// connection, the connection fails if one of them cannot be created.
///
/// The `grpc-timeout` header limits service execution. A zero timeout fails
/// with `DEADLINE_EXCEEDED` without calling the service. Positive fractional
/// milliseconds are rounded up to the next millisecond. If the header is
/// absent, service execution has no timeout.
pub struct GrpcServer {
    /// Names of the registered services, `names[i]` is served by
    /// `factories[i]`.
    names: Rc<[ByteString]>,
    /// `streaming[i]` tells the streaming methods of `names[i]`.
    streaming: Rc<[IsStreaming]>,
    factories: Vec<Factory>,
    default: Option<Factory>,
    max_message_size: usize,
    max_send_message_size: usize,
}

impl GrpcServer {
    /// Create grpc server with one service, it takes the calls for
    /// [`NamedService::NAME`].
    ///
    /// Add more services with [`service()`](Self::service), a call to any
    /// other service fails with `UNIMPLEMENTED`.
    pub fn new<T>(factory: T) -> Self
    where
        T: NamedService
            + ServiceFactory<(), ServerRequest, Res = ServerResponse, Error = ServerError>
            + 'static,
        T::InitError: Into<Box<dyn Error>>,
    {
        Self {
            names: Rc::new([]),
            streaming: Rc::new([]),
            factories: Vec::new(),
            default: None,
            max_message_size: DEFAULT_MAX_MESSAGE_SIZE,
            max_send_message_size: DEFAULT_MAX_SEND_MESSAGE_SIZE,
        }
        .service(factory)
    }

    #[must_use]
    /// Add a service, it takes the calls for [`NamedService::NAME`].
    ///
    /// `#[server]` implements [`NamedService`], use
    /// [`service_named()`](Self::service_named) for other services.
    ///
    /// # Panics
    ///
    /// Panics if a service with the same name is already added.
    pub fn service<T>(self, factory: T) -> Self
    where
        T: NamedService
            + ServiceFactory<(), ServerRequest, Res = ServerResponse, Error = ServerError>
            + 'static,
        T::InitError: Into<Box<dyn Error>>,
    {
        self.add(ByteString::from_static(T::NAME), T::is_streaming, factory)
    }

    #[must_use]
    /// Add a service that takes the calls for the service `name`, the full
    /// service name with the package, e.g. `helloworld.Greeter`.
    ///
    /// All its methods are unary ones, use [`service()`](Self::service)
    /// for a service with streaming methods.
    ///
    /// # Panics
    ///
    /// Panics if `name` is empty or contains `/`, or if a service with the
    /// same name is already added.
    pub fn service_named<T>(self, name: impl Into<ByteString>, factory: T) -> Self
    where
        T: ServiceFactory<(), ServerRequest, Res = ServerResponse, Error = ServerError> + 'static,
        T::InitError: Into<Box<dyn Error>>,
    {
        self.add(name.into(), not_streaming, factory)
    }

    fn add<T>(mut self, name: ByteString, streaming: IsStreaming, factory: T) -> Self
    where
        T: ServiceFactory<(), ServerRequest, Res = ServerResponse, Error = ServerError> + 'static,
        T::InitError: Into<Box<dyn Error>>,
    {
        assert!(
            !name.is_empty() && !name.contains('/'),
            "invalid grpc service name: {name:?}"
        );
        assert!(
            !self.names.contains(&name),
            "grpc service {name} is already added"
        );
        self.names = self.names.iter().cloned().chain([name]).collect();
        self.streaming = self.streaming.iter().copied().chain([streaming]).collect();
        self.factories.push(box_factory(factory));
        self
    }

    #[must_use]
    /// Set the service that takes the calls for the services that are not
    /// added, whatever their service name. All its methods are unary ones.
    ///
    /// # Panics
    ///
    /// Panics if the default service is already set.
    pub fn default_service<T>(mut self, factory: T) -> Self
    where
        T: ServiceFactory<(), ServerRequest, Res = ServerResponse, Error = ServerError> + 'static,
        T::InitError: Into<Box<dyn Error>>,
    {
        assert!(
            self.default.is_none(),
            "grpc default service is already set"
        );
        self.default = Some(box_factory(factory));
        self
    }

    #[must_use]
    /// Set the largest request message the server accepts, in bytes.
    ///
    /// A larger message, or a compressed one that is larger once
    /// decompressed, fails the call with `RESOURCE_EXHAUSTED`. The default
    /// is 4 MiB.
    pub fn max_message_size(mut self, size: usize) -> Self {
        self.max_message_size = size;
        self
    }

    #[must_use]
    /// Set the largest response message the server sends, in bytes.
    ///
    /// A larger message fails the call with `RESOURCE_EXHAUSTED`. A message
    /// is never sent if it is 4 GiB or larger, its length does not fit the
    /// length prefix. A compressed message is checked after compression. The
    /// default is 2 GiB - 1.
    pub fn max_send_message_size(mut self, size: usize) -> Self {
        self.max_send_message_size = size;
        self
    }

    async fn run(&self, io: IoBoxed) -> Result<(), Box<dyn Error>> {
        let cfg = io.shared();

        // init services
        let factories = self.factories.iter().chain(&self.default);
        let services = join_all(factories.map(|f| f.create(&())))
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

        let _ = h2::server::handle_one(
            io,
            Pipeline::new(
                (),
                PublishService {
                    cfg,
                    names: self.names.clone(),
                    streaming: self.streaming.clone(),
                    services,
                    max_size: self.max_message_size,
                    max_send_size: self.max_send_message_size,
                    streams: RefCell::new(HashMap::default()),
                },
            ),
            Pipeline::new((), ControlService),
        )
        .await;

        Ok(())
    }
}

fn box_factory<T>(factory: T) -> Factory
where
    T: ServiceFactory<(), ServerRequest, Res = ServerResponse, Error = ServerError> + 'static,
    T::InitError: Into<Box<dyn Error>>,
{
    boxed::factory(factory.map_init_err(Into::into))
}

impl<F: Filter> Service<(), Io<F>> for GrpcServer {
    type Res = ();
    type Error = Box<dyn Error>;

    async fn call(&self, io: Io<F>, _: Ctx<'_, Self, ()>) -> Result<(), Self::Error> {
        self.run(io.boxed()).await
    }
}

impl Service<(), IoBoxed> for GrpcServer {
    type Res = ();
    type Error = Box<dyn Error>;

    async fn call(&self, io: IoBoxed, _: Ctx<'_, Self, ()>) -> Result<(), Self::Error> {
        self.run(io).await
    }
}

struct ControlService;

impl Service<(), h2::Control<h2::StreamError>> for ControlService {
    type Res = h2::ControlAck;
    type Error = Rc<dyn Error>;

    async fn call(
        &self,
        msg: h2::Control<h2::StreamError>,
        _: Ctx<'_, Self, ()>,
    ) -> Result<Self::Res, Self::Error> {
        log::trace!("Control message: {msg:?}");
        Ok(msg.ack())
    }
}

struct PublishService {
    cfg: SharedCfg,
    /// Names of the registered services, `names[i]` is served by
    /// `services[i]`.
    names: Rc<[ByteString]>,
    /// `streaming[i]` tells the streaming methods of `names[i]`.
    streaming: Rc<[IsStreaming]>,
    /// The registered services, followed by the default one if there is one.
    services: Vec<BoxedService>,
    max_size: usize,
    max_send_size: usize,
    streams: RefCell<HashMap<StreamId, Inflight>>,
}

enum Inflight {
    /// A unary request, read before the service is called.
    Unary {
        /// The request path without the leading `/`, `service/method`.
        path: ByteString,
        data: Data,
        headers: HeaderMap,
    },
    /// A streaming request, the service reads it as it arrives.
    Stream(Rc<ServerRecv>),
}

impl PublishService {
    /// Returns the index of the service that serves `service`, the default
    /// service serves the names that are not registered.
    fn route(&self, service: &str) -> Option<usize> {
        match self.names.iter().position(|name| name == service) {
            Some(idx) => Some(idx),
            None => (self.services.len() > self.names.len()).then_some(self.names.len()),
        }
    }

    fn is_streaming(&self, idx: usize, method: &str) -> bool {
        self.streaming.get(idx).is_some_and(|f| f(method))
    }

    /// Checks the request message as data arrives, so a message over the
    /// limit or data after the message is not buffered. A unary request has
    /// exactly one message.
    fn check_request(&self, data: &[u8]) -> Result<(), (GrpcStatus, HeaderValue)> {
        let [_, a, b, c, d, ..] = *data else {
            return Ok(());
        };
        let len = u32::from_be_bytes([a, b, c, d]) as usize;
        if len > self.max_size {
            let msg = format!(
                "grpc: received message larger than max ({len} vs. {})",
                self.max_size
            );
            let msg = HeaderValue::try_from(msg)
                .unwrap_or_else(|_| HeaderValue::from_static("grpc: received message too large"));
            Err((GrpcStatus::ResourceExhausted, msg))
        } else if data.len() - 5 > len {
            Err((GrpcStatus::Internal, ERR_EXTRA_DATA))
        } else {
            Ok(())
        }
    }

    /// Returns the request message, decompressed if needed.
    async fn read_request(
        &self,
        headers: &HeaderMap,
        mut data: Bytes,
    ) -> Result<Bytes, (GrpcStatus, HeaderValue)> {
        self.check_request(&data)?;
        if data.len() < 5 {
            return Err((GrpcStatus::InvalidArgument, ERR_TRUNCATED));
        }
        let flag = data.get_u8();
        let len = data.get_u32() as usize;
        let Some(data) = data.split_to_checked(len) else {
            return Err((GrpcStatus::InvalidArgument, ERR_TRUNCATED));
        };
        // a message in an unknown encoding is unimplemented
        utils::read_message(
            flag,
            headers.get(consts::GRPC_ENCODING).cloned(),
            data,
            self.max_size,
            GrpcStatus::Unimplemented,
        )
        .await
    }
}

impl Service<(), h2::Message> for PublishService {
    type Res = ();
    type Error = h2::StreamError;

    #[allow(clippy::too_many_lines)]
    async fn call(
        &self,
        msg: h2::Message,
        ctx: Ctx<'_, Self, ()>,
    ) -> Result<Self::Res, Self::Error> {
        let id = msg.id();
        let h2::Message { stream, kind } = msg;

        match kind {
            h2::MessageKind::Headers {
                headers,
                pseudo,
                eof,
            } => {
                // not a grpc request, see the gRPC over HTTP/2 spec
                if let Some(msg) = check_content_type(&headers) {
                    let (code, st) = (
                        StatusCode::UNSUPPORTED_MEDIA_TYPE,
                        GrpcStatus::InvalidArgument,
                    );
                    reject(&stream, code, st, msg, eof);
                    return Ok(());
                }
                if let Some(msg) = check_method(pseudo.method.as_ref()) {
                    let (code, st) = (StatusCode::METHOD_NOT_ALLOWED, GrpcStatus::Internal);
                    reject(&stream, code, st, msg, eof);
                    return Ok(());
                }

                let path = pseudo.path.unwrap().split_off(1);
                let Some(n) = path.find('/') else {
                    // not a `service/method` path, the method is unknown
                    let msg =
                        encode_grpc_message(&format!("grpc: malformed method name: /{path}"));
                    reject(&stream, StatusCode::OK, GrpcStatus::Unimplemented, msg, eof);
                    return Ok(());
                };
                // the request is not read for an unknown service
                let Some(idx) = self.route(&path[..n]) else {
                    let msg =
                        encode_grpc_message(&format!("grpc: unknown service {}", &path[..n]));
                    reject(&stream, StatusCode::OK, GrpcStatus::Unimplemented, msg, eof);
                    return Ok(());
                };

                let (service, name) = split_path(path.clone());
                if self.is_streaming(idx, &name) {
                    let call = StreamCall {
                        stream: &stream,
                        svc: &self.services[idx],
                        service,
                        name,
                        headers,
                        eof,
                    };
                    self.call_stream(call, ctx).await;
                    return Ok(());
                }

                // stream eof, cannot do anything
                if eof {
                    if stream.send_response(StatusCode::OK, hdrs(), false).is_ok() {
                        send_error(&stream, GrpcStatus::InvalidArgument, ERR_NO_MESSAGE);
                    }
                    return Ok(());
                }

                let _ = self.streams.borrow_mut().insert(
                    stream.id(),
                    Inflight::Unary {
                        headers,
                        data: Data::Empty,
                        path,
                    },
                );
            }
            h2::MessageKind::Data(chunk, cap) => {
                let mut streams = self.streams.borrow_mut();
                match streams.get_mut(&id) {
                    Some(Inflight::Stream(recv)) => recv.push_data(chunk, cap),
                    Some(Inflight::Unary { data, .. }) => {
                        data.push(chunk, self.max_size);
                        if let Err((status, msg)) = self.check_request(data.as_slice()) {
                            remove(&mut streams, &id);
                            drop(streams);
                            if stream.send_response(StatusCode::OK, hdrs(), false).is_ok() {
                                send_error(&stream, status, msg);
                            }
                            // the client stops sending the rest of the request
                            stream.reset(Reason::NO_ERROR);
                        }
                    }
                    None => {}
                }
            }
            h2::MessageKind::Eof(eof) => {
                let inflight = remove(&mut self.streams.borrow_mut(), &id);
                match inflight {
                    Some(Inflight::Stream(recv)) => {
                        let end = match eof {
                            h2::StreamEof::Data(chunk, cap) => {
                                recv.push_data(chunk, cap);
                                Ok(HeaderMap::new())
                            }
                            h2::StreamEof::Trailers(hdrs) => Ok(hdrs),
                            h2::StreamEof::Error(err) => Err(match *err {
                                h2::StreamError::Reset(r) => StreamError::Reset(r),
                                _ => StreamError::Closed,
                            }),
                        };
                        recv.push_end(end);
                    }
                    Some(Inflight::Unary {
                        path,
                        mut data,
                        mut headers,
                    }) => {
                        match eof {
                            h2::StreamEof::Data(chunk, _cap) => {
                                data.push(chunk, self.max_size);
                            }
                            h2::StreamEof::Trailers(hdrs) => {
                                for (name, val) in &hdrs {
                                    headers.insert(name.clone(), val.clone());
                                }
                            }
                            h2::StreamEof::Error(err) => return Err(err.into_error()),
                        }
                        return self.call_unary(&stream, path, data, headers, ctx).await;
                    }
                    None => {}
                }
            }
            h2::MessageKind::Disconnect(_) => {
                if let Some(Inflight::Stream(recv)) = remove(&mut self.streams.borrow_mut(), &id) {
                    recv.push_end(Err(StreamError::Disconnected));
                }
            }
        }
        Ok(())
    }
}

/// A streaming call, passed to the service once its headers arrive.
struct StreamCall<'a> {
    stream: &'a StreamRef,
    svc: &'a BoxedService,
    service: ByteString,
    name: ByteString,
    headers: HeaderMap,
    /// The client sent no request messages.
    eof: bool,
}

impl PublishService {
    async fn call_unary(
        &self,
        stream: &StreamRef,
        path: ByteString,
        mut data: Data,
        headers: HeaderMap,
        ctx: Ctx<'_, Self, ()>,
    ) -> Result<(), h2::StreamError> {
        let data = match self.read_request(&headers, data.get()).await {
            Ok(data) => data,
            Err((status, msg)) => {
                if stream.send_response(StatusCode::OK, hdrs(), false).is_ok() {
                    send_error(stream, status, msg);
                }
                return Ok(());
            }
        };

        let (service, name) = split_path(path);
        // the service is known, it is checked with the headers
        let Some(idx) = self.route(&service) else {
            if stream.send_response(StatusCode::OK, hdrs(), false).is_ok() {
                let msg = encode_grpc_message(&format!("grpc: unknown service {service}"));
                send_error(stream, GrpcStatus::Unimplemented, msg);
            }
            return Ok(());
        };
        log::debug!("{}: Call service {service} method {name}", self.cfg.tag());
        let req = ServerRequest {
            payload: data,
            service,
            name,
            headers,
            streams: None,
        };
        // the response is compressed like the request
        let encoding = Enc::of_response(&req.headers);
        if stream
            .send_response(StatusCode::OK, response_headers(encoding), false)
            .is_err()
        {
            return Ok(());
        }

        let to = match request_timeout(&req.headers) {
            Ok(to) => to,
            Err((status, msg)) => {
                send_error(stream, status, msg);
                return Ok(());
            }
        };

        match timeout_checked(to, ctx.call(&self.services[idx], req)).await {
            Ok(Ok(res)) => {
                log::debug!("{}: Response is received {res:?}", self.cfg.tag());
                if res.payload.is_empty() {
                    // an empty message is never compressed
                    let _ = stream
                        .send_payload(Bytes::from_static(EMPTY_MESSAGE), false)
                        .await;
                } else {
                    match utils::frame_message(res.payload, self.max_send_size, encoding).await {
                        Ok(buf) => {
                            let _ = stream.send_pages(buf, false).await;
                        }
                        Err((status, msg)) => {
                            send_error(stream, status, msg);
                            return Ok(());
                        }
                    }
                }

                let mut trailers = HeaderMap::default();
                trailers.insert(consts::GRPC_STATUS, GrpcStatus::Ok.into());
                append_trailers(&mut trailers, res.headers);
                send_trailers(stream, trailers);
            }
            Ok(Err(err)) => {
                log::debug!(
                    "{}: Failure during service call: {:?}",
                    self.cfg.tag(),
                    err.message
                );
                send_trailers(stream, error_trailers(err));
            }
            Err(()) => {
                log::debug!(
                    "{}: Deadline exceeded failure during service call",
                    self.cfg.tag()
                );
                send_error(stream, GrpcStatus::DeadlineExceeded, ERR_DEADLINE);
            }
        }
        Ok(())
    }

    /// Calls the service with the streams of the call, the request messages
    /// are pushed to it as they arrive. The call ends when the service
    /// returns.
    async fn call_stream(&self, call: StreamCall<'_>, ctx: Ctx<'_, Self, ()>) {
        let StreamCall {
            stream,
            svc,
            service,
            name,
            headers,
            eof,
        } = call;

        let to = match request_timeout(&headers) {
            Ok(to) => to,
            Err((status, msg)) => {
                reject(stream, StatusCode::OK, status, msg, eof);
                return;
            }
        };

        let recv = ServerRecv::new(eof);
        if !eof {
            let _ = self
                .streams
                .borrow_mut()
                .insert(stream.id(), Inflight::Stream(recv.clone()));
        }

        // the response is compressed like the request, the headers are sent
        // with the first message or when the call ends
        let encoding = Enc::of_response(&headers);
        let send = SendShared::server(
            stream.clone(),
            response_headers(encoding),
            self.max_send_size,
            encoding,
        );
        let streams = CallStreams::new(
            recv.clone(),
            headers.clone(),
            self.max_size,
            headers.get(consts::GRPC_ENCODING).cloned(),
            send.clone(),
        );
        log::debug!(
            "{}: Call streaming service {service} method {name}",
            self.cfg.tag()
        );
        let req = ServerRequest {
            payload: Bytes::new(),
            service,
            name,
            headers,
            streams: Some(streams),
        };
        let result = timeout_checked(to, ctx.call(svc, req)).await;

        if let Some((res_headers, extra)) = send.finish() {
            let headers_sent = match res_headers {
                Some(hdrs) => send_response_headers(stream, hdrs).is_ok(),
                None => true,
            };
            if headers_sent {
                let mut trailers = match result {
                    Ok(Ok(res)) => {
                        let mut trailers = HeaderMap::default();
                        trailers.insert(consts::GRPC_STATUS, GrpcStatus::Ok.into());
                        append_trailers(&mut trailers, res.headers);
                        trailers
                    }
                    Ok(Err(err)) => {
                        log::debug!(
                            "{}: Failure during service call: {:?}",
                            self.cfg.tag(),
                            err.message
                        );
                        error_trailers(err)
                    }
                    Err(()) => {
                        log::debug!(
                            "{}: Deadline exceeded failure during service call",
                            self.cfg.tag()
                        );
                        let mut trailers = HeaderMap::default();
                        trailers.insert(consts::GRPC_STATUS, GrpcStatus::DeadlineExceeded.into());
                        trailers.insert(consts::GRPC_MESSAGE, ERR_DEADLINE);
                        trailers
                    }
                };
                let extra = extra.iter().map(|(n, v)| (n.clone(), v.clone()));
                append_trailers(&mut trailers, extra);
                send_trailers(stream, trailers);
                // the call is over, the client stops sending the request
                if !recv.is_ended() {
                    stream.reset(Reason::NO_ERROR);
                }
            }
        }
        remove(&mut self.streams.borrow_mut(), &stream.id());
    }
}

/// The response headers, with the encoding of the response messages.
fn response_headers(encoding: Option<Enc>) -> HeaderMap {
    let mut headers = hdrs();
    if let Some(enc) = encoding {
        headers.insert(consts::GRPC_ENCODING, enc.header());
    }
    headers
}

/// Rejects invalid or expired timeouts, zero means no header and no deadline.
fn request_timeout(headers: &HeaderMap) -> Result<Millis, (GrpcStatus, HeaderValue)> {
    let Some(value) = headers.get(consts::GRPC_TIMEOUT) else {
        return Ok(Millis::ZERO);
    };
    let timeout = try_parse_grpc_timeout(value)
        .map_err(|()| (GrpcStatus::InvalidArgument, ERR_DECODE_TIMEOUT))?;
    if timeout.is_zero() {
        Err((GrpcStatus::DeadlineExceeded, ERR_DEADLINE))
    } else {
        Ok(timeout)
    }
}

/// Appends the trailers of the service, except `grpc-status` and
/// `grpc-message`, the call succeeded whatever the service says.
fn append_trailers<I>(trailers: &mut HeaderMap, extra: I)
where
    I: IntoIterator<Item = (HeaderName, HeaderValue)>,
{
    for (name, val) in extra {
        if name != consts::GRPC_STATUS && name != consts::GRPC_MESSAGE {
            trailers.append(name, val);
        }
    }
}

fn error_trailers(err: ServerError) -> HeaderMap {
    let mut trailers = err.headers;
    trailers.insert(consts::GRPC_STATUS, err.status.into());
    trailers.insert(consts::GRPC_MESSAGE, err.message);
    trailers
}

/// Capacity of the streams map that is kept however few streams are open.
const STREAMS_CAPACITY: usize = 64;

/// Removes the stream `key`. The map gives back memory once fewer than a
/// quarter of its capacity is used, so a burst of streams is not kept for the
/// life of the connection.
fn remove<K: Hash + Eq, V>(map: &mut HashMap<K, V>, key: &K) -> Option<V> {
    let val = map.remove(key);
    if map.capacity() > STREAMS_CAPACITY && map.len() < map.capacity() / 4 {
        map.shrink_to(map.len() * 2);
    }
    val
}

/// Splits `service/method` into the service and the method name, the method
/// name ends at the next `/`. The path must contain a `/`.
fn split_path(mut path: ByteString) -> (ByteString, ByteString) {
    let n = path.find('/').unwrap_or(path.len());
    let service = path.split_to(n);
    let mut name = path.split_off(1);
    if let Some(n) = name.find('/') {
        name = name.split_to(n);
    }
    (service, name)
}

/// Returns the `grpc-message` to reject the request with if it is not a grpc
/// one.
fn check_content_type(hdrs: &HeaderMap) -> Option<HeaderValue> {
    match hdrs.get(CONTENT_TYPE) {
        Some(val) if utils::is_grpc_content_type(val.as_bytes()) => None,
        Some(val) => Some(utils::grpc_message(ERR_CONTENT_TYPE, val)),
        None => Some(HeaderValue::from_static(ERR_CONTENT_TYPE)),
    }
}

/// Returns the `grpc-message` of a request whose method is not `POST`.
fn check_method(method: Option<&Method>) -> Option<HeaderValue> {
    match method {
        Some(&Method::POST) => None,
        Some(method) => Some(encode_grpc_message(&format!(
            "grpc: method {method} is not supported"
        ))),
        None => Some(HeaderValue::from_static("grpc: method is not supported")),
    }
}

/// Rejects a request that is not a grpc call, with an HTTP status and the
/// grpc status the client reports.
fn reject(stream: &StreamRef, code: StatusCode, st: GrpcStatus, msg: HeaderValue, eof: bool) {
    let mut hdrs = hdrs();
    hdrs.insert(consts::GRPC_STATUS, st.into());
    hdrs.insert(consts::GRPC_MESSAGE, msg);
    let _ = stream.send_response(code, hdrs, true);
    if !eof {
        // the client stops sending the request
        stream.reset(Reason::NO_ERROR);
    }
}

fn hdrs() -> HeaderMap {
    let mut hdrs = HeaderMap::default();
    hdrs.insert(CONTENT_TYPE, HDR_APP_GRPC);
    hdrs.insert(consts::GRPC_ACCEPT_ENCODING, consts::ACCEPT_ENCODING);
    hdrs
}

fn send_error(stream: &StreamRef, st: GrpcStatus, msg: HeaderValue) {
    let mut trailers = HeaderMap::default();
    trailers.insert(consts::GRPC_STATUS, st.into());
    trailers.insert(consts::GRPC_MESSAGE, msg);
    send_trailers(stream, trailers);
}

/// Sends trailers, the stream is reset if they cannot be sent.
///
/// Trailers that exceed the peer's `SETTINGS_MAX_HEADER_LIST_SIZE` are not sent
/// and the stream stays open, it is reset with `INTERNAL_ERROR` instead.
fn send_trailers(stream: &StreamRef, trailers: HeaderMap) {
    if let Err(err) = stream.send_trailers(trailers) {
        log::debug!("{}: Cannot send trailers: {err:?}", stream.tag());
        stream.reset(Reason::INTERNAL_ERROR);
    }
}

/// Tries to parse the `grpc-timeout` header if it is present.
///
/// Follows the [gRPC over HTTP2 spec](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md).
fn try_parse_grpc_timeout(val: &HeaderValue) -> Result<Millis, ()> {
    // the unit is the last byte, the value may not be ascii
    let (&timeout_unit, timeout_value) = val.as_bytes().split_last().ok_or(())?;

    // gRPC spec specifies `TimeoutValue` as 1 to 8 ascii digits, no sign.
    // Caping this at 8 digits also prevents integer overflow from ever occurring
    if timeout_value.is_empty()
        || timeout_value.len() > 8
        || !timeout_value.iter().all(u8::is_ascii_digit)
    {
        return Err(());
    }

    let timeout_value = timeout_value
        .iter()
        .fold(0u64, |acc, d| acc * 10 + u64::from(d - b'0'));
    let duration = match timeout_unit {
        // Hours
        b'H' => Millis(u32::try_from(timeout_value * MILLIS_IN_HOUR).unwrap_or(u32::MAX)),
        // Minutes
        b'M' => Millis(u32::try_from(timeout_value * MILLIS_IN_MINUTE).unwrap_or(u32::MAX)),
        // Seconds
        b'S' => Millis(u32::try_from(timeout_value * 1000).unwrap_or(u32::MAX)),
        // Milliseconds
        b'm' => Millis(u32::try_from(timeout_value).unwrap_or(u32::MAX)),
        // Microseconds
        b'u' => Millis(u32::try_from(timeout_value.div_ceil(1000)).unwrap_or(u32::MAX)),
        // Nanoseconds
        b'n' => Millis(u32::try_from(timeout_value.div_ceil(1_000_000)).unwrap_or(u32::MAX)),
        _ => return Err(()),
    };

    Ok(duration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::send_size;

    #[test]
    fn content_type() {
        let check = |ct: Option<&'static str>| {
            let mut hdrs = HeaderMap::new();
            if let Some(ct) = ct {
                hdrs.insert(CONTENT_TYPE, HeaderValue::from_static(ct));
            }
            check_content_type(&hdrs)
        };
        assert_eq!(check(Some("application/grpc")), None);
        assert_eq!(check(Some("application/grpc+proto")), None);
        assert_eq!(check(Some("Application/GRPC;charset=utf-8")), None);
        assert_eq!(
            check(Some("application/json")).unwrap(),
            "grpc: invalid request content-type: application/json"
        );
        assert_eq!(
            check(Some("application/grpcx")).unwrap(),
            "grpc: invalid request content-type: application/grpcx"
        );
        assert_eq!(check(None).unwrap(), "grpc: invalid request content-type");
    }

    #[test]
    fn method() {
        assert_eq!(check_method(Some(&Method::POST)), None);
        assert_eq!(
            check_method(Some(&Method::GET)).unwrap(),
            "grpc: method GET is not supported"
        );
        assert_eq!(
            check_method(Some(&Method::from_bytes(b"BREW").unwrap())).unwrap(),
            "grpc: method BREW is not supported"
        );
        // `%` is a valid token character, it is percent-encoded
        assert_eq!(
            check_method(Some(&Method::from_bytes(b"A%B").unwrap())).unwrap(),
            "grpc: method A%25B is not supported"
        );
        assert_eq!(check_method(None).unwrap(), "grpc: method is not supported");
    }

    #[test]
    fn send_size_limit() {
        assert_eq!(send_size(0, 0).unwrap(), 0);
        assert_eq!(send_size(3, 3).unwrap(), 3);
        assert_eq!(
            send_size(4, 3).unwrap_err(),
            "grpc: trying to send message larger than max (4 vs. 3)"
        );
        let max = u32::MAX as usize;
        assert_eq!(send_size(max, usize::MAX).unwrap(), u32::MAX);
        assert_eq!(
            send_size(max + 1, usize::MAX).unwrap_err(),
            "grpc: message too large (4294967296 bytes)"
        );
    }

    #[test]
    fn inflight_size() {
        // a slot of the streams map holds the state of a request
        eprintln!(
            "SIZES {} {} {} {}",
            size_of::<Inflight>(),
            size_of::<ByteString>(),
            size_of::<Data>(),
            size_of::<HeaderMap>()
        );
        assert!(size_of::<Inflight>() <= 96, "{}", size_of::<Inflight>());
    }

    #[test]
    fn streams_shrink() {
        let mut map = HashMap::default();
        for id in 0..256 {
            map.insert(id, ());
        }
        let mut shrinks = 0;
        for id in 0..255 {
            let cap = map.capacity();
            assert!(remove(&mut map, &id).is_some());
            // a removed entry may leave a tombstone, which takes one off the
            // capacity
            if map.capacity() + 1 < cap {
                shrinks += 1;
                assert!(map.len() < cap / 4, "{} {cap}", map.len());
                // half of the capacity stays free, so the next streams do
                // not grow the map right away
                assert!(map.capacity() >= map.len() * 2, "{}", map.len());
            }
            assert!(
                map.capacity() <= 64 || map.len() >= map.capacity() / 4,
                "{} {}",
                map.len(),
                map.capacity()
            );
        }
        assert!(map.capacity() <= STREAMS_CAPACITY, "{}", map.capacity());
        assert!(shrinks <= 4, "{shrinks}");
        assert!(remove(&mut map, &1000).is_none());
        assert!(remove(&mut map, &255).is_some());

        // a small map is kept
        let mut map = HashMap::default();
        for id in 0..32 {
            map.insert(id, ());
        }
        for id in 0..32 {
            let cap = map.capacity();
            remove(&mut map, &id);
            assert!(map.capacity() + 1 >= cap, "{} {cap}", map.capacity());
        }
    }

    #[test]
    fn path() {
        for (path, service, name) in [
            ("test.Svc/Call", "test.Svc", "Call"),
            ("test.Svc/Call/extra", "test.Svc", "Call"),
            ("test.Svc/", "test.Svc", ""),
            ("/Call", "", "Call"),
        ] {
            let (s, n) = split_path(ByteString::from_static(path));
            assert_eq!((&*s, &*n), (service, name), "{path}");
        }
    }

    #[test]
    fn grpc_timeout() {
        for (val, millis) in [
            ("1H", 60 * 60 * 1000),
            ("2M", 2 * 60 * 1000),
            ("3S", 3000),
            ("4m", 4),
            ("1u", 1),
            ("999u", 1),
            ("1000u", 1),
            ("1001u", 2),
            ("1500u", 2),
            ("1n", 1),
            ("999999n", 1),
            ("1000000n", 1),
            ("1000001n", 2),
            ("2500000n", 3),
            ("0S", 0),
            ("0u", 0),
            ("0n", 0),
            // the largest value fits in a u32 of millis
            ("99999999H", u32::MAX),
            ("99999999M", u32::MAX),
            ("99999999S", u32::MAX),
            ("99999999m", 99_999_999),
        ] {
            let timeout = try_parse_grpc_timeout(&HeaderValue::from_static(val));
            assert_eq!(timeout, Ok(Millis(millis)), "{val}");
        }

        for val in [
            // no unit, no value
            "",
            "S",
            "1",
            "x",
            // more than 8 digits
            "123456789S", // not a number
            "abcS",
            "-1S",
            "+1S",
            "+S",
            "1.5S",
            " 1S", // unknown unit
            "1X",
            "1s",
            "1h",
        ] {
            let timeout = try_parse_grpc_timeout(&HeaderValue::from_static(val));
            assert_eq!(timeout, Err(()), "{val}");
        }

        // not ascii, a multi-byte character must not be split
        for val in [
            &b"\xff1S"[..],
            "1é".as_bytes(),
            "é".as_bytes(),
            "1Sé".as_bytes(),
        ] {
            let timeout = try_parse_grpc_timeout(&HeaderValue::from_bytes(val).unwrap());
            assert_eq!(timeout, Err(()), "{val:?}");
        }
    }
}
