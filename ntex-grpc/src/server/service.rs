use std::{cell::RefCell, error::Error, rc::Rc};

use ntex_bytes::{Buf, BufMut, BytePages, ByteString};
use ntex_h2::{self as h2, StreamRef, frame::Reason, frame::StreamId};
use ntex_http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_TYPE};
use ntex_io::{Filter, Io, IoBoxed};
use ntex_service::{Ctx, Pipeline, Service, ServiceFactory, cfg::SharedCfg};
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
const HDR_APP_GRPC: HeaderValue = HeaderValue::from_static("application/grpc");

const MILLIS_IN_HOUR: u64 = 60 * 60 * 1000;
const MILLIS_IN_MINUTE: u64 = 60 * 1000;

/// Grpc server
pub struct GrpcServer<T> {
    factory: Rc<T>,
    control: Pipeline<h2::Control<h2::StreamError>, h2::ControlAck, Rc<dyn Error>>,
}

impl<T> GrpcServer<T> {
    /// Create grpc server
    pub fn new(factory: T) -> Self {
        Self {
            factory: Rc::new(factory),
            control: Pipeline::new((), ControlService),
        }
    }
}

impl<Sf> GrpcServer<Sf>
where
    Sf: ServiceFactory<(), ServerRequest, Res = ServerResponse, Error = ServerError>
        + 'static,
    Sf::InitError: Into<Box<dyn Error>>,
{
    async fn run(&self, io: IoBoxed) -> Result<(), Box<dyn Error>> {
        let cfg = io.shared();

        // init server
        let svc = self.factory.create(&()).await.map_err(Into::into)?;

        let _ = h2::server::handle_one(
            io,
            Pipeline::new((), PublishService::new(svc, cfg)),
            self.control.bind(),
        )
        .await;

        Ok(())
    }
}

impl<Sf, F> Service<(), Io<F>> for GrpcServer<Sf>
where
    F: Filter,
    Sf: ServiceFactory<(), ServerRequest, Res = ServerResponse, Error = ServerError>
        + 'static,
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
    Sf: ServiceFactory<(), ServerRequest, Res = ServerResponse, Error = ServerError>
        + 'static,
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
    fn new(service: S, cfg: SharedCfg) -> Self {
        Self {
            cfg,
            service,
            streams: RefCell::new(HashMap::default()),
        }
    }
}

impl<S> Service<(), h2::Message> for PublishService<S>
where
    S: Service<(), ServerRequest, Res = ServerResponse, Error = ServerError> + 'static,
{
    type Res = ();
    type Error = h2::StreamError;

    #[allow(clippy::await_holding_refcell_ref, clippy::too_many_lines)]
    async fn call(
        &self,
        msg: h2::Message,
        ctx: Ctx<'_, Self, ()>,
    ) -> Result<Self::Res, Self::Error> {
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
                            for (name, val) in &hdrs {
                                inflight.headers.insert(name.clone(), val.clone());
                            }
                        }
                        h2::StreamEof::Error(err) => return Err(err.into_error()),
                    }

                    let mut data = inflight.data.get();
                    let _compressed = data.get_u8();
                    let len = data.get_u32();
                    if (len as usize) > data.len() {
                        if stream.send_response(StatusCode::OK, hdrs(), false).is_ok() {
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
                    if stream.send_response(StatusCode::OK, hdrs(), false).is_err() {
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
                        Ok(Ok(mut res)) => {
                            log::debug!("{}: Response is received {res:?}", self.cfg.tag());
                            let mut buf = BytePages::default();
                            buf.put_u8(0); // compression
                            buf.put_u32(res.payload.len() as u32); // length
                            res.payload.move_to(&mut buf);

                            let _ = stream.send_pages(buf, false).await;

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
                streams.remove(&id);
            }
        }
        Ok(())
    }
}

fn hdrs() -> HeaderMap {
    let mut hdrs = HeaderMap::default();
    hdrs.insert(CONTENT_TYPE, HDR_APP_GRPC);
    hdrs
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
        "n" => Millis(u32::try_from(timeout_value / 1_000_000).unwrap_or(u32::MAX)),
        _ => return Err(()),
    };

    Ok(duration)
}
