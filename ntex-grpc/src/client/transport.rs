use std::{convert::TryFrom, future::Future, str::FromStr, time::Duration};

use ntex_bytes::{Buf, BufMut, BytePages};
use ntex_error::Error;
use ntex_h2::{self as h2};
use ntex_http::{HeaderMap, HeaderValue, Method, header};
use ntex_util::time;

use super::{Client, ClientError, Transport, request::RequestContext, request::Response};
use crate::{DecodeError, GrpcStatus, Message, consts, service::MethodDef, utils::Data};

impl<T: MethodDef> Transport<T> for Client {
    type Error = Error<ClientError>;

    #[inline]
    async fn request(
        &self,
        val: &T::Input,
        ctx: &mut RequestContext,
    ) -> Result<Response<T>, Self::Error> {
        Transport::request(&self.0, val, ctx).await
    }
}

impl<T: MethodDef> Transport<T> for h2::client::Client {
    type Error = Error<ClientError>;

    /// The timeout covers waiting for a connection too.
    async fn request(
        &self,
        val: &T::Input,
        ctx: &mut RequestContext,
    ) -> Result<Response<T>, Self::Error> {
        with_deadline(ctx.get_timeout(), async {
            let client = self.client().await.map_err(|e| e.map(ClientError::from))?;
            send_request(&client, val, ctx).await
        })
        .await
    }
}

impl<T: MethodDef> Transport<T> for h2::client::SimpleClient {
    type Error = Error<ClientError>;

    async fn request(
        &self,
        val: &T::Input,
        ctx: &mut RequestContext,
    ) -> Result<Response<T>, Self::Error> {
        with_deadline(ctx.get_timeout(), send_request(self, val, ctx))
            .await
            .map_err(|e| e.with_service(self.service()))
    }
}

/// Stop waiting once the request timeout runs out.
///
/// The pending stream is dropped, which resets it with `CANCEL`.
async fn with_deadline<R>(
    timeout: Option<Duration>,
    fut: impl Future<Output = Result<R, Error<ClientError>>>,
) -> Result<R, Error<ClientError>> {
    if let Some(timeout) = timeout {
        time::timeout(timeout, fut)
            .await
            .unwrap_or_else(|()| Err(Error::from(ClientError::DeadlineExceeded(HeaderMap::new()))))
    } else {
        fut.await
    }
}

#[allow(clippy::too_many_lines)]
async fn send_request<T: MethodDef>(
    client: &h2::client::SimpleClient,
    val: &T::Input,
    ctx: &mut RequestContext,
) -> Result<Response<T>, Error<ClientError>> {
    let len = val.encoded_len();
    let mut buf = BytePages::default();
    buf.put_u8(0); // compression
    buf.put_u32(len as u32); // length
    val.write(&mut buf);
    let req_size = buf.len();

    if let Some(err) = ctx.take_error() {
        return Err(Error::from(ClientError::Http(err)).with_service(client.service()));
    }

    let mut hdrs = HeaderMap::new();
    hdrs.append(header::CONTENT_TYPE, consts::HDRV_CT_GRPC);
    hdrs.append(header::USER_AGENT, consts::HDRV_USER_AGENT);
    hdrs.insert(header::TE, consts::HDRV_TRAILERS);
    hdrs.insert(consts::GRPC_ENCODING, consts::IDENTITY);
    hdrs.insert(consts::GRPC_ACCEPT_ENCODING, consts::IDENTITY);
    for (key, val) in ctx.headers() {
        hdrs.insert(key.clone(), val.clone());
    }

    // send request
    let (snd_stream, rcv_stream) = client
        .send(Method::POST, T::PATH, hdrs, false)
        .await
        .map_err(|e| e.map(ClientError::from))?;
    if ctx.get_disconnect_on_drop() {
        snd_stream.disconnect_on_drop();
    }
    snd_stream
        .send_pages(buf, true)
        .await
        .map_err(|e| e.map(ClientError::from))?;

    // read response
    let mut status = None;
    let mut hdrs = HeaderMap::default();
    let mut trailers = HeaderMap::default();
    let mut payload = Data::Empty;
    // the status to report if the response has no `grpc-status`
    let mut missing = None;

    async {
        loop {
            let Some(msg) = rcv_stream.recv().await else {
                return Err(Error::from(ClientError::UnexpectedEof(status, hdrs)));
            };

            match msg.kind {
                h2::MessageKind::Headers {
                    headers,
                    pseudo,
                    eof,
                } => {
                    if eof {
                        // check grpc status
                        match check_grpc_status(&headers) {
                            Some(Ok(GrpcStatus::DeadlineExceeded)) => {
                                return Err(Error::from(ClientError::DeadlineExceeded(headers)));
                            }
                            Some(Ok(status)) if status != GrpcStatus::Ok => {
                                return Err(Error::from(ClientError::GrpcStatus(status, headers)));
                            }
                            Some(Err(())) => {
                                return Err(Error::from(ClientError::Decode(DecodeError::new(
                                    "Cannot parse grpc status",
                                ))));
                            }
                            Some(Ok(_)) | None => {}
                        }

                        return Err(Error::from(ClientError::UnexpectedEof(
                            pseudo.status,
                            headers,
                        )));
                    }
                    hdrs = headers;
                    status = pseudo.status;
                    continue;
                }
                h2::MessageKind::Data(data, _cap) => {
                    payload.push(data);
                    continue;
                }
                h2::MessageKind::Eof(data) => {
                    match data {
                        h2::StreamEof::Data(data, _cap) => {
                            payload.push(data);
                            missing = Some((GrpcStatus::Internal, NO_TRAILERS));
                        }
                        h2::StreamEof::Trailers(hdrs) => {
                            // check grpc status
                            match check_grpc_status(&hdrs) {
                                Some(Ok(GrpcStatus::Ok)) => Ok(()),
                                None => {
                                    missing = Some((GrpcStatus::Unknown, NO_GRPC_STATUS));
                                    Ok(())
                                }
                                Some(Ok(GrpcStatus::DeadlineExceeded)) => {
                                    return Err(Error::from(ClientError::DeadlineExceeded(hdrs)));
                                }
                                Some(Ok(st)) => {
                                    return Err(Error::from(ClientError::GrpcStatus(st, hdrs)));
                                }
                                Some(Err(())) => Err(Error::from(ClientError::Decode(
                                    DecodeError::new("Cannot parse grpc status"),
                                ))),
                            }?;
                            trailers = hdrs;
                        }
                        h2::StreamEof::Error(err) => {
                            return Err(err.map(ClientError::Stream));
                        }
                    }
                }
                h2::MessageKind::Disconnect(err) => {
                    return Err(err.map(ClientError::Operation));
                }
            }

            let mut data = payload.get();
            match status {
                Some(st) => {
                    if !st.is_success() {
                        return Err(Error::from(ClientError::Response(Some(st), hdrs, data)));
                    }
                }
                None => return Err(Error::from(ClientError::Response(None, hdrs, data))),
            }
            if let Some((status, msg)) = missing {
                if !trailers.contains_key(consts::GRPC_MESSAGE) {
                    trailers.insert(consts::GRPC_MESSAGE, HeaderValue::from_static(msg));
                }
                return Err(Error::from(ClientError::GrpcStatus(status, trailers)));
            }
            let resp_size = data.len();
            if resp_size < 5 {
                return Err(Error::from(ClientError::UnexpectedEof(status, hdrs)));
            }
            let _compressed = data.get_u8();
            let len = data.get_u32();
            let Some(mut block) = data.split_to_checked(len as usize) else {
                return Err(Error::from(ClientError::UnexpectedEof(None, hdrs)));
            };

            return match <T::Output as Message>::read(&mut block) {
                Ok(output) => Ok(Response {
                    output,
                    trailers,
                    req_size,
                    headers: hdrs,
                    res_size: resp_size,
                }),
                Err(e) => Err(Error::from(ClientError::Decode(e))),
            };
        }
    }
    .await
    .map_err(|e| e.with_service(client.service()))
}

const NO_TRAILERS: &str = "Response ended without trailers";
const NO_GRPC_STATUS: &str = "Response trailers have no grpc-status";

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
