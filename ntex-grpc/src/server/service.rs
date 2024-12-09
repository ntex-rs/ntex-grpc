use std::{cell::RefCell, rc::Rc};

use ntex_bytes::{Buf, BufMut, ByteString, BytesMut};
use ntex_h2::{self as h2, frame::Reason, frame::StreamId};
use ntex_http::{HeaderMap, HeaderValue, StatusCode};
use ntex_io::{Filter, Io, IoBoxed};
use ntex_service::{Service, ServiceCtx, ServiceFactory};
use ntex_util::HashMap;

use crate::{consts, status::GrpcStatus, utils::Data};

use super::{ServerError, ServerRequest, ServerResponse};

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
    T: ServiceFactory<ServerRequest, Response = ServerResponse, Error = ServerError>,
    T::Service: Clone,
{
    /// Create default server
    pub fn make_server(&self) -> GrpcService<T> {
        GrpcService {
            factory: self.factory.clone(),
        }
    }
}

impl<F, T> ServiceFactory<Io<F>> for GrpcServer<T>
where
    F: Filter,
    T: ServiceFactory<ServerRequest, Response = ServerResponse, Error = ServerError> + 'static,
    T::Service: Clone,
{
    type Response = ();
    type Error = T::InitError;
    type Service = GrpcService<T>;
    type InitError = ();

    async fn create(&self, _: ()) -> Result<Self::Service, Self::InitError> {
        Ok(self.make_server())
    }
}

pub struct GrpcService<T> {
    factory: Rc<T>,
}

impl<T, F> Service<Io<F>> for GrpcService<T>
where
    F: Filter,
    T: ServiceFactory<ServerRequest, Response = ServerResponse, Error = ServerError> + 'static,
{
    type Response = ();
    type Error = T::InitError;

    async fn call(&self, io: Io<F>, _: ServiceCtx<'_, Self>) -> Result<(), Self::Error> {
        // init server
        let service = self.factory.create(()).await?;

        let _ = h2::server::handle_one(
            io.into(),
            h2::Config::server(),
            ControlService,
            PublishService::new(service),
        )
        .await;

        Ok(())
    }
}

impl<T> Service<IoBoxed> for GrpcService<T>
where
    T: ServiceFactory<ServerRequest, Response = ServerResponse, Error = ServerError> + 'static,
{
    type Response = ();
    type Error = T::InitError;

    async fn call(&self, io: IoBoxed, _: ServiceCtx<'_, Self>) -> Result<(), Self::Error> {
        // init server
        let service = self.factory.create(()).await?;

        let _ = h2::server::handle_one(
            io,
            h2::Config::server(),
            ControlService,
            PublishService::new(service),
        )
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
        log::trace!("Control message: {:?}", msg);
        Ok::<_, ()>(msg.ack())
    }
}

struct PublishService<S: Service<ServerRequest>> {
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
    fn new(service: S) -> Self {
        Self {
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
                        let mut trailers = HeaderMap::default();
                        trailers.insert(consts::GRPC_STATUS, GrpcStatus::InvalidArgument.into());
                        trailers.insert(
                            consts::GRPC_MESSAGE,
                            HeaderValue::from_static("Cannot decode request message"),
                        );
                        stream.send_trailers(trailers);
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
                            let mut trailers = HeaderMap::default();
                            trailers
                                .insert(consts::GRPC_STATUS, GrpcStatus::InvalidArgument.into());
                            trailers.insert(
                                consts::GRPC_MESSAGE,
                                HeaderValue::from_static(
                                    "Cannot decode request message: not enough data provided",
                                ),
                            );
                            stream.send_trailers(trailers);
                        }
                        return Ok(());
                    }
                    let data = data
                        .split_to_checked(len as usize)
                        .ok_or(h2::StreamError::Reset(Reason::PROTOCOL_ERROR))?;

                    log::debug!("Call service {} method {}", inflight.service, inflight.name);
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

                    match ctx.call(&self.service, req).await {
                        Ok(res) => {
                            log::debug!("Response is received {:?}", res);
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
                        Err(err) => {
                            log::debug!("Failure during service call: {:?}", err.message);
                            let mut trailers = err.headers;
                            trailers.insert(consts::GRPC_STATUS, err.status.into());
                            trailers.insert(consts::GRPC_MESSAGE, err.message);
                            stream.send_trailers(trailers);
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
