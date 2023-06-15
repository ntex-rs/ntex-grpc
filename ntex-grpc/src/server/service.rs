use std::{cell::RefCell, rc::Rc};

use ntex_bytes::{Buf, BufMut, ByteString, BytesMut};
use ntex_h2::{self as h2, frame::StreamId};
use ntex_http::{HeaderMap, HeaderValue, StatusCode};
use ntex_io::{Filter, Io, IoBoxed};
use ntex_service::{Ctx, Service, ServiceFactory};
use ntex_util::{future::BoxFuture, future::Either, future::Ready, HashMap};

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
    type Future<'f> = Ready<Self::Service, Self::InitError>;

    fn create(&self, _: ()) -> Self::Future<'_> {
        Ready::Ok(self.make_server())
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
    type Future<'f> = BoxFuture<'f, Result<(), Self::Error>>;

    fn call<'a>(&'a self, io: Io<F>, _: Ctx<'a, Self>) -> Self::Future<'a> {
        Box::pin(async move {
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
        })
    }
}

impl<T> Service<IoBoxed> for GrpcService<T>
where
    T: ServiceFactory<ServerRequest, Response = ServerResponse, Error = ServerError> + 'static,
{
    type Response = ();
    type Error = T::InitError;
    type Future<'f> = BoxFuture<'f, Result<(), Self::Error>>;

    fn call<'a>(&'a self, io: IoBoxed, _: Ctx<'a, Self>) -> Self::Future<'a> {
        Box::pin(async move {
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
        })
    }
}

struct ControlService;

impl Service<h2::ControlMessage<h2::StreamError>> for ControlService {
    type Response = h2::ControlResult;
    type Error = ();
    type Future<'f> = Ready<Self::Response, Self::Error>;

    fn call<'a>(
        &'a self,
        msg: h2::ControlMessage<h2::StreamError>,
        _: Ctx<'a, Self>,
    ) -> Self::Future<'a> {
        log::trace!("Control message: {:?}", msg);
        Ready::Ok::<_, ()>(msg.ack())
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
    type Future<'f> = Either<
        BoxFuture<'f, Result<Self::Response, Self::Error>>,
        Ready<Self::Response, Self::Error>,
    >;

    fn call<'a>(&'a self, mut msg: h2::Message, ctx: Ctx<'a, Self>) -> Self::Future<'a> {
        let id = msg.id();
        let mut streams = self.streams.borrow_mut();

        match msg.kind().take() {
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
                    let _ = msg.stream().send_response(
                        StatusCode::NOT_FOUND,
                        HeaderMap::default(),
                        true,
                    );
                    return Either::Right(Ready::Ok(()));
                };

                // stream eof, cannot do anything
                if eof {
                    if msg
                        .stream()
                        .send_response(StatusCode::OK, HeaderMap::default(), false)
                        .is_ok()
                    {
                        let mut trailers = HeaderMap::default();
                        trailers.insert(consts::GRPC_STATUS, GrpcStatus::InvalidArgument.into());
                        trailers.insert(
                            consts::GRPC_MESSAGE,
                            HeaderValue::from_static("Cannot decode request message"),
                        );
                        msg.stream().send_trailers(trailers);
                    }
                    return Either::Right(Ready::Ok(()));
                }

                let mut path = path.split_off(1);
                let methodname = if let Some(n) = path.find('/') {
                    path.split_to(n)
                } else {
                    path
                };

                let _ = streams.insert(
                    msg.id(),
                    Inflight {
                        headers,
                        data: Data::Empty,
                        name: methodname,
                        service: srvname,
                    },
                );
            }
            h2::MessageKind::Data(data, _cap) => {
                if let Some(inflight) = streams.get_mut(&msg.id()) {
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
                        h2::StreamEof::Error(err) => return Either::Right(Ready::Err(err)),
                    }

                    let mut data = inflight.data.get();
                    let _compressed = data.get_u8();
                    let len = data.get_u32();
                    if (len as usize) > data.len() {
                        if msg
                            .stream()
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
                            msg.stream().send_trailers(trailers);
                        }
                        return Either::Right(Ready::Ok(()));
                    }
                    let data = data.split_to(len as usize);

                    log::debug!("Call service {} method {}", inflight.service, inflight.name);
                    let req = ServerRequest {
                        payload: data,
                        name: inflight.name,
                        headers: inflight.headers,
                    };
                    if msg
                        .stream()
                        .send_response(StatusCode::OK, HeaderMap::default(), false)
                        .is_err()
                    {
                        return Either::Right(Ready::Ok(()));
                    }

                    return Either::Left(Box::pin(async move {
                        match ctx.call(&self.service, req).await {
                            Ok(res) => {
                                log::debug!("Response is received {:?}", res);
                                let mut buf = BytesMut::with_capacity(res.payload.len() + 5);
                                buf.put_u8(0); // compression
                                buf.put_u32(res.payload.len() as u32); // length
                                buf.extend_from_slice(&res.payload);

                                let _ = msg.stream().send_payload(buf.freeze(), false).await;

                                let mut trailers = HeaderMap::default();
                                trailers.insert(consts::GRPC_STATUS, GrpcStatus::Ok.into());
                                for (name, val) in res.headers {
                                    trailers.append(name, val);
                                }

                                msg.stream().send_trailers(trailers);
                            }
                            Err(err) => {
                                let error = format!("Failure during service call: {}", err);
                                log::debug!("{}", error);
                                let mut trailers = HeaderMap::default();
                                trailers.insert(consts::GRPC_STATUS, GrpcStatus::Aborted.into());
                                if let Ok(val) = HeaderValue::from_str(&error) {
                                    trailers.insert(consts::GRPC_MESSAGE, val);
                                }
                                msg.stream().send_trailers(trailers);
                            }
                        };

                        Ok(())
                    }));
                }
            }
            h2::MessageKind::Disconnect(_) | h2::MessageKind::Empty => {
                streams.remove(&id);
            }
        }
        Either::Right(Ready::Ok(()))
    }
}
