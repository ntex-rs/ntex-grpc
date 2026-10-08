use std::{cell::RefCell, error::Error, rc::Rc};

use ntex_bytes::{Buf, BufMut, BytePages, ByteString, Bytes};
use ntex_h2::{self as h2, StreamRef, frame::Reason, frame::StreamId};
use ntex_http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_TYPE};
use ntex_io::{Filter, Io, IoBoxed};
use ntex_service::{Ctx, Pipeline, Service, ServiceFactory, cfg::SharedCfg};
use ntex_util::{HashMap, time::Millis, time::timeout_checked};

#[cfg(feature = "compression")]
use crate::Compression;
use crate::utils::{self, Data};
use crate::{consts, status::GrpcStatus};

use super::{ServerError, ServerRequest, ServerResponse};

const ERR_DECODE: HeaderValue =
    HeaderValue::from_static("Cannot decode request message: not enough data provided");
const ERR_DATA_DECODE: HeaderValue =
    HeaderValue::from_static("Cannot decode request message: not enough data provided");
const ERR_DECODE_TIMEOUT: HeaderValue =
    HeaderValue::from_static("Cannot decode grpc-timeout header");
const ERR_DEADLINE: HeaderValue = HeaderValue::from_static("Deadline exceeded");
const HDR_APP_GRPC: HeaderValue = HeaderValue::from_static("application/grpc");

/// The default limit of a request message.
const DEFAULT_MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

const MILLIS_IN_HOUR: u64 = 60 * 60 * 1000;
const MILLIS_IN_MINUTE: u64 = 60 * 1000;

/// Grpc server
pub struct GrpcServer<T> {
    factory: Rc<T>,
    max_message_size: usize,
}

impl<T> GrpcServer<T> {
    /// Create grpc server
    pub fn new(factory: T) -> Self {
        Self {
            factory: Rc::new(factory),
            max_message_size: DEFAULT_MAX_MESSAGE_SIZE,
        }
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
}

impl<Sf> GrpcServer<Sf>
where
    Sf: ServiceFactory<(), ServerRequest, Res = ServerResponse, Error = ServerError> + 'static,
    Sf::InitError: Into<Box<dyn Error>>,
{
    async fn run(&self, io: IoBoxed) -> Result<(), Box<dyn Error>> {
        let cfg = io.shared();

        // init server
        let svc = self.factory.create(&()).await.map_err(Into::into)?;

        let _ = h2::server::handle_one(
            io,
            Pipeline::new((), PublishService::new(svc, cfg, self.max_message_size)),
            Pipeline::new((), ControlService),
        )
        .await;

        Ok(())
    }
}

impl<Sf, F> Service<(), Io<F>> for GrpcServer<Sf>
where
    F: Filter,
    Sf: ServiceFactory<(), ServerRequest, Res = ServerResponse, Error = ServerError> + 'static,
    Sf::InitError: Into<Box<dyn Error>>,
{
    type Res = ();
    type Error = Box<dyn Error>;

    async fn call(&self, io: Io<F>, _: Ctx<'_, Self, ()>) -> Result<(), Self::Error> {
        self.run(io.boxed()).await
    }
}

impl<Sf> Service<(), IoBoxed> for GrpcServer<Sf>
where
    Sf: ServiceFactory<(), ServerRequest, Res = ServerResponse, Error = ServerError> + 'static,
    Sf::InitError: Into<Box<dyn Error>>,
{
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

struct PublishService<S: Service<(), ServerRequest>> {
    cfg: SharedCfg,
    service: S,
    max_size: usize,
    streams: RefCell<HashMap<StreamId, Inflight>>,
}

struct Inflight {
    name: ByteString,
    service: ByteString,
    data: Data,
    headers: HeaderMap,
}

impl<S> PublishService<S>
where
    S: Service<(), ServerRequest, Res = ServerResponse, Error = ServerError>,
{
    fn new(service: S, cfg: SharedCfg, max_size: usize) -> Self {
        Self {
            cfg,
            service,
            max_size,
            streams: RefCell::new(HashMap::default()),
        }
    }

    /// Returns the request message, decompressed if needed.
    async fn read_request(
        &self,
        headers: &HeaderMap,
        mut data: Bytes,
    ) -> Result<Bytes, (GrpcStatus, HeaderValue)> {
        if data.len() < 5 {
            return Err((GrpcStatus::InvalidArgument, ERR_DATA_DECODE));
        }
        let flag = data.get_u8();
        let len = data.get_u32() as usize;
        let Some(data) = data.split_to_checked(len) else {
            return Err((GrpcStatus::InvalidArgument, ERR_DATA_DECODE));
        };
        if len > self.max_size {
            let msg = format!(
                "grpc: received message larger than max ({len} vs. {})",
                self.max_size
            );
            let msg = HeaderValue::try_from(msg)
                .unwrap_or_else(|_| HeaderValue::from_static("grpc: received message too large"));
            return Err((GrpcStatus::ResourceExhausted, msg));
        }
        // a message in an unknown encoding is unimplemented
        utils::read_message(
            flag,
            headers,
            data,
            self.max_size,
            GrpcStatus::Unimplemented,
        )
        .await
    }
}

impl<S> Service<(), h2::Message> for PublishService<S>
where
    S: Service<(), ServerRequest, Res = ServerResponse, Error = ServerError> + 'static,
{
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
                let mut path = pseudo.path.unwrap().split_off(1);
                let srvname = if let Some(n) = path.find('/') {
                    path.split_to(n)
                } else {
                    // not found
                    let _ = stream.send_response(StatusCode::NOT_FOUND, hdrs(), true);
                    return Ok(());
                };

                // stream eof, cannot do anything
                if eof {
                    if stream.send_response(StatusCode::OK, hdrs(), false).is_ok() {
                        send_error(&stream, GrpcStatus::InvalidArgument, ERR_DECODE);
                    }
                    return Ok(());
                }

                let mut path = path.split_off(1);
                let methodname = if let Some(n) = path.find('/') {
                    path.split_to(n)
                } else {
                    path
                };

                let _ = self.streams.borrow_mut().insert(
                    stream.id(),
                    Inflight {
                        headers,
                        data: Data::Empty,
                        name: methodname,
                        service: srvname,
                    },
                );
            }
            h2::MessageKind::Data(data, _cap) => {
                if let Some(inflight) = self.streams.borrow_mut().get_mut(&stream.id()) {
                    inflight.data.push(data);
                }
            }
            h2::MessageKind::Eof(data) => {
                let inflight = self.streams.borrow_mut().remove(&id);
                if let Some(mut inflight) = inflight {
                    match data {
                        h2::StreamEof::Data(chunk, _cap) => inflight.data.push(chunk),
                        h2::StreamEof::Trailers(hdrs) => {
                            for (name, val) in &hdrs {
                                inflight.headers.insert(name.clone(), val.clone());
                            }
                        }
                        h2::StreamEof::Error(err) => return Err(err.into_error()),
                    }

                    let data = match self
                        .read_request(&inflight.headers, inflight.data.get())
                        .await
                    {
                        Ok(data) => data,
                        Err((status, msg)) => {
                            if stream.send_response(StatusCode::OK, hdrs(), false).is_ok() {
                                send_error(&stream, status, msg);
                            }
                            return Ok(());
                        }
                    };

                    log::debug!(
                        "{}: Call service {} method {}",
                        self.cfg.tag(),
                        inflight.service,
                        inflight.name
                    );
                    let req = ServerRequest {
                        payload: data,
                        name: inflight.name,
                        headers: inflight.headers,
                    };
                    // the response is compressed like the request
                    #[cfg(feature = "compression")]
                    let encoding = req
                        .headers
                        .get(consts::GRPC_ENCODING)
                        .and_then(Compression::from_header);
                    #[cfg(feature = "compression")]
                    let headers = {
                        let mut headers = hdrs();
                        if let Some(enc) = encoding {
                            headers.insert(consts::GRPC_ENCODING, enc.header());
                        }
                        headers
                    };
                    #[cfg(not(feature = "compression"))]
                    let headers = hdrs();
                    if stream
                        .send_response(StatusCode::OK, headers, false)
                        .is_err()
                    {
                        return Ok(());
                    }

                    // GRPC Timeout
                    let to = if let Some(to) = req.headers.get(consts::GRPC_TIMEOUT) {
                        if let Ok(to) = try_parse_grpc_timeout(to) {
                            to
                        } else {
                            send_error(&stream, GrpcStatus::InvalidArgument, ERR_DECODE_TIMEOUT);
                            return Ok(());
                        }
                    } else {
                        Millis::ZERO
                    };

                    match timeout_checked(to, ctx.call(&self.service, req)).await {
                        Ok(Ok(mut res)) => {
                            log::debug!("{}: Response is received {res:?}", self.cfg.tag());
                            let mut buf = BytePages::default();
                            #[cfg(feature = "compression")]
                            if let Some(enc) = encoding
                                && !res.payload.is_empty()
                            {
                                match enc.compress(res.payload.freeze()).await {
                                    Ok(payload) => {
                                        buf.put_u8(1);
                                        buf.put_u32(payload.len() as u32);
                                        buf.append(payload);
                                    }
                                    Err(err) => {
                                        let msg = HeaderValue::try_from(format!(
                                            "grpc: error while compressing: {err}"
                                        ))
                                        .unwrap_or_else(|_| {
                                            HeaderValue::from_static(
                                                "grpc: error while compressing",
                                            )
                                        });
                                        send_error(&stream, GrpcStatus::Internal, msg);
                                        return Ok(());
                                    }
                                }
                            }
                            if buf.is_empty() {
                                buf.put_u8(0); // compression
                                buf.put_u32(res.payload.len() as u32); // length
                                res.payload.move_to(&mut buf);
                            }

                            let _ = stream.send_pages(buf, false).await;

                            let mut trailers = HeaderMap::default();
                            trailers.insert(consts::GRPC_STATUS, GrpcStatus::Ok.into());
                            for (name, val) in res.headers {
                                trailers.append(name, val);
                            }

                            send_trailers(&stream, trailers);
                        }
                        Ok(Err(err)) => {
                            log::debug!(
                                "{}: Failure during service call: {:?}",
                                self.cfg.tag(),
                                err.message
                            );
                            let mut trailers = err.headers;
                            trailers.insert(consts::GRPC_STATUS, err.status.into());
                            trailers.insert(consts::GRPC_MESSAGE, err.message);
                            send_trailers(&stream, trailers);
                        }
                        Err(()) => {
                            log::debug!(
                                "{}: Deadline exceeded failure during service call",
                                self.cfg.tag()
                            );
                            send_error(&stream, GrpcStatus::DeadlineExceeded, ERR_DEADLINE);
                        }
                    }

                    return Ok(());
                }
            }
            h2::MessageKind::Disconnect(_) => {
                self.streams.borrow_mut().remove(&id);
            }
        }
        Ok(())
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
    let (timeout_value, timeout_unit) = val
        .to_str()
        .map_err(|_| ())
        .and_then(|s| if s.is_empty() { Err(()) } else { Ok(s) })?
        .split_at(val.len() - 1);

    // gRPC spec specifies `TimeoutValue` will be at most 8 digits
    // Caping this at 8 digits also prevents integer overflow from ever occurring
    if timeout_value.len() > 8 {
        return Err(());
    }

    let timeout_value: u64 = timeout_value.parse().map_err(|_| ())?;
    let duration = match timeout_unit {
        // Hours
        "H" => Millis(u32::try_from(timeout_value * MILLIS_IN_HOUR).unwrap_or(u32::MAX)),
        // Minutes
        "M" => Millis(u32::try_from(timeout_value * MILLIS_IN_MINUTE).unwrap_or(u32::MAX)),
        // Seconds
        "S" => Millis(u32::try_from(timeout_value * 1000).unwrap_or(u32::MAX)),
        // Milliseconds
        "m" => Millis(u32::try_from(timeout_value).unwrap_or(u32::MAX)),
        // Microseconds
        "u" => Millis(u32::try_from(timeout_value / 1000).unwrap_or(u32::MAX)),
        // Nanoseconds
        "n" => Millis(u32::try_from(timeout_value / 1_000_000).unwrap_or(u32::MAX)),
        _ => return Err(()),
    };

    Ok(duration)
}
