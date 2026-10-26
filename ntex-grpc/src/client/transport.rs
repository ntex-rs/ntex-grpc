use std::time::{Duration, Instant};
use std::{convert::TryFrom, future::Future, str::FromStr};

use ntex_bytes::{Buf, BufMut, BytePages, ByteString, Bytes};
use ntex_error::Error;
use ntex_h2::{self as h2, ConnectionError, OperationError, StreamError, frame::Reason};
use ntex_http::{HeaderMap, HeaderValue, Method, StatusCode, error::Error as HttpError, header};
use ntex_util::time;

use super::{CallFailure, Cause, Client, ClientError, NotGrpc, Transport};
use super::{Status, request::RequestContext, request::Response};
#[cfg(feature = "compression")]
use crate::Compression;
use crate::utils::{self, Data};
use crate::{GrpcStatus, Message, consts, service::MethodDef};

impl<St, T: MethodDef> Transport<T, St> for Client {
    type Error = Error<ClientError>;

    #[inline]
    async fn request(
        &self,
        val: &T::Input,
        ctx: &mut RequestContext<'_, St>,
    ) -> Result<Response<T>, Self::Error> {
        Transport::request(&self.0, val, ctx).await
    }
}

impl<T: MethodDef, St> Transport<T, St> for h2::client::Client {
    type Error = Error<ClientError>;

    /// The timeout covers waiting for a connection too.
    async fn request(
        &self,
        val: &T::Input,
        ctx: &mut RequestContext<'_, St>,
    ) -> Result<Response<T>, Self::Error> {
        with_deadline(ctx.get_timeout(), async |deadline| {
            let client = self.client().await.map_err(|e| e.map(connect_error))?;
            send_request(&client, val, ctx, deadline).await
        })
        .await
    }
}

impl<T: MethodDef, St> Transport<T, St> for h2::client::SimpleClient {
    type Error = Error<ClientError>;

    async fn request(
        &self,
        val: &T::Input,
        ctx: &mut RequestContext<'_, St>,
    ) -> Result<Response<T>, Self::Error> {
        with_deadline(ctx.get_timeout(), async |deadline| {
            send_request(self, val, ctx, deadline).await
        })
        .await
        .map_err(|e| e.with_service(self.service()))
    }
}

/// Stop waiting once the request timeout runs out.
///
/// The pending stream is dropped, which resets it with `CANCEL`. A zero
/// timeout has already run out, the request is not sent then.
///
/// `f` gets the deadline, the time the timeout runs out.
pub(crate) async fn with_deadline<R, F>(
    timeout: Option<Duration>,
    f: impl FnOnce(Option<Instant>) -> F,
) -> Result<R, Error<ClientError>>
where
    F: Future<Output = Result<R, Error<ClientError>>>,
{
    if let Some(timeout) = timeout {
        if timeout.is_zero() {
            return Err(deadline_exceeded(TIMEOUT_ZERO, HeaderMap::new()));
        }
        let deadline = Instant::now().checked_add(timeout);
        time::timeout(timeout, f(deadline))
            .await
            .unwrap_or_else(|()| Err(deadline_exceeded(TIMEOUT_RAN_OUT, HeaderMap::new())))
    } else {
        f(None).await
    }
}

/// The request timeout ran out, `hdrs` are the response headers received
/// so far.
pub(crate) fn deadline_exceeded(msg: &'static str, hdrs: HeaderMap) -> Error<ClientError> {
    Error::from(ClientError::DeadlineExceeded(
        CallFailure::new(GrpcStatus::DeadlineExceeded, msg).with_headers(hdrs),
    ))
}

/// Maps a failure to get a connection, the request was not sent.
pub(crate) fn connect_error(err: h2::client::ClientError) -> ClientError {
    ClientError::Connection {
        request_sent: false,
        failure: CallFailure::new(GrpcStatus::Unavailable, error_text(&err)).with_cause(err),
    }
}

/// Maps an invalid header set for the call, see
/// [`RequestContext::take_error()`].
pub(crate) fn request_error(err: HttpError) -> ClientError {
    let failure = CallFailure::new(GrpcStatus::Internal, err.to_string()).with_cause(err);
    ClientError::InvalidRequest(failure)
}

/// The headers of a request: the grpc ones and the ones set for the call.
pub(crate) fn request_headers<St>(ctx: &RequestContext<'_, St>) -> HeaderMap {
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
    hdrs
}

#[allow(clippy::too_many_lines)]
async fn send_request<T: MethodDef, St>(
    client: &h2::client::SimpleClient,
    val: &T::Input,
    ctx: &mut RequestContext<'_, St>,
    deadline: Option<Instant>,
) -> Result<Response<T>, Error<ClientError>> {
    if let Some(err) = ctx.take_error() {
        return Err(Error::from(request_error(err)).with_service(client.service()));
    }

    let buf = encode_message(val, ctx)
        .await
        .map_err(|e| Error::from(e).with_service(client.service()))?;
    let req_size = buf.len();

    let hdrs = request_headers(ctx);

    // send request
    let (snd_stream, rcv_stream) = client
        .send(Method::POST, T::PATH, hdrs, false)
        .await
        .map_err(|e| operation_error(e, false, HeaderMap::new(), deadline))?;
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
            return Err(operation_error(err, true, HeaderMap::new(), deadline));
        }
    }

    // read response
    let mut status = None;
    let mut hdrs = HeaderMap::default();
    let mut trailers = None;
    let mut payload = Data::Empty;
    // the failure to report if the response has no `grpc-status`
    let mut missing = None;
    let max_size = ctx.get_max_message_size();

    async {
        loop {
            let Some(msg) = rcv_stream.recv().await else {
                let failure =
                    CallFailure::new(GrpcStatus::Unavailable, STREAM_CLOSED).with_headers(hdrs);
                return Err(Error::from(ClientError::Connection {
                    request_sent: true,
                    failure,
                }));
            };

            match msg.kind {
                h2::MessageKind::Headers {
                    headers,
                    pseudo,
                    eof,
                } => {
                    if eof {
                        return Err(Error::from(trailers_only(pseudo.status, headers)));
                    }
                    hdrs = headers;
                    status = pseudo.status;
                    continue;
                }
                h2::MessageKind::Data(data, _cap) => {
                    payload.push(data, message_limit(status, &hdrs, max_size));
                    if let Some((st, msg)) = check_message(status, &hdrs, &payload, max_size) {
                        return Err(invalid_response(st, msg, hdrs));
                    }
                    if !limit_other_body(status, &hdrs, &mut payload) {
                        continue;
                    }
                    // not a grpc response, the rest of the body is not read
                }
                h2::MessageKind::Eof(data) => match data {
                    h2::StreamEof::Data(data, _cap) => {
                        payload.push(data, message_limit(status, &hdrs, max_size));
                        if let Some((st, msg)) = check_message(status, &hdrs, &payload, max_size) {
                            return Err(invalid_response(st, msg, hdrs));
                        }
                        limit_other_body(status, &hdrs, &mut payload);
                        missing = Some((GrpcStatus::Internal, NO_TRAILERS));
                    }
                    h2::StreamEof::Trailers(hdrs) => {
                        match check_grpc_status(&hdrs) {
                            Some(Ok(GrpcStatus::Ok)) => {}
                            None => missing = Some((GrpcStatus::Unknown, NO_GRPC_STATUS)),
                            Some(Ok(st)) => {
                                return Err(Error::from(ClientError::Status(Status::new(
                                    st, hdrs,
                                ))));
                            }
                            Some(Err(msg)) => {
                                return Err(invalid_response(GrpcStatus::Unknown, msg, hdrs));
                            }
                        }
                        trailers = Some(hdrs);
                    }
                    h2::StreamEof::Error(err) => {
                        return Err(err.map(|e| stream_error(e, hdrs, deadline)));
                    }
                },
                h2::MessageKind::Disconnect(err) => {
                    return Err(operation_error(err, true, hdrs, deadline));
                }
            }

            let mut data = payload.get();
            if !is_grpc(status, &hdrs) {
                return Err(Error::from(ClientError::NotGrpc(NotGrpc::new(
                    status, hdrs, data,
                ))));
            }
            // the trailers, or the response headers if the response has none
            let trailers = trailers.unwrap_or_else(|| std::mem::take(&mut hdrs));
            if let Some((st, msg)) = missing {
                return Err(invalid_response(st, msg, trailers));
            }
            let resp_size = data.len();
            if resp_size < 5 {
                return Err(invalid_response(GrpcStatus::Internal, NO_MESSAGE, trailers));
            }
            let flag = data.get_u8();
            let len = data.get_u32();
            let Some(block) = data.split_to_checked(len as usize) else {
                return Err(invalid_response(GrpcStatus::Internal, TRUNCATED, trailers));
            };
            // a message in an unknown encoding is an internal error
            let mut block = match utils::read_message(
                flag,
                hdrs.get(consts::GRPC_ENCODING).cloned(),
                block,
                max_size,
                GrpcStatus::Internal,
            )
            .await
            {
                Ok(block) => block,
                Err((st, msg)) => return Err(invalid_response(st, text(&msg), trailers)),
            };

            return match <T::Output as Message>::read(&mut block) {
                Ok(output) => Ok(Response {
                    output,
                    trailers,
                    req_size,
                    headers: hdrs,
                    res_size: resp_size,
                }),
                Err(e) => {
                    let failure = CallFailure::new(GrpcStatus::Internal, e.to_string())
                        .with_headers(trailers)
                        .with_cause(e);
                    Err(Error::from(ClientError::InvalidResponse(failure)))
                }
            };
        }
    }
    .await
    .map_err(|e| e.with_service(client.service()))
}

/// The most of a response body that is not a grpc one, kept for the error.
const MAX_OTHER_BODY: usize = 64 * 1024;
const TIMEOUT_ZERO: &str = "Request timeout is zero";
pub(crate) const TIMEOUT_RAN_OUT: &str = "Request timeout ran out";
pub(crate) const NO_TRAILERS: &str = "Response ended without trailers";
pub(crate) const NO_GRPC_STATUS: &str = "Response trailers have no grpc-status";
pub(crate) const NO_MESSAGE: &str = "Response has no message";
pub(crate) const TRUNCATED: &str = "Response message is truncated";
pub(crate) const STREAM_CLOSED: &str = "Stream closed before the response ended";
pub(crate) const EXTRA_DATA: &str = "cardinality violation: expected <EOF> for non server-streaming RPCs, but received another message";

pub(crate) fn invalid_response(
    status: GrpcStatus,
    msg: impl Into<ByteString>,
    hdrs: HeaderMap,
) -> Error<ClientError> {
    Error::from(ClientError::InvalidResponse(
        CallFailure::new(status, msg).with_headers(hdrs),
    ))
}

/// Maps a response that ends with its headers.
///
/// A status the server sent wins, then a response that is not a grpc one.
pub(crate) fn trailers_only(status: Option<StatusCode>, hdrs: HeaderMap) -> ClientError {
    let (code, msg) = match check_grpc_status(&hdrs) {
        Some(Ok(GrpcStatus::Ok)) if is_grpc(status, &hdrs) => {
            (GrpcStatus::Internal, NO_MESSAGE.into())
        }
        None if is_grpc(status, &hdrs) => (GrpcStatus::Unknown, NO_GRPC_STATUS.into()),
        Some(Ok(GrpcStatus::Ok)) | None => {
            return ClientError::NotGrpc(NotGrpc::new(status, hdrs, Bytes::new()));
        }
        Some(Ok(st)) => return ClientError::Status(Status::new(st, hdrs)),
        Some(Err(msg)) => (GrpcStatus::Unknown, msg),
    };
    ClientError::InvalidResponse(CallFailure::new(code, msg).with_headers(hdrs))
}

/// Maps a stream reset by the server.
///
/// `REFUSED_STREAM` means the server did not process the request. A
/// `CANCEL` once the deadline has passed means the server stopped the call
/// because of the timeout.
pub(crate) fn reset_error(
    reason: Reason,
    hdrs: HeaderMap,
    deadline: Option<Instant>,
    cause: impl Into<Cause>,
) -> ClientError {
    let msg = format!("Stream reset with {reason:?}");
    if reason == Reason::REFUSED_STREAM {
        let failure = CallFailure::new(GrpcStatus::Unavailable, msg);
        return ClientError::NotProcessed(failure.with_headers(hdrs).with_cause(cause));
    }
    if reason == Reason::CANCEL && deadline.is_some_and(|d| Instant::now() >= d) {
        let failure = CallFailure::new(
            GrpcStatus::DeadlineExceeded,
            "Stream reset with CANCEL after the deadline",
        );
        return ClientError::DeadlineExceeded(failure.with_headers(hdrs).with_cause(cause));
    }
    let failure = CallFailure::new(GrpcStatus::from(reason), msg);
    ClientError::StreamFailed(failure.with_headers(hdrs).with_cause(cause))
}

/// Maps a failed stream, the connection is still up.
pub(crate) fn stream_error(
    err: StreamError,
    hdrs: HeaderMap,
    deadline: Option<Instant>,
) -> ClientError {
    if let StreamError::Reset(reason) = err {
        return reset_error(reason, hdrs, deadline, err);
    }
    let code = if matches!(err, StreamError::CapacityTimeout) {
        GrpcStatus::Unavailable
    } else {
        GrpcStatus::Internal
    };
    let failure = CallFailure::new(code, err.to_string());
    ClientError::StreamFailed(failure.with_headers(hdrs).with_cause(err))
}

/// Maps an error of the connection or the stream.
///
/// `sent` is whether the request was handed to the connection. A GOAWAY
/// fails only the streams the server did not process.
pub(crate) fn operation_error(
    err: Error<OperationError>,
    sent: bool,
    hdrs: HeaderMap,
    deadline: Option<Instant>,
) -> Error<ClientError> {
    err.map(|err| {
        let failure = |code| {
            CallFailure::new(code, err.to_string())
                .with_headers(hdrs.clone())
                .with_cause(err)
        };
        match err {
            OperationError::Connection(ConnectionError::GoAway(_)) => {
                ClientError::NotProcessed(failure(GrpcStatus::Unavailable))
            }
            OperationError::Disconnecting | OperationError::OverflowedStreamId if !sent => {
                ClientError::NotProcessed(failure(GrpcStatus::Unavailable))
            }
            OperationError::Connection(_)
            | OperationError::Disconnected
            | OperationError::Disconnecting
            | OperationError::OverflowedStreamId => ClientError::Connection {
                request_sent: sent,
                failure: failure(GrpcStatus::Unavailable),
            },
            OperationError::HeaderListTooLarge { .. } => {
                ClientError::InvalidRequest(failure(GrpcStatus::Internal))
            }
            OperationError::RemoteReset(reason) => reset_error(reason, hdrs, deadline, err),
            OperationError::Stream(e) => stream_error(e, hdrs, deadline),
            OperationError::Idle
            | OperationError::Payload
            | OperationError::Closed(_)
            | OperationError::LocalReset(_) => {
                ClientError::StreamFailed(failure(GrpcStatus::Internal))
            }
        }
    })
}

/// The error and its sources, e.g. "Connect error: Connection refused".
fn error_text(err: &dyn std::error::Error) -> String {
    let mut msg = err.to_string();
    let mut source = err.source();
    while let Some(err) = source {
        msg.push_str(": ");
        msg.push_str(&err.to_string());
        source = err.source();
    }
    msg
}

/// The text of a percent-encoded `grpc-message`.
pub(crate) fn text(val: &HeaderValue) -> ByteString {
    ByteString::from(utils::percent_decode(val).into_owned())
}

/// Whether the response is a grpc one, its body is a grpc message then.
pub(crate) fn is_grpc(status: Option<StatusCode>, hdrs: &HeaderMap) -> bool {
    status == Some(StatusCode::OK)
        && hdrs
            .get(header::CONTENT_TYPE)
            .is_some_and(|val| utils::is_grpc_content_type(val.as_bytes()))
}

/// The largest message to buffer, the body of a response that is not a grpc
/// one has no length prefix.
fn message_limit(status: Option<StatusCode>, hdrs: &HeaderMap, max_size: usize) -> usize {
    if is_grpc(status, hdrs) { max_size } else { 0 }
}

/// Keeps at most [`MAX_OTHER_BODY`] bytes of a response body that is not a
/// grpc one, e.g. an error page from a proxy. Returns true once the limit is
/// reached, the rest of the body is dropped.
pub(crate) fn limit_other_body(
    status: Option<StatusCode>,
    hdrs: &HeaderMap,
    payload: &mut Data,
) -> bool {
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
) -> Option<(GrpcStatus, ByteString)> {
    if !is_grpc(status, hdrs) {
        return None;
    }
    let data = payload.as_slice();
    let len = u32::from_be_bytes(data.get(1..5)?.try_into().ok()?) as usize;
    if len > max_size {
        let msg = format!("Received message larger than max ({len} vs. {max_size})");
        Some((GrpcStatus::ResourceExhausted, msg.into()))
    } else if data.len() - 5 > len {
        Some((GrpcStatus::Internal, EXTRA_DATA.into()))
    } else {
        None
    }
}

/// Encodes the request message with its length prefix.
///
/// A message is sent uncompressed if it is small or does not get smaller.
#[cfg_attr(not(feature = "compression"), allow(clippy::unused_async))]
pub(crate) async fn encode_message<M: Message, St>(
    val: &M,
    ctx: &RequestContext<'_, St>,
) -> Result<BytePages, ClientError> {
    let max_size = ctx.get_max_send_message_size();

    #[cfg(feature = "compression")]
    if let Some(enc) = ctx.get_compression() {
        let len = val.encoded_len();
        if len >= crate::compression::MIN_SIZE {
            // the uncompressed message must fit the length prefix too
            send_size(len, usize::MAX)?;
            let mut msg = BytePages::default();
            val.write(&mut msg);
            let compressed = enc.compress(&mut msg).await.map_err(|(status, msg)| {
                ClientError::InvalidRequest(CallFailure::new(status, text(&msg)))
            })?;
            let len = send_size(msg.len(), max_size)?;
            utils::prepend_prefix(&mut msg, compressed, len);
            return Ok(msg);
        }
    }

    let len = send_size(val.encoded_len(), max_size)?;
    let mut buf = BytePages::default();
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
    Err(ClientError::InvalidRequest(CallFailure::new(
        GrpcStatus::ResourceExhausted,
        msg,
    )))
}

/// Reads `grpc-status`, an unknown or invalid code is an error with the
/// message to report it with `UNKNOWN`.
pub(crate) fn check_grpc_status(hdrs: &HeaderMap) -> Option<Result<GrpcStatus, ByteString>> {
    let val = hdrs.get(consts::GRPC_STATUS)?;
    Some(
        val.to_str()
            .ok()
            .and_then(|v| u8::from_str(v).ok())
            .and_then(|v| GrpcStatus::try_from(v).ok())
            .ok_or_else(|| {
                format!(
                    "Unknown grpc-status: {}",
                    String::from_utf8_lossy(val.as_bytes())
                )
                .into()
            }),
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    fn headers() -> HeaderMap {
        let mut hdrs = HeaderMap::new();
        hdrs.insert(header::CONTENT_TYPE, HeaderValue::from_static("x"));
        hdrs
    }

    fn failure(err: &ClientError) -> &CallFailure {
        match err {
            ClientError::InvalidRequest(f)
            | ClientError::NotProcessed(f)
            | ClientError::Connection { failure: f, .. }
            | ClientError::StreamFailed(f)
            | ClientError::InvalidResponse(f)
            | ClientError::DeadlineExceeded(f) => f,
            err => panic!("{err:?}"),
        }
    }

    #[test]
    fn operation_errors() {
        let map = |err, sent| {
            let err = operation_error(Error::from(err), sent, headers(), None);
            let f = failure(&err);
            assert_eq!(f.headers.get(header::CONTENT_TYPE).unwrap(), "x");
            assert!(f.source().is_some());
            let msg = f.message.clone();
            (err.into_error(), msg)
        };
        let goaway = OperationError::Connection(ConnectionError::GoAway(Reason::NO_ERROR));
        let keepalive = OperationError::Connection(ConnectionError::KeepaliveTimeout);

        for sent in [false, true] {
            let (err, _) = map(goaway, sent);
            assert!(matches!(err, ClientError::NotProcessed(_)), "{err:?}");
            assert_eq!(err.status(), GrpcStatus::Unavailable);

            let (err, msg) = map(keepalive, sent);
            assert!(
                matches!(err, ClientError::Connection { request_sent, .. } if request_sent == sent),
                "{err:?}"
            );
            assert_eq!(err.status(), GrpcStatus::Unavailable);
            assert_eq!(msg, "Keep-alive timeout");

            let (err, msg) = map(OperationError::Disconnected, sent);
            assert!(
                matches!(err, ClientError::Connection { request_sent, .. } if request_sent == sent),
                "{err:?}"
            );
            assert_eq!(msg, "Connection is closed");
        }

        // not sent, the server did not see the request
        for err in [
            OperationError::Disconnecting,
            OperationError::OverflowedStreamId,
        ] {
            let (err, _) = map(err, false);
            assert!(matches!(err, ClientError::NotProcessed(_)), "{err:?}");
        }
        let (err, msg) = map(OperationError::Disconnecting, true);
        assert!(
            matches!(
                err,
                ClientError::Connection {
                    request_sent: true,
                    ..
                }
            ),
            "{err:?}"
        );
        assert_eq!(msg, "Connection is disconnecting");

        let (err, _) = map(
            OperationError::HeaderListTooLarge { size: 2, max: 1 },
            false,
        );
        assert!(matches!(err, ClientError::InvalidRequest(_)), "{err:?}");
        assert_eq!(err.status(), GrpcStatus::Internal);

        let (err, _) = map(OperationError::RemoteReset(Reason::REFUSED_STREAM), true);
        assert!(matches!(err, ClientError::NotProcessed(_)), "{err:?}");
        let (err, _) = map(OperationError::RemoteReset(Reason::CANCEL), true);
        assert!(matches!(err, ClientError::StreamFailed(_)), "{err:?}");
        assert_eq!(err.status(), GrpcStatus::Cancelled);

        let (err, _) = map(OperationError::Stream(StreamError::CapacityTimeout), true);
        assert!(matches!(err, ClientError::StreamFailed(_)), "{err:?}");
        assert_eq!(err.status(), GrpcStatus::Unavailable);

        for err in [
            OperationError::Idle,
            OperationError::Payload,
            OperationError::Closed(None),
        ] {
            let (err, _) = map(err, true);
            assert!(matches!(err, ClientError::StreamFailed(_)), "{err:?}");
            assert_eq!(err.status(), GrpcStatus::Internal);
        }
    }

    #[test]
    fn reset_mapping() {
        let past = Instant::now().checked_sub(Duration::from_millis(1));
        let future = Instant::now().checked_add(Duration::from_secs(60));
        let map = |reason, deadline| {
            let err = reset_error(reason, headers(), deadline, StreamError::Reset(reason));
            let f = failure(&err);
            assert_eq!(f.headers.get(header::CONTENT_TYPE).unwrap(), "x");
            assert!(f.source().is_some());
            err
        };

        let err = map(Reason::CANCEL, past);
        assert!(matches!(err, ClientError::DeadlineExceeded(_)), "{err:?}");
        assert_eq!(err.status(), GrpcStatus::DeadlineExceeded);
        assert_eq!(err.message(), "Stream reset with CANCEL after the deadline");

        for deadline in [future, None] {
            let err = map(Reason::CANCEL, deadline);
            assert!(matches!(err, ClientError::StreamFailed(_)), "{err:?}");
            assert_eq!(err.status(), GrpcStatus::Cancelled);
            assert_eq!(err.message(), "Stream reset with CANCEL");
        }

        let err = map(Reason::REFUSED_STREAM, past);
        assert!(matches!(err, ClientError::NotProcessed(_)), "{err:?}");
        assert_eq!(err.status(), GrpcStatus::Unavailable);

        let err = map(Reason::ENHANCE_YOUR_CALM, None);
        assert!(matches!(err, ClientError::StreamFailed(_)), "{err:?}");
        assert_eq!(err.status(), GrpcStatus::ResourceExhausted);
    }

    #[test]
    fn stream_errors() {
        let err = stream_error(StreamError::WindowOverflowed, headers(), None);
        assert!(matches!(err, ClientError::StreamFailed(_)), "{err:?}");
        assert_eq!(err.status(), GrpcStatus::Internal);
        assert_eq!(err.message(), StreamError::WindowOverflowed.to_string());
        assert!(err.source().is_some());
    }

    #[test]
    fn send_size_limit() {
        let msg = |res: Result<u32, ClientError>| match res.unwrap_err() {
            ClientError::InvalidRequest(f) => {
                assert_eq!(f.code, GrpcStatus::ResourceExhausted);
                assert!(f.headers.is_empty());
                f.message
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
    fn trailers_only_mapping() {
        let map =
            |status: Option<u16>, ct: Option<&'static str>, grpc_status: Option<&'static str>| {
                let mut hdrs = HeaderMap::new();
                if let Some(ct) = ct {
                    hdrs.insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
                }
                if let Some(st) = grpc_status {
                    hdrs.insert(consts::GRPC_STATUS, HeaderValue::from_static(st));
                }
                let status = status.map(|st| StatusCode::from_u16(st).unwrap());
                trailers_only(status, hdrs)
            };
        let grpc = Some("application/grpc");

        // a status from the server wins
        let err = map(Some(503), None, Some("5"));
        assert!(
            matches!(&err, ClientError::Status(st) if st.code == GrpcStatus::NotFound),
            "{err:?}"
        );
        let err = map(Some(200), grpc, Some("4"));
        assert!(matches!(&err, ClientError::Status(_)), "{err:?}");
        assert_eq!(err.status(), GrpcStatus::DeadlineExceeded);

        let err = map(Some(503), None, Some("x"));
        assert!(matches!(err, ClientError::InvalidResponse(_)), "{err:?}");
        assert_eq!(err.status(), GrpcStatus::Unknown);
        assert_eq!(err.message(), "Unknown grpc-status: x");

        let err = map(Some(503), grpc, None);
        assert!(matches!(err, ClientError::NotGrpc(_)), "{err:?}");
        assert_eq!(err.status(), GrpcStatus::Unavailable);
        let err = map(None, grpc, Some("0"));
        assert!(
            matches!(&err, ClientError::NotGrpc(r) if r.http_status.is_none()),
            "{err:?}"
        );
        let err = map(Some(200), Some("text/html"), None);
        assert!(matches!(err, ClientError::NotGrpc(_)), "{err:?}");
        assert_eq!(err.status(), GrpcStatus::Unknown);
        assert_eq!(err.message(), "Invalid content-type: text/html");

        // a grpc response without status or message
        let err = map(Some(200), grpc, None);
        assert!(matches!(err, ClientError::InvalidResponse(_)), "{err:?}");
        assert_eq!(err.status(), GrpcStatus::Unknown);
        assert_eq!(err.message(), NO_GRPC_STATUS);
        let err = map(Some(200), grpc, Some("0"));
        assert!(matches!(err, ClientError::InvalidResponse(_)), "{err:?}");
        assert_eq!(err.status(), GrpcStatus::Internal);
        assert_eq!(err.message(), NO_MESSAGE);
    }

    #[test]
    fn error_text_sources() {
        let err = h2::client::ClientError::Connect(ntex_net::connect::ConnectError::Io(
            std::io::Error::other("refused"),
        ));
        assert_eq!(error_text(&err), "Connect error: Tcp connect: refused");
    }
}
