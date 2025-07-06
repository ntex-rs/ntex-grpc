/// protobuf encoding utils
/// cloned from https://github.com/hyperium/tonic/
use std::{borrow::Cow, cmp::min, convert::TryFrom, fmt};

use ntex_bytes::{Buf, BufMut, Bytes, BytesMut};

pub const MIN_TAG: u32 = 1;
pub const MAX_TAG: u32 = (1 << 29) - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(u8)]
pub enum WireType {
    Varint = 0,
    SixtyFourBit = 1,
    LengthDelimited = 2,
    StartGroup = 3,
    EndGroup = 4,
    ThirtyTwoBit = 5,
}

impl TryFrom<u64> for WireType {
    type Error = DecodeError;

    #[inline]
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(WireType::Varint),
            1 => Ok(WireType::SixtyFourBit),
            2 => Ok(WireType::LengthDelimited),
            3 => Ok(WireType::StartGroup),
            4 => Ok(WireType::EndGroup),
            5 => Ok(WireType::ThirtyTwoBit),
            _ => Err(DecodeError::new(format!(
                "invalid wire type value: {value}"
            ))),
        }
    }
}

/// Returns the encoded length of the value in LEB128 variable length format.
/// The returned value will be between 1 and 10, inclusive.
#[inline]
pub fn encoded_len_varint(value: u64) -> usize {
    // Based on [VarintSize64][1].
    // [1]: https://github.com/google/protobuf/blob/3.3.x/src/google/protobuf/io/coded_stream.h#L1301-L1309
    ((((value | 1).leading_zeros() ^ 63) * 9 + 73) / 64) as usize
}

/// Encodes an integer value into LEB128 variable length format, and writes it to the buffer.
/// The buffer must have enough remaining space (maximum 10 bytes).
#[inline]
pub fn encode_varint(mut value: u64, buf: &mut BytesMut) {
    loop {
        if value < 0x80 {
            buf.put_u8(value as u8);
            break;
        } else {
            buf.put_u8(((value & 0x7F) | 0x80) as u8);
            value >>= 7;
        }
    }
}

/// Decodes a LEB128-encoded variable length integer from the buffer.
#[inline]
pub fn decode_varint(buf: &mut Bytes) -> Result<u64, DecodeError> {
    let bytes = buf.chunk();
    let len = bytes.len();
    if len == 0 {
        return Err(DecodeError::new("invalid varint"));
    }

    let byte = bytes[0];
    if byte < 0x80 {
        buf.advance(1);
        Ok(u64::from(byte))
    } else if len > 10 || bytes[len - 1] < 0x80 {
        let (value, advance) = decode_varint_slice(bytes)?;
        buf.advance(advance);
        Ok(value)
    } else {
        decode_varint_slow(buf)
    }
}

/// Decodes a LEB128-encoded variable length integer from the slice, returning the value and the
/// number of bytes read.
///
/// Based loosely on [`ReadVarint64FromArray`][1] with a varint overflow check from
/// [`ConsumeVarint`][2].
///
/// ## Safety
///
/// The caller must ensure that `bytes` is non-empty and either `bytes.len() >= 10` or the last
/// element in bytes is < `0x80`.
///
/// [1]: https://github.com/google/protobuf/blob/3.3.x/src/google/protobuf/io/coded_stream.cc#L365-L406
/// [2]: https://github.com/protocolbuffers/protobuf-go/blob/v1.27.1/encoding/protowire/wire.go#L358
#[inline]
fn decode_varint_slice(bytes: &[u8]) -> Result<(u64, usize), DecodeError> {
    // Fully unrolled varint decoding loop. Splitting into 32-bit pieces gives better performance.

    // Use assertions to ensure memory safety, but it should always be optimized after inline.
    assert!(!bytes.is_empty());
    assert!(bytes.len() > 10 || bytes[bytes.len() - 1] < 0x80);

    let mut b: u8 = unsafe { *bytes.get_unchecked(0) };
    let mut part0: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((u64::from(part0), 1));
    };
    part0 -= 0x80;
    b = unsafe { *bytes.get_unchecked(1) };
    part0 += u32::from(b) << 7;
    if b < 0x80 {
        return Ok((u64::from(part0), 2));
    };
    part0 -= 0x80 << 7;
    b = unsafe { *bytes.get_unchecked(2) };
    part0 += u32::from(b) << 14;
    if b < 0x80 {
        return Ok((u64::from(part0), 3));
    };
    part0 -= 0x80 << 14;
    b = unsafe { *bytes.get_unchecked(3) };
    part0 += u32::from(b) << 21;
    if b < 0x80 {
        return Ok((u64::from(part0), 4));
    };
    part0 -= 0x80 << 21;
    let value = u64::from(part0);

    b = unsafe { *bytes.get_unchecked(4) };
    let mut part1: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 5));
    };
    part1 -= 0x80;
    b = unsafe { *bytes.get_unchecked(5) };
    part1 += u32::from(b) << 7;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 6));
    };
    part1 -= 0x80 << 7;
    b = unsafe { *bytes.get_unchecked(6) };
    part1 += u32::from(b) << 14;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 7));
    };
    part1 -= 0x80 << 14;
    b = unsafe { *bytes.get_unchecked(7) };
    part1 += u32::from(b) << 21;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 8));
    };
    part1 -= 0x80 << 21;
    let value = value + ((u64::from(part1)) << 28);

    b = unsafe { *bytes.get_unchecked(8) };
    let mut part2: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((value + (u64::from(part2) << 56), 9));
    };
    part2 -= 0x80;
    b = unsafe { *bytes.get_unchecked(9) };
    part2 += u32::from(b) << 7;
    // Check for u64::MAX overflow. See [`ConsumeVarint`][1] for details.
    // [1]: https://github.com/protocolbuffers/protobuf-go/blob/v1.27.1/encoding/protowire/wire.go#L358
    if b < 0x02 {
        return Ok((value + (u64::from(part2) << 56), 10));
    };

    // We have overrun the maximum size of a varint (10 bytes) or the final byte caused an overflow.
    // Assume the data is corrupt.
    Err(DecodeError::new("invalid varint"))
}

/// Decodes a LEB128-encoded variable length integer from the buffer, advancing the buffer as
/// necessary.
///
/// Contains a varint overflow check from [`ConsumeVarint`][1].
///
/// [1]: https://github.com/protocolbuffers/protobuf-go/blob/v1.27.1/encoding/protowire/wire.go#L358
#[inline(never)]
#[cold]
fn decode_varint_slow<B>(buf: &mut B) -> Result<u64, DecodeError>
where
    B: Buf,
{
    let mut value = 0;
    for count in 0..min(10, buf.remaining()) {
        let byte = buf.get_u8();
        value |= u64::from(byte & 0x7F) << (count * 7);
        if byte <= 0x7F {
            // Check for u64::MAX overflow. See [`ConsumeVarint`][1] for details.
            // [1]: https://github.com/protocolbuffers/protobuf-go/blob/v1.27.1/encoding/protowire/wire.go#L358
            if count == 9 && byte >= 0x02 {
                return Err(DecodeError::new("invalid varint"));
            } else {
                return Ok(value);
            }
        }
    }

    Err(DecodeError::new("invalid varint"))
}

/// Encodes a Protobuf field key, which consists of a wire type designator and
/// the field tag.
#[inline]
pub fn encode_key(tag: u32, wire_type: WireType, buf: &mut BytesMut) {
    debug_assert!((MIN_TAG..=MAX_TAG).contains(&tag));
    let key = (tag << 3) | wire_type as u32;
    encode_varint(u64::from(key), buf);
}

/// Decodes a Protobuf field key, which consists of a wire type designator and
/// the field tag.
#[inline]
pub fn decode_key(buf: &mut Bytes) -> Result<(u32, WireType), DecodeError> {
    let key = decode_varint(buf)?;
    if key > u64::from(u32::MAX) {
        return Err(DecodeError::new(format!("invalid key value: {key}")));
    }
    let wire_type = WireType::try_from(key & 0x07)?;
    let tag = key as u32 >> 3;

    if tag < MIN_TAG {
        return Err(DecodeError::new("invalid tag value: 0"));
    }

    Ok((tag, wire_type))
}

/// Returns the width of an encoded Protobuf field key with the given tag.
/// The returned width will be between 1 and 5 bytes (inclusive).
#[inline]
pub fn key_len(tag: u32) -> usize {
    encoded_len_varint(u64::from(tag << 3))
}

/// Checks that the expected wire type matches the actual wire type,
/// or returns an error result.
#[inline]
pub fn check_wire_type(expected: WireType, actual: WireType) -> Result<(), DecodeError> {
    if expected != actual {
        return Err(DecodeError::new(format!(
            "invalid wire type: {actual:?} (expected {expected:?})",
        )));
    }
    Ok(())
}

pub fn skip_field(wire_type: WireType, tag: u32, buf: &mut Bytes) -> Result<(), DecodeError> {
    let len = match wire_type {
        WireType::Varint => decode_varint(buf).map(|_| 0)?,
        WireType::ThirtyTwoBit => 4,
        WireType::SixtyFourBit => 8,
        WireType::LengthDelimited => decode_varint(buf)?,
        WireType::StartGroup => loop {
            let (inner_tag, inner_wire_type) = decode_key(buf)?;
            match inner_wire_type {
                WireType::EndGroup => {
                    if inner_tag != tag {
                        return Err(DecodeError::new("unexpected end group tag"));
                    }
                    break 0;
                }
                _ => skip_field(inner_wire_type, inner_tag, buf)?,
            }
        },
        WireType::EndGroup => return Err(DecodeError::new("unexpected end group tag")),
    };

    buf.split_to_checked(len as usize)
        .ok_or_else(DecodeError::incomplete)?;
    Ok(())
}

/// A Protobuf message decoding error.
#[derive(Clone, PartialEq, Eq)]
pub struct DecodeError {
    inner: Box<Inner>,
}

#[derive(Clone, PartialEq, Eq)]
struct Inner {
    /// A 'best effort' root cause description.
    description: Cow<'static, str>,
    /// A stack of (message, field) name pairs, which identify the specific
    /// message type and field where decoding failed. The stack contains an
    /// entry per level of nesting.
    stack: Vec<(&'static str, &'static str)>,
}

impl DecodeError {
    /// Creates a new `DecodeError` with a 'best effort' root cause description.
    ///
    /// Meant to be used only by `Message` implementations.
    #[doc(hidden)]
    #[cold]
    pub fn new(description: impl Into<Cow<'static, str>>) -> DecodeError {
        DecodeError {
            inner: Box::new(Inner {
                description: description.into(),
                stack: Vec::new(),
            }),
        }
    }

    /// Pushes a (message, field) name location pair on to the location stack.
    ///
    /// Meant to be used only by `Message` implementations.
    #[doc(hidden)]
    pub fn push(mut self, message: &'static str, field: &'static str) -> Self {
        self.inner.stack.push((message, field));
        self
    }

    pub(crate) fn incomplete() -> Self {
        Self::new("Not enough data")
    }
}

impl fmt::Debug for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecodeError")
            .field("description", &self.inner.description)
            .field("stack", &self.inner.stack)
            .finish()
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("failed to decode Protobuf message: ")?;
        for &(message, field) in &self.inner.stack {
            write!(f, "{message}.{field}: ")?;
        }
        f.write_str(&self.inner.description)
    }
}

impl std::error::Error for DecodeError {}
