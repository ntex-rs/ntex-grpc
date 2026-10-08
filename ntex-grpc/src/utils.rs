use std::mem;

use ntex_bytes::{Bytes, BytesMut};
use ntex_http::{HeaderMap, HeaderValue};

use crate::consts;

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

    pub(crate) fn push(&mut self, data: Bytes) {
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

/// Why a message's compressed flag cannot be handled.
pub(crate) enum FlagError {
    /// The message is compressed with an encoding we do not support.
    Unsupported(HeaderValue),
    /// The flag is invalid, or set without a `grpc-encoding`.
    Invalid(HeaderValue),
}

/// Checks the compressed flag of a message, only identity is supported.
pub(crate) fn check_compressed_flag(flag: u8, hdrs: &HeaderMap) -> Result<(), FlagError> {
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
