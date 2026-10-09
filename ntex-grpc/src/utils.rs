use std::{borrow::Cow, mem};

use base64::engine::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use ntex_bytes::{Bytes, BytesMut};
use ntex_http::{HeaderMap, HeaderValue};
use urly::quoting::{Component, unquote};

#[cfg(feature = "compression")]
use crate::Compression;
use crate::{GrpcStatus, consts};

pub(crate) enum Data {
    Chunk(Bytes),
    MutChunk(BytesMut),
    Empty,
}

impl Data {
    pub(crate) fn get(&mut self) -> Bytes {
        match mem::replace(self, Data::Empty) {
            Data::Chunk(data) => data,
            Data::MutChunk(data) => data.freeze(),
            Data::Empty => Bytes::new(),
        }
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        match self {
            Data::Chunk(data) => data,
            Data::MutChunk(data) => data,
            Data::Empty => &[],
        }
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        match self {
            Data::Chunk(data) => data.truncate(len),
            Data::MutChunk(data) => data.truncate(len),
            Data::Empty => {}
        }
    }

    /// Appends a chunk of a grpc message.
    ///
    /// A single chunk is kept as it is. Once the length prefix has arrived
    /// and the message is at most `limit` bytes, the buffer is sized for the
    /// whole message, otherwise it grows as data arrives.
    pub(crate) fn push(&mut self, data: Bytes, limit: usize) {
        if data.is_empty() {
            return;
        }
        let mut buf = match mem::replace(self, Data::Empty) {
            Data::Chunk(cur) => BytesMut::from(cur),
            Data::MutChunk(buf) => buf,
            Data::Empty => {
                *self = Data::Chunk(data);
                return;
            }
        };
        if let Some(size) = message_size(&buf, &data, limit) {
            buf.reserve_exact(size.saturating_sub(buf.len()));
        }
        buf.extend_from_slice(&data);
        *self = Data::MutChunk(buf);
    }
}

/// The size of a message with its length prefix, if the prefix has arrived
/// and the message is at most `limit` bytes.
fn message_size(cur: &[u8], data: &[u8], limit: usize) -> Option<usize> {
    let mut prefix = cur.iter().chain(data).copied();
    let (_, a, b, c, d) = (
        prefix.next()?,
        prefix.next()?,
        prefix.next()?,
        prefix.next()?,
        prefix.next()?,
    );
    let len = u32::from_be_bytes([a, b, c, d]) as usize;
    (len <= limit).then_some(5 + len)
}

/// Encode a binary metadata value.
///
/// gRPC sends values of headers whose name ends with `-bin` as base64,
/// without padding.
///
/// ```
/// let val = ntex_grpc::encode_binary_header(&[0xff, 0x00]);
/// assert_eq!(val, "/wA");
/// ```
pub fn encode_binary_header(value: &[u8]) -> HeaderValue {
    let encoded = STANDARD_NO_PAD.encode(value);
    // SAFETY: base64 output is visible ASCII, which is a valid header value
    unsafe { HeaderValue::from_shared_unchecked(encoded.into()) }
}

/// Decode a binary metadata value, the value of a header whose name ends
/// with `-bin`.
///
/// Accepts base64 with or without padding. Returns `None` if the value is
/// not valid base64.
///
/// ```
/// use ntex_grpc::{HeaderValue, decode_binary_header};
///
/// let val = HeaderValue::from_static("/wA=");
/// assert_eq!(decode_binary_header(&val).unwrap(), [0xff, 0x00]);
/// ```
pub fn decode_binary_header(value: &HeaderValue) -> Option<Vec<u8>> {
    let value = value.as_bytes();
    // padded base64 is a multiple of 4 bytes long
    if value.len().is_multiple_of(4) {
        STANDARD.decode(value).ok()
    } else {
        STANDARD_NO_PAD.decode(value).ok()
    }
}

/// Why a message's compressed flag cannot be handled.
enum FlagError {
    /// The message is compressed with an encoding we do not support.
    Unsupported(HeaderValue),
    /// The flag is invalid, or set without a `grpc-encoding`.
    Invalid(HeaderValue),
}

/// Checks the compressed flag of a message, only identity is supported.
fn check_compressed_flag(flag: u8, hdrs: &HeaderMap) -> Result<(), FlagError> {
    match flag {
        0 => Ok(()),
        1 => match hdrs.get(consts::GRPC_ENCODING) {
            Some(enc) if !enc.as_bytes().eq_ignore_ascii_case(b"identity") => Err(
                FlagError::Unsupported(grpc_message("Unsupported grpc-encoding", enc)),
            ),
            _ => Err(FlagError::Invalid(HeaderValue::from_static(
                "Compressed message without grpc-encoding",
            ))),
        },
        _ => Err(FlagError::Invalid(
            HeaderValue::try_from(format!("Invalid compressed flag {flag}"))
                .unwrap_or_else(|_| HeaderValue::from_static("Invalid compressed flag")),
        )),
    }
}

/// Returns the message, decompressed if needed.
///
/// `unsupported` is the status of a message in an encoding we do not
/// support. The decompressed message must not be larger than `max_size`.
#[cfg_attr(not(feature = "compression"), allow(clippy::unused_async))]
pub(crate) async fn read_message(
    flag: u8,
    hdrs: &HeaderMap,
    block: Bytes,
    max_size: usize,
    unsupported: GrpcStatus,
) -> Result<Bytes, (GrpcStatus, HeaderValue)> {
    #[cfg(feature = "compression")]
    if flag == 1
        && let Some(enc) = hdrs
            .get(consts::GRPC_ENCODING)
            .and_then(Compression::from_header)
    {
        return enc.decompress(block, max_size).await;
    }
    #[cfg(not(feature = "compression"))]
    let _ = max_size;

    match check_compressed_flag(flag, hdrs) {
        Ok(()) => Ok(block),
        Err(FlagError::Unsupported(msg)) => Err((unsupported, msg)),
        Err(FlagError::Invalid(msg)) => Err((GrpcStatus::Internal, msg)),
    }
}

/// Builds a `grpc-message` of `prefix: value`.
///
/// `grpc-message` is percent-encoded, so the value is only added if it is
/// plain text.
pub(crate) fn grpc_message(prefix: &'static str, val: &HeaderValue) -> HeaderValue {
    val.to_str()
        .ok()
        .filter(|v| !v.contains(['%', '\t']))
        .and_then(|v| HeaderValue::try_from(format!("{prefix}: {v}")).ok())
        .unwrap_or_else(|| HeaderValue::from_static(prefix))
}

/// Decodes a percent-encoded `grpc-message`.
///
/// Invalid escapes are kept as they are, the spec says a bad message must
/// not be dropped.
pub(crate) fn percent_decode(val: &HeaderValue) -> Cow<'_, str> {
    match val.to_str() {
        Ok(s) => unquote(s, Component::Opaque),
        // not printable ascii, so not encoded as the spec says
        Err(_) => String::from_utf8_lossy(val.as_bytes()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(data: &mut Data, chunk: &[u8]) {
        data.push(Bytes::copy_from_slice(chunk), 100);
    }

    fn capacity(data: &Data) -> usize {
        let Data::MutChunk(buf) = data else {
            panic!("not a buffer")
        };
        buf.capacity()
    }

    #[test]
    fn data_push() {
        let msg = [&[0, 0, 0, 0, 100][..], &[1; 100]].concat();

        // a single chunk is not copied
        let chunk = Bytes::copy_from_slice(&msg);
        let mut data = Data::Empty;
        data.push(chunk.clone(), 100);
        data.push(Bytes::new(), 100);
        assert_eq!(data.get().as_ptr(), chunk.as_ptr());

        // the buffer is sized once for the whole message
        for split in [&[10, 50][..], &[2, 4, 50], &[1, 1, 1, 1, 1, 50]] {
            let mut data = Data::Empty;
            let mut pos = 0;
            for &end in split {
                push(&mut data, &msg[pos..end]);
                pos = end;
            }
            assert_eq!(capacity(&data), 105);
            let ptr = data.as_slice().as_ptr();
            push(&mut data, &msg[pos..]);
            assert_eq!(capacity(&data), 105);
            assert_eq!(data.as_slice().as_ptr(), ptr);
            assert_eq!(data.get(), msg);
        }

        // the prefix is not complete yet
        let mut data = Data::Empty;
        push(&mut data, &msg[..2]);
        push(&mut data, &msg[2..4]);
        assert_eq!(capacity(&data), 4);

        // the reserved size does not depend on how the buffer grows
        let mut data = Data::Empty;
        push(&mut data, &msg[..60]);
        push(&mut data, &msg[60..70]);
        assert_eq!(capacity(&data), 105);

        // a message over the limit is not reserved for
        let big = [&[0, 0, 0, 0, 101][..], &[1; 101]].concat();
        let mut data = Data::Empty;
        push(&mut data, &big[..10]);
        push(&mut data, &big[10..20]);
        assert!(capacity(&data) < 106);
        push(&mut data, &big[20..]);
        assert_eq!(data.get(), big);

        // data after the message
        let msg = [&msg[..], &[1; 20]].concat();
        let mut data = Data::Empty;
        push(&mut data, &msg[..10]);
        push(&mut data, &msg[10..]);
        assert_eq!(data.get(), msg);
    }

    #[test]
    fn binary_header() {
        assert_eq!(encode_binary_header(b""), "");
        assert_eq!(encode_binary_header(b"a"), "YQ");
        assert_eq!(encode_binary_header(b"ab"), "YWI");
        assert_eq!(encode_binary_header(b"abc"), "YWJj");

        let dec = |v: &'static str| decode_binary_header(&HeaderValue::from_static(v));
        assert_eq!(dec("YQ").unwrap(), b"a");
        assert_eq!(dec("YQ==").unwrap(), b"a");
        assert_eq!(dec("YWI=").unwrap(), b"ab");
        assert_eq!(dec("YWJj").unwrap(), b"abc");
        assert_eq!(dec("").unwrap(), b"");
        assert_eq!(dec("/+8").unwrap(), [0xff, 0xef]);
        // url-safe alphabet and bad padding are rejected
        assert!(dec("_-8").is_none());
        assert!(dec("YQ=").is_none());
        assert!(dec("Y").is_none());
        assert!(dec("YQ=\t").is_none());
    }

    #[test]
    fn percent_decode_message() {
        let dec = |v: &'static str| percent_decode(&HeaderValue::from_static(v)).into_owned();
        let plain = HeaderValue::from_static("plain text");
        assert!(matches!(
            percent_decode(&plain),
            Cow::Borrowed("plain text")
        ));
        assert_eq!(dec("100%25 done"), "100% done");
        assert_eq!(dec("a%0Ab%0a"), "a\nb\n");
        assert_eq!(dec("%E2%82%AC"), "\u{20ac}");
        // `+` is not a space here
        assert_eq!(dec("a+b"), "a+b");
        // bad escapes stay as they are
        assert_eq!(dec("%"), "%");
        assert_eq!(dec("%4"), "%4");
        assert_eq!(dec("%zz%41"), "%zzA");
        assert_eq!(dec("%FFok"), "%FFok");
        // obs-text is not valid, but still readable
        let raw = HeaderValue::from_bytes(b"caf\xc3\xa9").unwrap();
        assert_eq!(percent_decode(&raw), "caf\u{e9}");
    }
}
