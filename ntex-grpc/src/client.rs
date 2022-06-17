use std::{cell::RefCell, future::Future, mem, rc::Rc};

use async_trait::async_trait;
use ntex::channel::oneshot;
use ntex::connect::{Address, Connect, ConnectError, Connector as DefaultConnector};
use ntex::http::{header, HeaderMap, Method};
use ntex::io::IoBoxed;
use ntex::service::{fn_service, IntoService, Service};
use ntex::util::{Buf, BufMut, Bytes, BytesMut, HashMap, Ready};
use ntex_h2 as h2;
use ntex_h2::client::{Client, Connector};
use ntex_h2::{frame::StreamId, Stream};
use prost::Message;

use crate::{consts, service::MethodDef, service::Transport, ServiceError};

#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    #[error("{0}")]
    Http(#[from] ntex_h2::client::ClientError),
}

#[derive(Clone)]
pub struct ClientTransport(Rc<Inner>);

struct Inner {
    client: Client,
    inflight: RefCell<HashMap<StreamId, Inflight>>,
}

struct Inflight {
    _stream: Stream,
    data: Data,
    tx: oneshot::Sender<Result<(Bytes, HeaderMap), ServiceError>>,
}

enum Data {
    Data(Bytes),
    MutData(BytesMut),
    Empty,
}

impl Data {
    fn get(&mut self) -> Bytes {
        match mem::replace(self, Data::Empty) {
            Data::Data(data) => data,
            Data::MutData(data) => data.freeze(),
            Data::Empty => Bytes::new(),
        }
    }

    fn push(&mut self, data: Bytes) {
        *self = match mem::replace(self, Data::Empty) {
            Data::Data(d) => {
                let mut d = BytesMut::from(d);
                d.extend_from_slice(&data);
                Data::MutData(d)
            }
            Data::MutData(mut d) => {
                d.extend_from_slice(&data);
                Data::MutData(d)
            }
            Data::Empty => Data::Data(data),
        };
    }
}

#[async_trait(?Send)]
impl Transport for ClientTransport {
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

        let stream = self.0.client.send_request(Method::POST, T::PATH, hdrs);
        stream.send_data(buf.freeze(), true);

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
                h2::MessageKind::Headers {
                    pseudo, headers, ..
                } => {
                    // println!("Got response: {:#?}\nheaders: {:#?}", pseudo, headers);
                }
                h2::MessageKind::Data(data) => {
                    inflight.data.push(data);
                }
                h2::MessageKind::Eof(data) => {
                    let result = match data {
                        h2::StreamEof::Data(data) => {
                            inflight.data.push(data);
                            Ok((inflight.data.get(), HeaderMap::default()))
                        }
                        h2::StreamEof::Trailers(hdrs) => Ok((inflight.data.get(), hdrs)),
                        h2::StreamEof::Reset(reason) => Err(ServiceError::H2Reset(reason)),
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

pub struct ClientConnector<A, T>(Connector<A, T>);

impl<A> ClientConnector<A, ()>
where
    A: Address,
{
    #[allow(clippy::new_ret_no_self)]
    /// Create new h2 connector
    pub fn new() -> ClientConnector<A, DefaultConnector<A>> {
        ClientConnector(Connector::new())
    }
}

impl<A, T> ClientConnector<A, T>
where
    A: Address,
{
    /// Use custom connector
    pub fn connector<U, F>(self, connector: F) -> ClientConnector<A, U>
    where
        F: IntoService<U, Connect<A>>,
        U: Service<Connect<A>, Error = ConnectError>,
        IoBoxed: From<U::Response>,
    {
        ClientConnector(self.0.connector(connector))
    }
}

impl<A, T> ClientConnector<A, T>
where
    A: Address,
    T: Service<Connect<A>, Error = ConnectError>,
    IoBoxed: From<T::Response>,
{
    /// Connect to http2 server
    pub fn connect(
        &self,
        address: A,
    ) -> impl Future<Output = Result<ClientTransport, ClientError>> {
        let fut = self.0.connect(address);
        async move {
            let con = fut.await?;
            let client = con.client();
            let inner = Rc::new(Inner {
                client,
                inflight: RefCell::new(HashMap::default()),
            });

            let tr = inner.clone();
            ntex::rt::spawn(async move {
                let _ = con
                    .start(fn_service(move |msg: h2::Message| {
                        Ready::from(tr.handle_message(msg))
                    }))
                    .await;
            });
            Ok(ClientTransport(inner))
        }
    }
}
