use std::{convert::TryFrom, str::FromStr};

use ntex_bytes::{Buf, BufMut, BytesMut};
use ntex_h2::{self as h2};
use ntex_http::{header, HeaderMap, Method};
use ntex_util::future::BoxFuture;

use crate::{consts, service::MethodDef, utils::Data, DecodeError, GrpcStatus, Message};

use super::request::{RequestContext, Response};
use super::{Client, ClientError, Transport};

impl<T: MethodDef> Transport<T> for Client {
    type Error = ClientError;

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

            // send request
            let (snd_stream, rcv_stream) = self.0.send(Method::POST, T::PATH, hdrs, false).await?;
            snd_stream.send_payload(buf.freeze(), true).await?;

            // read response
            let mut status = None;
            let mut hdrs = HeaderMap::default();
            let mut trailers = HeaderMap::default();
            let mut payload = Data::Empty;

            loop {
                let mut msg = if let Some(msg) = rcv_stream.recv().await {
                    msg
                } else {
                    return Err(ClientError::UnexpectedEof(status, hdrs));
                };

                match msg.kind().take() {
                    h2::MessageKind::Headers {
                        headers,
                        pseudo,
                        eof,
                    } => {
                        if eof {
                            // check grpc status
                            match check_grpc_status(&headers) {
                                Some(Ok(status)) => {
                                    if status != GrpcStatus::Ok {
                                        return Err(ClientError::GrpcStatus(status, headers));
                                    }
                                }
                                Some(Err(())) => {
                                    return Err(ClientError::Decode(DecodeError::new(
                                        "Cannot parse grpc status",
                                    )));
                                }
                                None => {}
                            }

                            return Err(ClientError::UnexpectedEof(pseudo.status, headers));
                        } else {
                            hdrs = headers;
                            status = pseudo.status;
                        }
                        continue;
                    }
                    h2::MessageKind::Data(data, _cap) => {
                        payload.push(data);
                        continue;
                    }
                    h2::MessageKind::Eof(data) => {
                        match data {
                            h2::StreamEof::Data(data) => {
                                payload.push(data);
                            }
                            h2::StreamEof::Trailers(hdrs) => {
                                // check grpc status
                                match check_grpc_status(&hdrs) {
                                    Some(Ok(status)) => {
                                        if status != GrpcStatus::Ok {
                                            return Err(ClientError::GrpcStatus(status, hdrs));
                                        }
                                    }
                                    Some(Err(())) => {
                                        return Err(ClientError::Decode(DecodeError::new(
                                            "Cannot parse grpc status",
                                        )));
                                    }
                                    None => {}
                                }
                                trailers = hdrs;
                            }
                            h2::StreamEof::Error(err) => return Err(ClientError::Stream(err)),
                        };
                    }
                    h2::MessageKind::Disconnect(err) => return Err(ClientError::Operation(err)),
                    h2::MessageKind::Empty => {}
                }

                let mut data = payload.get();
                match status {
                    Some(st) => {
                        if !st.is_success() {
                            return Err(ClientError::Response(Some(st), hdrs, data));
                        }
                    }
                    None => return Err(ClientError::Response(None, hdrs, data)),
                }
                let _compressed = data.get_u8();
                let len = data.get_u32();
                return match <T::Output as Message>::read(&mut data.split_to(len as usize)) {
                    Ok(output) => Ok(Response {
                        output,
                        trailers,
                        req_size,
                        headers: hdrs,
                        res_size: data.len(),
                    }),
                    Err(e) => Err(ClientError::Decode(e)),
                };
            }
        })
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
