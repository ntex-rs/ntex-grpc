use std::{cell::RefCell, future::Future, rc::Rc};

use ntex_connect::{Address, Connect, ConnectError, Connector as DefaultConnector};
use ntex_h2::{self as h2, client};
use ntex_io::IoBoxed;
use ntex_service::{fn_service, Pipeline, Service};
use ntex_util::{future::Ready, HashMap};

use crate::client::{transport::Inner, Client, ClientError, ClientInformation};

pub struct Connector<A: Address, T>(Pipeline<client::Connector<A, T>>);

impl<A, T> Connector<A, T>
where
    A: Address,
{
    /// Create new grpc connector
    pub fn new(connector: client::Connector<A, T>) -> Connector<A, T> {
        Connector(Pipeline::new(connector))
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
        Self(Pipeline::new(connector))
    }
}

impl<A, T> Connector<A, T>
where
    A: Address,
    T: Service<Connect<A>, Error = ConnectError>,
    IoBoxed: From<T::Response>,
{
    /// Connect and create client instance
    pub async fn create<C: ClientInformation<Client>>(
        &self,
        address: A,
    ) -> Result<C, ClientError> {
        Ok(C::create(self.connect(address).await?))
    }

    /// Connect to http2 server
    pub async fn connect(&self, address: A) -> Result<Client, ClientError> {
        let con = self.0.get_ref().connect(address).await?;
        let inner = Rc::new(Inner {
            client: con.client(),
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
