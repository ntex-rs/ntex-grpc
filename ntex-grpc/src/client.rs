use std::{cell::RefCell, convert::TryFrom, future::Future, rc::Rc, str::FromStr};

use async_trait::async_trait;
use ntex_bytes::{Buf, BufMut, Bytes, BytesMut};
use ntex_connect::{Address, Connect, ConnectError, Connector as DefaultConnector};
use ntex_h2::{self as h2, client, frame::StreamId, Stream};
use ntex_http::{header, HeaderMap, Method};
use ntex_io::{IoBoxed, OnDisconnect};
use ntex_service::{fn_service, Service};
use ntex_util::{channel::oneshot, future::Ready, HashMap};

use crate::transport::{ClientInformation, MethodDef, Transport};
use crate::{consts, utils::Data, GrpcStatus, Message, Response, ServiceError, GRPC_STATUS};

#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    #[error("{0}")]
    Http(#[from] ntex_h2::client::ClientError),
}

#[derive(Clone)]
pub struct Client(Rc<Inner>);

struct Inner {
    client: client::Client,
    inflight: RefCell<HashMap<StreamId, Inflight>>,
}

struct Inflight {
    _stream: Stream,
    data: Data,
    headers: Option<HeaderMap>,
    tx: oneshot::Sender<Result<(Bytes, HeaderMap, HeaderMap), ServiceError>>,
}

impl Client {
    #[inline]
    /// Gracefully close connection
    pub fn close(&self) {
        self.0.client.close()
    }

    #[inline]
    /// Check if connection is closed
    pub fn is_closed(&self) -> bool {
        self.0.client.is_closed()
    }

    #[inline]
    /// Notify when connection get closed
    pub fn on_disconnect(&self) -> OnDisconnect {
        self.0.client.on_disconnect()
    }
}

#[async_trait(?Send)]
impl<T: MethodDef> Transport<T> for Client {
    type Error = ServiceError;

    async fn request(&self, val: &T::Input) -> Result<Response<T>, Self::Error> {
        let len = val.encoded_len();
        let mut buf = BytesMut::with_capacity(len + 5);
        buf.put_u8(0); // compression
        buf.put_u32(len as u32); // length
        val.encode(&mut buf)?;

        let mut hdrs = HeaderMap::new();
        hdrs.append(header::CONTENT_TYPE, consts::HDRV_CT_GRPC);
        hdrs.append(header::USER_AGENT, consts::HDRV_USER_AGENT);
        hdrs.insert(header::TE, consts::HDRV_TRAILERS);

        let stream = self
            .0
            .client
            .send_request(Method::POST, T::PATH, hdrs, false)
            .await?;
        stream.send_payload(buf.freeze(), true).await?;

        let (tx, rx) = oneshot::channel();
        self.0.inflight.borrow_mut().insert(
            stream.id(),
            Inflight {
                tx,
                _stream: stream,
                headers: None,
                data: Data::Empty,
            },
        );

        match rx.await {
            Ok(Ok((mut data, headers, trailers))) => {
                let _compressed = data.get_u8();
                let len = data.get_u32();
                match <T::Output as Message>::decode(&mut data.split_to(len as usize)) {
                    Ok(output) => Ok(Response {
                        output,
                        headers,
                        trailers,
                    }),
                    Err(_e) => Err(ServiceError::Canceled),
                }
            }
            Ok(Err(err)) => Err(err),
            Err(_) => Err(ServiceError::Canceled),
        }
    }
}

impl Inner {
    fn handle_message(&self, mut msg: h2::Message) -> Result<(), ()> {
        let id = msg.id();
        let mut inner = self.inflight.borrow_mut();

        if let Some(inflight) = inner.get_mut(&id) {
            match msg.kind().take() {
                h2::MessageKind::Headers {
                    headers,
                    pseudo,
                    eof,
                } => {
                    if let Some(status) = pseudo.status {
                        if eof {
                            let _ = inner
                                .remove(&id)
                                .unwrap()
                                .tx
                                .send(Err(ServiceError::UnexpectedEof(status, headers)));
                            return Err(());
                        } else if !status.is_success() {
                            let _ = inner
                                .remove(&id)
                                .unwrap()
                                .tx
                                .send(Err(ServiceError::Response(status, headers)));
                            return Err(());
                        } else {
                            inflight.headers = Some(headers);
                        }
                    } else {
                        let _ = inner
                            .remove(&id)
                            .unwrap()
                            .tx
                            .send(Err(ServiceError::UnknownResponseStatus(headers)));
                        return Err(());
                    }
                }
                h2::MessageKind::Data(data, _cap) => {
                    inflight.data.push(data);
                }
                h2::MessageKind::Eof(data) => {
                    let mut inflight = inner.remove(&id).unwrap();

                    let result = match data {
                        h2::StreamEof::Data(data) => {
                            inflight.data.push(data);
                            Ok((
                                inflight.data.get(),
                                inflight.headers.unwrap_or_default(),
                                HeaderMap::default(),
                            ))
                        }
                        h2::StreamEof::Trailers(hdrs) => {
                            // check grpc status
                            if let Some(val) = hdrs.get(GRPC_STATUS) {
                                if let Ok(status) = val
                                    .to_str()
                                    .map_err(|_| ())
                                    .and_then(|v| u8::from_str(v).map_err(|_| ()))
                                    .and_then(GrpcStatus::try_from)
                                {
                                    if status != GrpcStatus::Ok {
                                        let _ = inflight
                                            .tx
                                            .send(Err(ServiceError::GrpcStatus(status, hdrs)));
                                        return Err(());
                                    }
                                }
                            }

                            Ok((
                                inflight.data.get(),
                                inflight.headers.unwrap_or_default(),
                                hdrs,
                            ))
                        }
                        h2::StreamEof::Error(err) => Err(ServiceError::Stream(err)),
                    };
                    let _ = inflight.tx.send(result);
                }
                h2::MessageKind::Empty => {
                    inner.remove(&id);
                }
            }
        }
        Ok(())
    }
}

pub struct Connector<A, T>(Rc<client::Connector<A, T>>);

impl<A, T> Connector<A, T>
where
    A: Address,
{
    /// Create new grpc connector
    pub fn new(connector: client::Connector<A, T>) -> Connector<A, T> {
        Connector(Rc::new(connector))
    }
}

impl<A, T> Clone for Connector<A, T>
where
    A: Address,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<A> Default for Connector<A, DefaultConnector<A>>
where
    A: Address,
{
    fn default() -> Self {
        Connector::new(client::Connector::default())
    }
}

impl<A, T> From<client::Connector<A, T>> for Connector<A, T>
where
    A: Address,
    T: Service<Connect<A>, Error = ConnectError>,
    IoBoxed: From<T::Response>,
{
    fn from(connector: client::Connector<A, T>) -> Self {
        Self(Rc::new(connector))
    }
}

impl<A, T> Connector<A, T>
where
    A: Address,
    T: Service<Connect<A>, Error = ConnectError>,
    IoBoxed: From<T::Response>,
{
    /// Connect and create client instance
    pub fn create<C: ClientInformation<Client>>(
        &self,
        address: A,
    ) -> impl Future<Output = Result<C, ClientError>> {
        let fut = self.connect(address);

        async move { Ok(C::create(fut.await?)) }
    }

    /// Connect to http2 server
    pub fn connect(&self, address: A) -> impl Future<Output = Result<Client, ClientError>> {
        let slf = self.0.clone();
        async move {
            let con = slf.connect(address).await?;
            let client = con.client();
            let inner = Rc::new(Inner {
                client,
                inflight: RefCell::new(HashMap::default()),
            });

            let tr = inner.clone();
            ntex_rt::spawn(async move {
                let _ = con
                    .start(fn_service(move |msg: h2::Message| {
                        Ready::from(tr.handle_message(msg))
                    }))
                    .await;
            });
            Ok(Client(inner))
        }
    }
}
