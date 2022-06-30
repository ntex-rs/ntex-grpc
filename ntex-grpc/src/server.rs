#![allow(unused_variables, unused_must_use)]
use std::{cell::RefCell, future::Future, pin::Pin, rc::Rc, task::Context, task::Poll};

use ntex_bytes::{Buf, BufMut};
use ntex_h2::{self as h2, frame::StreamId};
use ntex_http::{HeaderMap, StatusCode};
use ntex_io::{Filter, Io, IoBoxed};
use ntex_util::{future::Either, future::Ready, HashMap};

pub use ntex_bytes::{ByteString, Bytes, BytesMut};
pub use ntex_service::{Service, ServiceFactory};

pub use crate::error::ServerError;
use crate::utils::Data;

#[derive(Debug)]
pub struct Request {
    pub name: ByteString,
    pub payload: Bytes,
}

#[derive(Debug)]
pub struct Response {
    pub payload: Bytes,
}

impl Response {
    #[inline]
    pub fn new(payload: Bytes) -> Response {
        Response { payload }
    }
}

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
    T: ServiceFactory<Request, Response = Response, Error = ServerError>,
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
    T: ServiceFactory<Request, Response = Response, Error = ServerError> + 'static,
    T::Service: Clone,
{
    type Response = ();
    type Error = T::InitError;
    type Service = GrpcService<T>;
    type InitError = ();
    type Future = Ready<Self::Service, Self::InitError>;

    fn new_service(&self, _: ()) -> Self::Future {
        Ready::Ok(self.make_server())
    }
}

pub struct GrpcService<T> {
    factory: Rc<T>,
}

impl<T, F> Service<Io<F>> for GrpcService<T>
where
    F: Filter,
    T: ServiceFactory<Request, Response = Response, Error = ServerError> + 'static,
{
    type Response = ();
    type Error = T::InitError;
    type Future = Pin<Box<dyn Future<Output = Result<(), Self::Error>>>>;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&self, io: Io<F>) -> Self::Future {
        let fut = self.factory.new_service(());

        Box::pin(async move {
            // init server
            let service = fut.await?;

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
    T: ServiceFactory<Request, Response = Response, Error = ServerError> + 'static,
{
    type Response = ();
    type Error = T::InitError;
    type Future = Pin<Box<dyn Future<Output = Result<(), Self::Error>>>>;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&self, io: IoBoxed) -> Self::Future {
        let fut = self.factory.new_service(());

        Box::pin(async move {
            // init server
            let service = fut.await?;

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
    type Future = Ready<Self::Response, Self::Error>;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[inline]
    fn poll_shutdown(&self, _: &mut Context<'_>, _: bool) -> Poll<()> {
        Poll::Ready(())
    }

    fn call(&self, msg: h2::ControlMessage<h2::StreamError>) -> Self::Future {
        log::trace!("Control message: {:?}", msg);
        Ready::Ok::<_, ()>(msg.ack())
    }
}

struct PublishService<S: Service<Request>> {
    service: S,
    streams: RefCell<HashMap<StreamId, Inflight>>,
}

struct Inflight {
    name: ByteString,
    service: ByteString,
    data: Data,
}

impl<S> PublishService<S>
where
    S: Service<Request, Response = Response, Error = ServerError>,
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
    S: Service<Request, Response = Response, Error = ServerError> + 'static,
{
    type Response = ();
    type Error = h2::StreamError;
    type Future = Either<
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>,
        Ready<Self::Response, Self::Error>,
    >;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[inline]
    fn poll_shutdown(&self, _: &mut Context<'_>, _: bool) -> Poll<()> {
        Poll::Ready(())
    }

    fn call(&self, mut msg: h2::Message) -> Self::Future {
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
                    // TODO: return not found
                    return Either::Right(Ready::Ok(()));
                };
                let mut path = path.split_off(1);
                let methodname = if let Some(n) = path.find('/') {
                    path.split_to(n)
                } else {
                    path
                };

                let method = pseudo.method.unwrap();
                let _ = streams.insert(
                    msg.id(),
                    Inflight {
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
                let mut inflight = streams.remove(&id).unwrap();

                let result = match data {
                    h2::StreamEof::Data(chunk) => inflight.data.push(chunk),
                    h2::StreamEof::Trailers(hdrs) => (),
                    h2::StreamEof::Error(err) => return Either::Right(Ready::Err(err)),
                };

                let mut data = inflight.data.get();
                let _compressed = data.get_u8();
                let len = data.get_u32();
                let data = data.split_to(len as usize);

                log::debug!("Call service {} method {}", inflight.service, inflight.name);
                let req = Request {
                    payload: data,
                    name: inflight.name,
                };

                let fut = self.service.call(req);
                return Either::Left(Box::pin(async move {
                    match fut.await {
                        Ok(res) => {
                            let mut buf = BytesMut::with_capacity(res.payload.len() + 5);
                            buf.put_u8(0); // compression
                            buf.put_u32(res.payload.len() as u32); // length
                            buf.extend_from_slice(&res.payload);

                            msg.stream().send_response(
                                StatusCode::OK,
                                HeaderMap::default(),
                                false,
                            );
                            msg.stream().send_payload(buf.freeze(), true).await;
                        }
                        Err(err) => (),
                    };

                    Ok(())
                }));
            }
            h2::MessageKind::Empty => {
                streams.remove(&id);
            }
        }
        Either::Right(Ready::Ok(()))
    }
}
