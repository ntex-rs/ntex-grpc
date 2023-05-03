use std::{cell::RefCell, convert::TryFrom, str::FromStr};

use ntex_bytes::{Buf, BufMut, Bytes, BytesMut};
use ntex_h2::{self as h2, client, frame::Reason, frame::StreamId, Stream, StreamRef};
use ntex_http::{header, HeaderMap, Method, StatusCode};
use ntex_util::{channel::oneshot, future::BoxFuture, HashMap};

use crate::service::MethodDef;
use crate::{consts, utils::Data, DecodeError, GrpcStatus, Message, ServiceError};

use super::request::{RequestContext, Response};
use super::{Client, Transport};

pub(super) struct Inner {
    pub(super) client: client::Client,
    pub(super) inflight: RefCell<HashMap<StreamId, Inflight>>,
}

pub(super) struct Inflight {
    _stream: Stream,
    data: Data,
    status: Option<StatusCode>,
    headers: Option<HeaderMap>,
    tx: oneshot::Sender<Result<(Option<StatusCode>, Bytes, HeaderMap, HeaderMap), ServiceError>>,
}

impl<T: MethodDef> Transport<T> for Client {
    type Error = ServiceError;

    type Future<'f> = BoxFuture<'f, Result<Response<T>, Self::Error>>
    where Self: 'f,
          T::Input: 'f;

    fn request<'a>(&'a self, val: &'a T::Input, ctx: RequestContext) -> Self::Future<'a> {
        Box::pin(async move {
            let len = val.encoded_len();
            let mut buf = BytesMut::with_capacity(len + 5);
            buf.put_u8(0); // compression
            buf.put_u32(len as u32); // length
            val.write(&mut buf);
            let req_size = buf.len();

            let mut hdrs = HeaderMap::new();
            hdrs.append(header::CONTENT_TYPE, consts::HDRV_CT_GRPC);
            hdrs.append(header::USER_AGENT, consts::HDRV_USER_AGENT);
            hdrs.insert(header::TE, consts::HDRV_TRAILERS);
            hdrs.insert(consts::GRPC_ENCODING, consts::IDENTITY);
            hdrs.insert(consts::GRPC_ACCEPT_ENCODING, consts::IDENTITY);
            for (key, val) in ctx.headers() {
                hdrs.insert(key.clone(), val.clone())
            }

            let stream = self
                .0
                .client
                .send_request(Method::POST, T::PATH, hdrs, false)
                .await?;

            let s_ref = (*stream).clone();
            let (tx, rx) = oneshot::channel();
            self.0.inflight.borrow_mut().insert(
                stream.id(),
                Inflight {
                    tx,
                    _stream: stream,
                    status: None,
                    headers: None,
                    data: Data::Empty,
                },
            );
            let hnd = StreamHnd(&s_ref, &self.0);
            s_ref.send_payload(buf.freeze(), true).await?;

            let result = match rx.await {
                Ok(Ok((status, mut data, headers, trailers))) => {
                    match status {
                        Some(st) => {
                            if !st.is_success() {
                                return Err(ServiceError::Response(Some(st), headers, data));
                            }
                        }
                        None => return Err(ServiceError::Response(None, headers, data)),
                    }
                    let _compressed = data.get_u8();
                    let len = data.get_u32();
                    match <T::Output as Message>::read(&mut data.split_to(len as usize)) {
                        Ok(output) => Ok(Response {
                            output,
                            headers,
                            trailers,
                            req_size,
                            res_size: data.len(),
                        }),
                        Err(e) => Err(ServiceError::Decode(e)),
                    }
                }
                Ok(Err(err)) => Err(err),
                Err(_) => Err(ServiceError::Canceled),
            };
            drop(hnd);
            result
        })
    }
}

struct StreamHnd<'a>(&'a StreamRef, &'a Inner);

impl<'a> Drop for StreamHnd<'a> {
    fn drop(&mut self) {
        self.0.reset(Reason::CANCEL);
        self.1.inflight.borrow_mut().remove(&self.0.id());
    }
}

impl Inner {
    pub(super) fn handle_message(&self, mut msg: h2::Message) -> Result<(), ()> {
        let id = msg.id();
        let mut inner = self.inflight.borrow_mut();

        if let Some(inflight) = inner.get_mut(&id) {
            match msg.kind().take() {
                h2::MessageKind::Headers {
                    headers,
                    pseudo,
                    eof,
                } => {
                    if eof {
                        let tx = inner.remove(&id).unwrap().tx;

                        // check grpc status
                        match check_grpc_status(&headers) {
                            Some(Ok(status)) => {
                                if status != GrpcStatus::Ok {
                                    let _ =
                                        tx.send(Err(ServiceError::GrpcStatus(status, headers)));
                                    return Err(());
                                }
                            }
                            Some(Err(())) => {
                                let _ = tx.send(Err(ServiceError::Decode(DecodeError::new(
                                    "Cannot parse grpc status",
                                ))));
                                return Err(());
                            }
                            None => {}
                        }

                        let _ = tx.send(Err(ServiceError::UnexpectedEof(pseudo.status, headers)));
                        return Err(());
                    } else {
                        inflight.status = pseudo.status;
                        inflight.headers = Some(headers);
                    }
                }
                h2::MessageKind::Data(data, _cap) => {
                    inflight.data.push(data);
                }
                h2::MessageKind::Eof(data) => {
                    let mut inflight = inner.remove(&id).unwrap();
                    let tx = inflight.tx;

                    let result = match data {
                        h2::StreamEof::Data(data) => {
                            inflight.data.push(data);
                            Ok((
                                inflight.status,
                                inflight.data.get(),
                                inflight.headers.unwrap_or_default(),
                                HeaderMap::default(),
                            ))
                        }
                        h2::StreamEof::Trailers(hdrs) => {
                            // check grpc status
                            match check_grpc_status(&hdrs) {
                                Some(Ok(status)) => {
                                    if status != GrpcStatus::Ok {
                                        let _ =
                                            tx.send(Err(ServiceError::GrpcStatus(status, hdrs)));
                                        return Err(());
                                    }
                                }
                                Some(Err(())) => {
                                    let _ = tx.send(Err(ServiceError::Decode(DecodeError::new(
                                        "Cannot parse grpc status",
                                    ))));
                                    return Err(());
                                }
                                None => {}
                            }

                            Ok((
                                inflight.status,
                                inflight.data.get(),
                                inflight.headers.unwrap_or_default(),
                                hdrs,
                            ))
                        }
                        h2::StreamEof::Error(err) => Err(ServiceError::Stream(err)),
                    };
                    let _ = tx.send(result);
                }
                h2::MessageKind::Disconnect(err) => {
                    let _ = inner
                        .remove(&id)
                        .unwrap()
                        .tx
                        .send(Err(ServiceError::Operation(err)));
                }
                h2::MessageKind::Empty => {
                    inner.remove(&id);
                }
            }
        }
        Ok(())
    }
}

fn check_grpc_status(hdrs: &HeaderMap) -> Option<Result<GrpcStatus, ()>> {
    // check grpc status
    if let Some(val) = hdrs.get(consts::GRPC_STATUS) {
        if let Ok(status) = val
            .to_str()
            .map_err(|_| ())
            .and_then(|v| u8::from_str(v).map_err(|_| ()))
            .and_then(GrpcStatus::try_from)
        {
            Some(Ok(status))
        } else {
            Some(Err(()))
        }
    } else {
        None
    }
}
