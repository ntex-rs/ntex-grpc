use std::{convert::TryFrom, future::Future, str::FromStr, time::Duration};

use ntex_bytes::{Buf, BufMut, BytePages, Bytes};
use ntex_error::Error;
use ntex_h2::{self as h2};
use ntex_http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use ntex_util::time;

use super::{Client, ClientError, Transport, request::RequestContext, request::Response};
use crate::utils::{self, Data, FlagError};
use crate::{GrpcStatus, Message, consts, service::MethodDef};

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
                                return Err(Error::from(ClientError::GrpcStatus(
                                    status, headers, None,
                                )));
                            }
                            Some(Err(msg)) => {
                                return Err(synthesized_status(
                                    GrpcStatus::Unknown,
                                    headers,
                                    msg,
                                    Bytes::new(),
                                ));
                            }
                            Some(Ok(_)) | None => {}
                        }
                        if let Some(st) = pseudo.status.filter(|st| *st != StatusCode::OK) {
                            return Err(http_status_error(st, headers, Bytes::new()));
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
                            missing = Some((
                                GrpcStatus::Internal,
                                HeaderValue::from_static(NO_TRAILERS),
                            ));
                        }
                        h2::StreamEof::Trailers(hdrs) => {
                            // check grpc status
                            match check_grpc_status(&hdrs) {
                                Some(Ok(GrpcStatus::Ok)) => {}
                                None => {
                                    missing = Some((
                                        GrpcStatus::Unknown,
                                        HeaderValue::from_static(NO_GRPC_STATUS),
                                    ));
                                }
                                Some(Ok(GrpcStatus::DeadlineExceeded)) => {
                                    return Err(Error::from(ClientError::DeadlineExceeded(hdrs)));
                                }
                                Some(Ok(st)) => {
                                    return Err(Error::from(ClientError::GrpcStatus(
                                        st, hdrs, None,
                                    )));
                                }
                                Some(Err(msg)) => {
                                    return Err(synthesized_status(
                                        GrpcStatus::Unknown,
                                        hdrs,
                                        msg,
                                        payload.get(),
                                    ));
                                }
                            }
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
                    if st != StatusCode::OK {
                        return Err(http_status_error(st, hdrs, data));
                    }
                }
                None => return Err(Error::from(ClientError::Response(None, hdrs, data))),
            }
            if let Some((status, msg)) = check_content_type(&hdrs).or(missing) {
                return Err(synthesized_status(status, trailers, msg, data));
            }
            let resp_size = data.len();
            if resp_size < 5 {
                return Err(Error::from(ClientError::UnexpectedEof(status, hdrs)));
            }
            // we only accept identity, a compliant server never compresses
            if let Err(FlagError::Unsupported(msg) | FlagError::Invalid(msg)) =
                utils::check_compressed_flag(data[0], &hdrs)
            {
                return Err(synthesized_status(
                    GrpcStatus::Internal,
                    trailers,
                    msg,
                    data,
                ));
            }
            data.advance(1);
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

/// Maps an HTTP status other than 200 of a response without `grpc-status`.
///
/// The response headers are reported in place of the trailers.
fn http_status_error(st: StatusCode, hdrs: HeaderMap, body: Bytes) -> Error<ClientError> {
    let status = match st.as_u16() {
        400 => GrpcStatus::Internal,
        401 => GrpcStatus::Unauthenticated,
        403 => GrpcStatus::PermissionDenied,
        404 => GrpcStatus::Unimplemented,
        429 | 502 | 503 | 504 => GrpcStatus::Unavailable,
        _ => GrpcStatus::Unknown,
    };
    let msg = HeaderValue::try_from(format!("HTTP status {}", st.as_u16()))
        .unwrap_or_else(|_| HeaderValue::from_static("HTTP error"));
    synthesized_status(status, hdrs, msg, body)
}

/// Reports a status the client picked, `msg` is added as `grpc-message`
/// unless the server sent one.
fn synthesized_status(
    status: GrpcStatus,
    mut hdrs: HeaderMap,
    msg: HeaderValue,
    body: Bytes,
) -> Error<ClientError> {
    if !hdrs.contains_key(consts::GRPC_MESSAGE) {
        hdrs.insert(consts::GRPC_MESSAGE, msg);
    }
    Error::from(ClientError::GrpcStatus(status, hdrs, Some(body)))
}

/// Returns the status to report if the response is not a grpc response.
///
/// Accepts `application/grpc`, optionally followed by `+format` or `;params`.
fn check_content_type(hdrs: &HeaderMap) -> Option<(GrpcStatus, HeaderValue)> {
    let Some(val) = hdrs.get(header::CONTENT_TYPE) else {
        return Some((
            GrpcStatus::Unknown,
            HeaderValue::from_static("Response has no content-type"),
        ));
    };
    let ct = val.as_bytes();
    let prefix = b"application/grpc";
    if ct.len() >= prefix.len()
        && ct[..prefix.len()].eq_ignore_ascii_case(prefix)
        && matches!(ct.get(prefix.len()), None | Some(b'+' | b';'))
    {
        return None;
    }

    Some((
        GrpcStatus::Unknown,
        utils::grpc_message("Invalid content-type", val),
    ))
}

/// Reads `grpc-status`, an unknown or invalid code is an error with the
/// `grpc-message` to report it with `UNKNOWN`.
fn check_grpc_status(hdrs: &HeaderMap) -> Option<Result<GrpcStatus, HeaderValue>> {
    let val = hdrs.get(consts::GRPC_STATUS)?;
    Some(
        val.to_str()
            .ok()
            .and_then(|v| u8::from_str(v).ok())
            .and_then(|v| GrpcStatus::try_from(v).ok())
            .ok_or_else(|| utils::grpc_message("Unknown grpc-status", val)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_status_mapping() {
        for (code, status) in [
            (400, GrpcStatus::Internal),
            (401, GrpcStatus::Unauthenticated),
            (403, GrpcStatus::PermissionDenied),
            (404, GrpcStatus::Unimplemented),
            (429, GrpcStatus::Unavailable),
            (502, GrpcStatus::Unavailable),
            (503, GrpcStatus::Unavailable),
            (504, GrpcStatus::Unavailable),
            (500, GrpcStatus::Unknown),
            (302, GrpcStatus::Unknown),
            (204, GrpcStatus::Unknown),
        ] {
            let st = StatusCode::from_u16(code).unwrap();
            let err = http_status_error(st, HeaderMap::new(), Bytes::new());
            let ClientError::GrpcStatus(s, hdrs, _) = &*err else {
                panic!("{err:?}");
            };
            assert_eq!(*s, status, "{code}");
            assert_eq!(
                hdrs.get(consts::GRPC_MESSAGE).unwrap(),
                &format!("HTTP status {code}")
            );
        }
    }
}
