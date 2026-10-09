use std::{convert::TryFrom, future::Future, str::FromStr, time::Duration};

use ntex_bytes::{Buf, BufMut, BytePages, Bytes};
use ntex_error::Error;
use ntex_h2::{self as h2, OperationError, frame::Reason};
use ntex_http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use ntex_util::time;

use super::{Client, ClientError, Transport, request::RequestContext, request::Response};
#[cfg(feature = "compression")]
use crate::Compression;
use crate::utils::{self, Data};
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
/// The pending stream is dropped, which resets it with `CANCEL`. A zero
/// timeout has already run out, the request is not sent then.
async fn with_deadline<R>(
    timeout: Option<Duration>,
    fut: impl Future<Output = Result<R, Error<ClientError>>>,
) -> Result<R, Error<ClientError>> {
    if let Some(timeout) = timeout {
        if timeout.is_zero() {
            return Err(Error::from(ClientError::DeadlineExceeded(HeaderMap::new())));
        }
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
    if let Some(err) = ctx.take_error() {
        return Err(Error::from(ClientError::Http(err)).with_service(client.service()));
    }

    let buf = encode_message(val, ctx)
        .await
        .map_err(|e| Error::from(e).with_service(client.service()))?;
    let req_size = buf.len();

    let mut hdrs = HeaderMap::new();
    hdrs.append(header::CONTENT_TYPE, consts::HDRV_CT_GRPC);
    hdrs.append(header::USER_AGENT, consts::HDRV_USER_AGENT);
    hdrs.insert(header::TE, consts::HDRV_TRAILERS);
    #[cfg(feature = "compression")]
    let enc = ctx
        .get_compression()
        .map_or(consts::IDENTITY, Compression::header);
    #[cfg(not(feature = "compression"))]
    let enc = consts::IDENTITY;
    hdrs.insert(consts::GRPC_ENCODING, enc);
    hdrs.insert(consts::GRPC_ACCEPT_ENCODING, consts::ACCEPT_ENCODING);
    // the context holds no reserved headers, see `RequestContext::header()`
    for (key, val) in ctx.headers() {
        hdrs.append(key.clone(), val.clone());
    }

    // send request
    let (snd_stream, rcv_stream) = client
        .send(Method::POST, T::PATH, hdrs, false)
        .await
        .map_err(|e| operation_error(e, HeaderMap::new(), Bytes::new()))?;
    if ctx.get_disconnect_on_drop() {
        snd_stream.disconnect_on_drop();
    }
    if let Err(err) = snd_stream.send_pages(buf, true).await {
        // the server can reply and reset the stream before it reads the
        // whole request, the reply or the reset is in the receive queue
        if !matches!(
            *err,
            OperationError::RemoteReset(_) | OperationError::Closed(Some(_))
        ) {
            return Err(operation_error(err, HeaderMap::new(), Bytes::new()));
        }
    }

    // read response
    let mut status = None;
    let mut hdrs = HeaderMap::default();
    let mut trailers = HeaderMap::default();
    let mut payload = Data::Empty;
    // the status to report if the response has no `grpc-status`
    let mut missing = None;
    let max_size = ctx.get_max_message_size();

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
                    payload.push(data, message_limit(status, &hdrs, max_size));
                    if let Some((st, msg)) = check_message(status, &hdrs, &payload, max_size) {
                        return Err(synthesized_status(st, hdrs, msg, payload.get()));
                    }
                    if !limit_other_body(status, &hdrs, &mut payload) {
                        continue;
                    }
                    // not a grpc response, the rest of the body is not read
                }
                h2::MessageKind::Eof(data) => {
                    match data {
                        h2::StreamEof::Data(data, _cap) => {
                            payload.push(data, message_limit(status, &hdrs, max_size));
                            if let Some((st, msg)) =
                                check_message(status, &hdrs, &payload, max_size)
                            {
                                return Err(synthesized_status(st, hdrs, msg, payload.get()));
                            }
                            limit_other_body(status, &hdrs, &mut payload);
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
                            if let h2::StreamError::Reset(reason) = *err {
                                return Err(reset_error(reason, hdrs, payload.get()));
                            }
                            return Err(err.map(ClientError::Stream));
                        }
                    }
                }
                h2::MessageKind::Disconnect(err) => {
                    return Err(operation_error(err, hdrs, payload.get()));
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
            let body = data.clone();
            let flag = data.get_u8();
            let len = data.get_u32();
            let Some(block) = data.split_to_checked(len as usize) else {
                return Err(Error::from(ClientError::UnexpectedEof(None, hdrs)));
            };
            // a message in an unknown encoding is an internal error
            let max_size = ctx.get_max_message_size();
            let mut block = match utils::read_message(
                flag,
                &hdrs,
                block,
                max_size,
                GrpcStatus::Internal,
            )
            .await
            {
                Ok(block) => block,
                Err((status, msg)) => {
                    return Err(synthesized_status(status, trailers, msg, body));
                }
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

/// The most of a response body that is not a grpc one, kept for the error.
const MAX_OTHER_BODY: usize = 64 * 1024;
const NO_TRAILERS: &str = "Response ended without trailers";
const NO_GRPC_STATUS: &str = "Response trailers have no grpc-status";
const EXTRA_DATA: &str = "cardinality violation: expected <EOF> for non server-streaming RPCs, but received another message";

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

/// Maps a stream reset by the server.
///
/// The response headers received so far are reported in place of the
/// trailers.
fn reset_error(reason: Reason, hdrs: HeaderMap, body: Bytes) -> Error<ClientError> {
    let msg = HeaderValue::try_from(format!("Stream reset with {reason:?}"))
        .unwrap_or_else(|_| HeaderValue::from_static("Stream reset"));
    synthesized_status(GrpcStatus::from(reason), hdrs, msg, body)
}

/// Maps a lost connection to `UNAVAILABLE`.
///
/// The connection closed, failed or is going away, the call may be retried
/// on a new one. Other errors are reported as [`ClientError::Operation`].
fn operation_error(
    err: Error<OperationError>,
    hdrs: HeaderMap,
    body: Bytes,
) -> Error<ClientError> {
    if matches!(
        *err,
        OperationError::Connection(_)
            | OperationError::Disconnected
            | OperationError::Disconnecting
            | OperationError::OverflowedStreamId
    ) {
        let msg = HeaderValue::try_from(err.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("Connection is closed"));
        err.map(|_| synthesized(GrpcStatus::Unavailable, hdrs, msg, body))
    } else {
        err.map(ClientError::Operation)
    }
}

/// Whether the response is a grpc one, its body is a grpc message then.
fn is_grpc(status: Option<StatusCode>, hdrs: &HeaderMap) -> bool {
    status == Some(StatusCode::OK)
        && hdrs
            .get(header::CONTENT_TYPE)
            .is_some_and(|val| is_grpc_content_type(val.as_bytes()))
}

/// The largest message to buffer, the body of a response that is not a grpc
/// one has no length prefix.
fn message_limit(status: Option<StatusCode>, hdrs: &HeaderMap, max_size: usize) -> usize {
    if is_grpc(status, hdrs) { max_size } else { 0 }
}

/// Keeps at most [`MAX_OTHER_BODY`] bytes of a response body that is not a
/// grpc one, e.g. an error page from a proxy. Returns true once the limit is
/// reached, the rest of the body is dropped.
fn limit_other_body(status: Option<StatusCode>, hdrs: &HeaderMap, payload: &mut Data) -> bool {
    if payload.as_slice().len() <= MAX_OTHER_BODY || is_grpc(status, hdrs) {
        return false;
    }
    payload.truncate(MAX_OTHER_BODY);
    true
}

/// Checks the response message as data arrives.
///
/// Fails on a message over the size limit, or on data after the message,
/// a unary response has exactly one. Neither is buffered then. Only a grpc
/// response is checked, other responses fail later anyway.
fn check_message(
    status: Option<StatusCode>,
    hdrs: &HeaderMap,
    payload: &Data,
    max_size: usize,
) -> Option<(GrpcStatus, HeaderValue)> {
    if !is_grpc(status, hdrs) {
        return None;
    }
    let data = payload.as_slice();
    let len = u32::from_be_bytes(data.get(1..5)?.try_into().ok()?) as usize;
    if len > max_size {
        let msg = format!("Received message larger than max ({len} vs. {max_size})");
        Some((
            GrpcStatus::ResourceExhausted,
            HeaderValue::try_from(msg)
                .unwrap_or_else(|_| HeaderValue::from_static("Received message larger than max")),
        ))
    } else if data.len() - 5 > len {
        Some((GrpcStatus::Internal, HeaderValue::from_static(EXTRA_DATA)))
    } else {
        None
    }
}

/// Reports a status the client picked, `msg` is added as `grpc-message`
/// unless the server sent one.
fn synthesized_status(
    status: GrpcStatus,
    hdrs: HeaderMap,
    msg: HeaderValue,
    body: Bytes,
) -> Error<ClientError> {
    Error::from(synthesized(status, hdrs, msg, body))
}

fn synthesized(
    status: GrpcStatus,
    mut hdrs: HeaderMap,
    msg: HeaderValue,
    body: Bytes,
) -> ClientError {
    if !hdrs.contains_key(consts::GRPC_MESSAGE) {
        hdrs.insert(consts::GRPC_MESSAGE, msg);
    }
    ClientError::GrpcStatus(status, hdrs, Some(body))
}

/// Encodes the request message with its length prefix.
///
/// A message is sent uncompressed if it is small or does not get smaller.
#[cfg_attr(not(feature = "compression"), allow(clippy::unused_async))]
async fn encode_message<M: Message>(
    val: &M,
    ctx: &RequestContext,
) -> Result<BytePages, ClientError> {
    let max_size = ctx.get_max_send_message_size();
    let mut buf = BytePages::default();

    #[cfg(feature = "compression")]
    if let Some(enc) = ctx.get_compression() {
        let len = val.encoded_len();
        if len >= crate::compression::MIN_SIZE {
            // the uncompressed message must fit the length prefix too
            send_size(len, usize::MAX)?;
            let mut msg = BytePages::default();
            val.write(&mut msg);
            let compressed = enc.compress(&mut msg).await.map_err(|(status, msg)| {
                let mut hdrs = HeaderMap::new();
                hdrs.insert(consts::GRPC_MESSAGE, msg);
                ClientError::GrpcStatus(status, hdrs, None)
            })?;
            let len = send_size(msg.len(), max_size)?;
            buf.put_u8(u8::from(compressed));
            buf.put_u32(len);
            msg.move_to(&mut buf);
            return Ok(buf);
        }
    }

    let len = send_size(val.encoded_len(), max_size)?;
    buf.put_u8(0);
    buf.put_u32(len);
    val.write(&mut buf);
    Ok(buf)
}

/// Returns the length prefix of a request message, or the error if the
/// message must not be sent.
fn send_size(len: usize, max_size: usize) -> Result<u32, ClientError> {
    let msg = if len > max_size {
        format!("trying to send message larger than max ({len} vs. {max_size})")
    } else if let Ok(len) = u32::try_from(len) {
        return Ok(len);
    } else {
        format!("grpc: message too large ({len} bytes)")
    };
    let msg = HeaderValue::try_from(msg)
        .unwrap_or_else(|_| HeaderValue::from_static("grpc: message too large"));
    let mut hdrs = HeaderMap::new();
    hdrs.insert(consts::GRPC_MESSAGE, msg);
    Err(ClientError::GrpcStatus(
        GrpcStatus::ResourceExhausted,
        hdrs,
        None,
    ))
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
    if is_grpc_content_type(val.as_bytes()) {
        return None;
    }
    Some((
        GrpcStatus::Unknown,
        utils::grpc_message("Invalid content-type", val),
    ))
}

/// `application/grpc`, optionally with a `+` or `;` suffix, in any case.
fn is_grpc_content_type(ct: &[u8]) -> bool {
    let prefix = b"application/grpc";
    ct.len() >= prefix.len()
        && ct[..prefix.len()].eq_ignore_ascii_case(prefix)
        && matches!(ct.get(prefix.len()), None | Some(b'+' | b';'))
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
    fn connection_errors() {
        let unavailable = |err: OperationError| {
            let mut hdrs = HeaderMap::new();
            hdrs.insert(header::CONTENT_TYPE, HeaderValue::from_static("x"));
            let err = operation_error(Error::from(err), hdrs, Bytes::from_static(b"1"));
            let ClientError::GrpcStatus(GrpcStatus::Unavailable, hdrs, Some(body)) = &*err else {
                panic!("{err:?}");
            };
            assert_eq!(hdrs.get(header::CONTENT_TYPE).unwrap(), "x");
            assert_eq!(body, &b"1"[..]);
            hdrs.get(consts::GRPC_MESSAGE).unwrap().clone()
        };
        assert_eq!(
            unavailable(OperationError::Connection(
                h2::ConnectionError::KeepaliveTimeout
            )),
            "Keep-alive timeout"
        );
        assert_eq!(
            unavailable(OperationError::Disconnected),
            "Connection is closed"
        );
        assert_eq!(
            unavailable(OperationError::Disconnecting),
            "Connection is disconnecting"
        );
        assert_eq!(
            unavailable(OperationError::OverflowedStreamId),
            "The stream ID space is overflowed"
        );

        let err = operation_error(
            Error::from(OperationError::Idle),
            HeaderMap::new(),
            Bytes::new(),
        );
        assert!(matches!(*err, ClientError::Operation(OperationError::Idle)));
    }

    #[test]
    fn send_size_limit() {
        let msg = |res: Result<u32, ClientError>| match res.unwrap_err() {
            ClientError::GrpcStatus(GrpcStatus::ResourceExhausted, hdrs, None) => {
                assert_eq!(hdrs.len(), 1);
                hdrs.get(consts::GRPC_MESSAGE).unwrap().clone()
            }
            err => panic!("{err:?}"),
        };
        assert_eq!(send_size(0, 0).unwrap(), 0);
        assert_eq!(send_size(3, 3).unwrap(), 3);
        assert_eq!(
            msg(send_size(4, 3)),
            "trying to send message larger than max (4 vs. 3)"
        );

        let max = u32::MAX as usize;
        assert_eq!(send_size(max, usize::MAX).unwrap(), u32::MAX);
        #[cfg(target_pointer_width = "64")]
        assert_eq!(
            msg(send_size(max + 1, usize::MAX)),
            "grpc: message too large (4294967296 bytes)"
        );
    }

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
