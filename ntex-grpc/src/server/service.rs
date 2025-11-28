use std::{cell::RefCell, rc::Rc};

use ntex_bytes::{Buf, BufMut, ByteString, BytesMut};
use ntex_h2::{self as h2, StreamRef, frame::Reason, frame::StreamId};
use ntex_http::{HeaderMap, HeaderValue, StatusCode};
use ntex_io::{Filter, Io, IoBoxed};
use ntex_service::{Service, ServiceCtx, ServiceFactory, cfg::SharedCfg};
use ntex_util::{HashMap, time::Millis, time::timeout_checked};

use crate::{consts, status::GrpcStatus, utils::Data};

use super::{ServerError, ServerRequest, ServerResponse};

const ERR_DECODE: HeaderValue =
    HeaderValue::from_static("Cannot decode request message: not enough data provided");
const ERR_DATA_DECODE: HeaderValue =
    HeaderValue::from_static("Cannot decode request message: not enough data provided");
const ERR_DECODE_TIMEOUT: HeaderValue =
    HeaderValue::from_static("Cannot decode grpc-timeout header");
const ERR_DEADLINE: HeaderValue = HeaderValue::from_static("Deadline exceeded");

const MILLIS_IN_HOUR: u64 = 60 * 60 * 1000;
const MILLIS_IN_MINUTE: u64 = 60 * 1000;

/// Grpc server
pub struct GrpcServer<T> {
    factory: Rc<T>,
}

impl<T> GrpcServer<T> {
    /// Create grpc server
    pub fn new(factory: T) -> Self {
        Self {
            factory: Rc::new(factory),
        }
    }
}

impl<T> GrpcServer<T>
where
    T: ServiceFactory<ServerRequest, SharedCfg, Response = ServerResponse, Error = ServerError>,
    T::Service: Clone,
{
    /// Create default server
    pub fn make_server(&self, cfg: SharedCfg) -> GrpcService<T> {
        log::trace!("{}: Starting grpc service", cfg.tag());

        GrpcService {
            cfg,
            factory: self.factory.clone(),
        }
    }
}

impl<F, T> ServiceFactory<Io<F>, SharedCfg> for GrpcServer<T>
where
    F: Filter,
    T: ServiceFactory<ServerRequest, SharedCfg, Response = ServerResponse, Error = ServerError>
        + 'static,
    T::Service: Clone,
{
    type Response = ();
    type Error = T::InitError;
    type Service = GrpcService<T>;
    type InitError = ();

    async fn create(&self, cfg: SharedCfg) -> Result<Self::Service, Self::InitError> {
        Ok(self.make_server(cfg))
    }
}

pub struct GrpcService<T> {
    cfg: SharedCfg,
    factory: Rc<T>,
}

impl<T, F> Service<Io<F>> for GrpcService<T>
where
    F: Filter,
    T: ServiceFactory<ServerRequest, SharedCfg, Response = ServerResponse, Error = ServerError>
        + 'static,
{
    type Response = ();
    type Error = T::InitError;

    async fn call(&self, io: Io<F>, _: ServiceCtx<'_, Self>) -> Result<(), Self::Error> {
        // init server
        let service = self.factory.create(self.cfg).await?;

        let _ = h2::server::handle_one(
            io.into(),
            PublishService::new(service, self.cfg),
            ControlService,
        )
        .await;

        Ok(())
    }
}

impl<T> Service<IoBoxed> for GrpcService<T>
where
    T: ServiceFactory<ServerRequest, SharedCfg, Response = ServerResponse, Error = ServerError>
        + 'static,
{
    type Response = ();
    type Error = T::InitError;

    async fn call(&self, io: IoBoxed, _: ServiceCtx<'_, Self>) -> Result<(), Self::Error> {
        // init server
        let service = self.factory.create(self.cfg).await?;

        let _ = h2::server::handle_one(io, PublishService::new(service, self.cfg), ControlService)
            .await;

        Ok(())
    }
}

struct ControlService;

impl Service<h2::ControlMessage<h2::StreamError>> for ControlService {
    type Response = h2::ControlResult;
    type Error = ();

    async fn call(
        &self,
        msg: h2::ControlMessage<h2::StreamError>,
        _: ServiceCtx<'_, Self>,
    ) -> Result<Self::Response, Self::Error> {
        log::trace!("Control message: {msg:?}");
        Ok::<_, ()>(msg.ack())
    }
}

struct PublishService<S: Service<ServerRequest>> {
    cfg: SharedCfg,
    service: S,
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
    S: Service<ServerRequest, Response = ServerResponse, Error = ServerError>,
{
    fn new(service: S, cfg: SharedCfg) -> Self {
        Self {
            cfg,
            service,
            streams: RefCell::new(HashMap::default()),
        }
    }
}

impl<S> Service<h2::Message> for PublishService<S>
where
    S: Service<ServerRequest, Response = ServerResponse, Error = ServerError> + 'static,
{
    type Response = ();
    type Error = h2::StreamError;

    #[allow(clippy::await_holding_refcell_ref)]
    async fn call(
        &self,
        msg: h2::Message,
        ctx: ServiceCtx<'_, Self>,
    ) -> Result<Self::Response, Self::Error> {
        let id = msg.id();
        let h2::Message { stream, kind } = msg;
        let mut streams = self.streams.borrow_mut();

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
                    let _ =
                        stream.send_response(StatusCode::NOT_FOUND, HeaderMap::default(), true);
                    return Ok(());
                };

                // stream eof, cannot do anything
                if eof {
                    if stream
                        .send_response(StatusCode::OK, HeaderMap::default(), false)
                        .is_ok()
                    {
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

                let _ = streams.insert(
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
                if let Some(inflight) = streams.get_mut(&stream.id()) {
                    inflight.data.push(data);
                }
            }
            h2::MessageKind::Eof(data) => {
                if let Some(mut inflight) = streams.remove(&id) {
                    match data {
                        h2::StreamEof::Data(chunk) => inflight.data.push(chunk),
                        h2::StreamEof::Trailers(hdrs) => {
                            for (name, val) in hdrs.iter() {
                                inflight.headers.insert(name.clone(), val.clone());
                            }
                        }
                        h2::StreamEof::Error(err) => return Err(err),
                    }

                    let mut data = inflight.data.get();
                    let _compressed = data.get_u8();
                    let len = data.get_u32();
                    if (len as usize) > data.len() {
                        if stream
                            .send_response(StatusCode::OK, HeaderMap::default(), false)
                            .is_ok()
                        {
                            send_error(&stream, GrpcStatus::InvalidArgument, ERR_DATA_DECODE);
                        }
                        return Ok(());
                    }
                    let data = data
                        .split_to_checked(len as usize)
                        .ok_or(h2::StreamError::Reset(Reason::PROTOCOL_ERROR))?;

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
                    if stream
                        .send_response(StatusCode::OK, HeaderMap::default(), false)
                        .is_err()
                    {
                        return Ok(());
                    }
                    drop(streams);

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
                        Ok(Ok(res)) => {
                            log::debug!("{}: Response is received {res:?}", self.cfg.tag());
                            let mut buf = BytesMut::with_capacity(res.payload.len() + 5);
                            buf.put_u8(0); // compression
                            buf.put_u32(res.payload.len() as u32); // length
                            buf.extend_from_slice(&res.payload);

                            let _ = stream.send_payload(buf.freeze(), false).await;

                            let mut trailers = HeaderMap::default();
                            trailers.insert(consts::GRPC_STATUS, GrpcStatus::Ok.into());
                            for (name, val) in res.headers {
                                trailers.append(name, val);
                            }

                            stream.send_trailers(trailers);
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
                            stream.send_trailers(trailers);
                        }
                        Err(_) => {
                            log::debug!(
                                "{}: Deadline exceeded failure during service call",
                                self.cfg.tag()
                            );
                            send_error(&stream, GrpcStatus::DeadlineExceeded, ERR_DEADLINE);
                        }
                    };

                    return Ok(());
                }
            }
            h2::MessageKind::Disconnect(_) => {
                streams.remove(&id);
            }
        }
        Ok(())
    }
}

fn send_error(stream: &StreamRef, st: GrpcStatus, msg: HeaderValue) {
    let mut trailers = HeaderMap::default();
    trailers.insert(consts::GRPC_STATUS, st.into());
    trailers.insert(consts::GRPC_MESSAGE, msg);
    stream.send_trailers(trailers);
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
        "n" => Millis(u32::try_from(timeout_value / 1000000).unwrap_or(u32::MAX)),
        _ => return Err(()),
    };

    Ok(duration)
}
