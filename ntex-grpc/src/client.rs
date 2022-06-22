use std::{cell::RefCell, future::Future, mem, rc::Rc};

use async_trait::async_trait;
use ntex_bytes::{Buf, BufMut, Bytes, BytesMut};
use ntex_connect::{Address, Connect, ConnectError, Connector as DefaultConnector};
use ntex_h2::{self as h2, client, frame::StreamId, Stream};
use ntex_http::{header, HeaderMap, Method};
use ntex_io::IoBoxed;
use ntex_service::{fn_service, IntoService, Service};
use ntex_util::{channel::oneshot, future::Ready, HashMap};
use prost::Message;

use crate::{consts, service::MethodDef, service::Transport, ServiceError};

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
    tx: oneshot::Sender<Result<(Bytes, HeaderMap), ServiceError>>,
}

enum Data {
    Chunk(Bytes),
    MutChunk(BytesMut),
    Empty,
}

impl Data {
    fn get(&mut self) -> Bytes {
        match mem::replace(self, Data::Empty) {
            Data::Chunk(data) => data,
            Data::MutChunk(data) => data.freeze(),
            Data::Empty => Bytes::new(),
        }
    }

    fn push(&mut self, data: Bytes) {
        if !data.is_empty() {
            *self = match mem::replace(self, Data::Empty) {
                Data::Chunk(d) => {
                    let mut d = BytesMut::from(d);
                    d.extend_from_slice(&data);
                    Data::MutChunk(d)
                }
                Data::MutChunk(mut d) => {
                    d.extend_from_slice(&data);
                    Data::MutChunk(d)
                }
                Data::Empty => Data::Chunk(data),
            };
        }
    }
}

#[async_trait(?Send)]
impl Transport for Client {
    type Error = ServiceError;

    async fn request<T: MethodDef>(
        &self,
        val: T::Input,
    ) -> Result<(T::Output, HeaderMap), Self::Error> {
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
            .send_request(Method::POST, T::PATH, hdrs)
            .await?;
        stream.send_payload(buf.freeze(), true).await?;

        let (tx, rx) = oneshot::channel();
        self.0.inflight.borrow_mut().insert(
            stream.id(),
            Inflight {
                tx,
                _stream: stream,
                data: Data::Empty,
            },
        );

        match rx.await {
            Ok(Ok((mut data, trailers))) => {
                let _compressed = data.get_u8();
                let len = data.get_u32();
                match <T::Output as Message>::decode(data.split_to(len as usize)) {
                    Ok(item) => Ok((item, trailers)),
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
                h2::MessageKind::Headers { .. } => {
                    // println!("Got response: {:#?}\nheaders: {:#?}", pseudo, headers);
                }
                h2::MessageKind::Data(data, _cap) => {
                    inflight.data.push(data);
                }
                h2::MessageKind::Eof(data) => {
                    let result = match data {
                        h2::StreamEof::Data(data) => {
                            inflight.data.push(data);
                            Ok((inflight.data.get(), HeaderMap::default()))
                        }
                        h2::StreamEof::Trailers(hdrs) => Ok((inflight.data.get(), hdrs)),
                        h2::StreamEof::Error(err) => Err(ServiceError::Stream(err)),
                    };
                    let _ = inner.remove(&id).unwrap().tx.send(result);
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

impl<A> Connector<A, ()>
where
    A: Address,
{
    #[allow(clippy::new_ret_no_self)]
    /// Create new h2 connector
    pub fn new() -> Connector<A, DefaultConnector<A>> {
        Connector(Rc::new(client::Connector::new()))
    }
}

impl<A, T> Connector<A, T>
where
    A: Address,
{
    /// Use custom connector
    pub fn connector<U, F>(self, connector: F) -> Connector<A, U>
    where
        F: IntoService<U, Connect<A>>,
        U: Service<Connect<A>, Error = ConnectError>,
        IoBoxed: From<U::Response>,
    {
        Connector(Rc::new(self.0.connector(connector)))
    }
}

impl<A, T> Connector<A, T>
where
    A: Address,
    T: Service<Connect<A>, Error = ConnectError>,
    IoBoxed: From<T::Response>,
{
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
