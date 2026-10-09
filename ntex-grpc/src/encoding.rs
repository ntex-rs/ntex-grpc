//! protobuf encoding utils
//! cloned from `<https://github.com/hyperium/tonic/>`
use std::{borrow::Cow, cmp::min, convert::TryFrom, fmt, rc::Rc};

use ntex_bytes::{Buf, BufMut, BytePages, Bytes};

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
pub fn encode_varint(mut value: u64, buf: &mut BytePages) {
    loop {
        if value < 0x80 {
            buf.put_u8(value as u8);
            break;
        }
        buf.put_u8(((value & 0x7F) | 0x80) as u8);
        value >>= 7;
    }
}

/// Decodes a LEB128-encoded variable length integer from the buffer.
#[inline]
pub fn decode_varint(buf: &mut Bytes) -> Result<u64, DecodeError> {
    let bytes = buf.as_ref();
    let len = buf.len();
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
#[allow(clippy::assert_is_empty)]
fn decode_varint_slice(bytes: &[u8]) -> Result<(u64, usize), DecodeError> {
    // Fully unrolled varint decoding loop. Splitting into 32-bit pieces gives better performance.

    // Use assertions to ensure memory safety, but it should always be optimized after inline.
    assert!(!bytes.is_empty());
    assert!(bytes.len() > 10 || bytes[bytes.len() - 1] < 0x80);

    let mut b: u8 = unsafe { *bytes.get_unchecked(0) };
    let mut part0: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((u64::from(part0), 1));
    }
    part0 -= 0x80;
    b = unsafe { *bytes.get_unchecked(1) };
    part0 += u32::from(b) << 7;
    if b < 0x80 {
        return Ok((u64::from(part0), 2));
    }
    part0 -= 0x80 << 7;
    b = unsafe { *bytes.get_unchecked(2) };
    part0 += u32::from(b) << 14;
    if b < 0x80 {
        return Ok((u64::from(part0), 3));
    }
    part0 -= 0x80 << 14;
    b = unsafe { *bytes.get_unchecked(3) };
    part0 += u32::from(b) << 21;
    if b < 0x80 {
        return Ok((u64::from(part0), 4));
    }
    part0 -= 0x80 << 21;
    let value = u64::from(part0);

    b = unsafe { *bytes.get_unchecked(4) };
    let mut part1: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 5));
    }
    part1 -= 0x80;
    b = unsafe { *bytes.get_unchecked(5) };
    part1 += u32::from(b) << 7;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 6));
    }
    part1 -= 0x80 << 7;
    b = unsafe { *bytes.get_unchecked(6) };
    part1 += u32::from(b) << 14;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 7));
    }
    part1 -= 0x80 << 14;
    b = unsafe { *bytes.get_unchecked(7) };
    part1 += u32::from(b) << 21;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 8));
    }
    part1 -= 0x80 << 21;
    let value = value + ((u64::from(part1)) << 28);

    b = unsafe { *bytes.get_unchecked(8) };
    let mut part2: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((value + (u64::from(part2) << 56), 9));
    }
    part2 -= 0x80;
    b = unsafe { *bytes.get_unchecked(9) };
    part2 += u32::from(b) << 7;
    // Check for u64::MAX overflow. See [`ConsumeVarint`][1] for details.
    // [1]: https://github.com/protocolbuffers/protobuf-go/blob/v1.27.1/encoding/protowire/wire.go#L358
    if b < 0x02 {
        return Ok((value + (u64::from(part2) << 56), 10));
    }

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
fn decode_varint_slow(buf: &mut Bytes) -> Result<u64, DecodeError> {
    let mut value = 0;
    for count in 0..min(10, buf.remaining()) {
        let byte = buf.get_u8();
        value |= u64::from(byte & 0x7F) << (count * 7);
        if byte <= 0x7F {
            // Check for u64::MAX overflow. See [`ConsumeVarint`][1] for details.
            // [1]: https://github.com/protocolbuffers/protobuf-go/blob/v1.27.1/encoding/protowire/wire.go#L358
            return if count == 9 && byte >= 0x02 {
                Err(DecodeError::new("invalid varint"))
            } else {
                Ok(value)
            };
        }
    }

    Err(DecodeError::new("invalid varint"))
}

/// Encodes a Protobuf field key, which consists of a wire type designator and
/// the field tag.
#[inline]
pub fn encode_key(tag: u32, wire_type: WireType, buf: &mut BytePages) {
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

/// Splits the body of a group off the buffer.
///
/// The buffer must start right after the start group key with the given tag.
/// Returns the fields between the keys and advances the buffer past the
/// matching end group key.
pub fn split_group(tag: u32, buf: &mut Bytes) -> Result<Bytes, DecodeError> {
    let mut rest = buf.clone();
    loop {
        let body_len = buf.len() - rest.len();
        let (inner_tag, inner_wire_type) = decode_key(&mut rest)?;
        if inner_wire_type == WireType::EndGroup {
            if inner_tag != tag {
                return Err(DecodeError::new("unexpected end group tag"));
            }
            let body = buf.split_to(body_len);
            *buf = rest;
            return Ok(body);
        }
        skip_field(inner_wire_type, inner_tag, &mut rest)?;
    }
}

pub fn skip_field(wire_type: WireType, tag: u32, buf: &mut Bytes) -> Result<(), DecodeError> {
    let len = match wire_type {
        WireType::Varint => decode_varint(buf).map(|_| 0)?,
        WireType::ThirtyTwoBit => 4,
        WireType::SixtyFourBit => 8,
        WireType::LengthDelimited => decode_varint(buf)?,
        WireType::StartGroup => split_group(tag, buf).map(|_| 0)?,
        WireType::EndGroup => return Err(DecodeError::new("unexpected end group tag")),
    };

    buf.split_to_checked(len as usize)
        .ok_or_else(DecodeError::incomplete)?;
    Ok(())
}

/// A Protobuf message decoding error.
#[derive(Clone, PartialEq, Eq)]
pub struct DecodeError {
    inner: Rc<Inner>,
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
            inner: Rc::new(Inner {
                description: description.into(),
                stack: Vec::new(),
            }),
        }
    }

    /// Pushes a (message, field) name location pair on to the location stack.
    ///
    /// Meant to be used only by `Message` implementations.
    #[doc(hidden)]
    #[must_use]
    pub fn push(mut self, message: &'static str, field: &'static str) -> Self {
        let inner = if let Some(inner) = Rc::get_mut(&mut self.inner) {
            inner
        } else {
            self.inner = Rc::new(Inner {
                description: self.inner.description.clone(),
                stack: self.inner.stack.clone(),
            });
            Rc::get_mut(&mut self.inner).unwrap()
        };
        inner.stack.push((message, field));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(value: u64) -> Bytes {
        let mut buf = BytePages::default();
        encode_varint(value, &mut buf);
        buf.freeze()
    }

    #[test]
    fn varint_roundtrip() {
        let cases: [(u64, usize); 18] = [
            (0, 1),
            (1, 1),
            (0x7f, 1),
            (0x80, 2),
            (300, 2),
            (0x3fff, 2),
            (0x4000, 3),
            (0x1f_ffff, 3),
            (0x20_0000, 4),
            (0x0fff_ffff, 4),
            (0x1000_0000, 5),
            (u64::from(u32::MAX), 5),
            (1 << 35, 6),
            (1 << 42, 7),
            (1 << 49, 8),
            (1 << 56, 9),
            (1 << 63, 10),
            (u64::MAX, 10),
        ];

        for (value, len) in cases {
            let bytes = encode(value);
            assert_eq!(bytes.len(), len, "{value}");
            assert_eq!(encoded_len_varint(value), len, "{value}");

            let mut buf = bytes.clone();
            assert_eq!(decode_varint(&mut buf).unwrap(), value);
            assert!(buf.is_empty());

            // the unrolled slice decoder returns the same value and width
            assert_eq!(decode_varint_slice(&bytes).unwrap(), (value, len));
        }
    }

    #[test]
    fn varint_decode_errors() {
        // empty buffer
        assert!(decode_varint(&mut Bytes::new()).is_err());

        // last byte overflows u64, slice path
        let mut overflow = vec![0xff_u8; 9];
        overflow.push(0x02);
        assert!(decode_varint(&mut Bytes::from(overflow.clone())).is_err());
        assert!(decode_varint_slice(&overflow).is_err());

        // more than 10 continuation bytes, slice path
        assert!(decode_varint(&mut Bytes::from(vec![0xff_u8; 11])).is_err());

        // truncated varint, slow path
        assert!(decode_varint(&mut Bytes::from_static(&[0x80])).is_err());
        assert!(decode_varint(&mut Bytes::from(vec![0xff_u8; 10])).is_err());
    }

    #[test]
    fn varint_slow_path_stops_at_terminator() {
        // trailing continuation byte forces the slow path, the value still ends early
        let mut buf = Bytes::from_static(&[0x80, 0x01, 0x80]);
        assert_eq!(decode_varint(&mut buf).unwrap(), 128);
        assert_eq!(buf, Bytes::from_static(&[0x80]));
    }

    #[test]
    fn wire_type_from_u64() {
        let all = [
            WireType::Varint,
            WireType::SixtyFourBit,
            WireType::LengthDelimited,
            WireType::StartGroup,
            WireType::EndGroup,
            WireType::ThirtyTwoBit,
        ];
        for (value, wire_type) in all.iter().enumerate() {
            assert_eq!(WireType::try_from(value as u64).unwrap(), *wire_type);
        }

        let err = WireType::try_from(6).unwrap_err();
        assert!(err.to_string().contains("invalid wire type value: 6"));
        assert!(WireType::try_from(u64::MAX).is_err());
    }

    #[test]
    fn key_roundtrip() {
        let cases = [
            (1_u32, WireType::Varint),
            (2, WireType::SixtyFourBit),
            (3, WireType::LengthDelimited),
            (4, WireType::StartGroup),
            (5, WireType::EndGroup),
            (16, WireType::ThirtyTwoBit),
            (2047, WireType::Varint),
            (MAX_TAG, WireType::LengthDelimited),
        ];

        for (tag, wire_type) in cases {
            let mut buf = BytePages::default();
            encode_key(tag, wire_type, &mut buf);
            let bytes = buf.freeze();
            assert_eq!(bytes.len(), key_len(tag), "{tag}");

            let mut src = bytes;
            assert_eq!(decode_key(&mut src).unwrap(), (tag, wire_type));
            assert!(src.is_empty());
        }
    }

    #[test]
    fn key_decode_errors() {
        // key does not fit into u32
        let err = decode_key(&mut encode(u64::from(u32::MAX) + 1)).unwrap_err();
        assert!(err.to_string().contains("invalid key value"));

        // wire types 6 and 7 do not exist
        for wire_type in [6_u64, 7] {
            let err = decode_key(&mut encode((8 << 3) | wire_type)).unwrap_err();
            assert!(err.to_string().contains("invalid wire type value"));
        }

        // tag 0 is reserved
        let err = decode_key(&mut Bytes::from_static(&[0x02])).unwrap_err();
        assert!(err.to_string().contains("invalid tag value: 0"));

        assert!(decode_key(&mut Bytes::new()).is_err());
    }

    #[test]
    fn check_wire_type_mismatch() {
        assert!(check_wire_type(WireType::Varint, WireType::Varint).is_ok());

        let err = check_wire_type(WireType::Varint, WireType::LengthDelimited).unwrap_err();
        assert_eq!(
            err.to_string(),
            "failed to decode Protobuf message: invalid wire type: LengthDelimited (expected Varint)"
        );
    }

    #[test]
    fn skip_simple_fields() {
        let cases: [(WireType, &[u8]); 4] = [
            (WireType::Varint, &[0x96, 0x01]),
            (WireType::ThirtyTwoBit, &[1, 2, 3, 4]),
            (WireType::SixtyFourBit, &[1, 2, 3, 4, 5, 6, 7, 8]),
            (WireType::LengthDelimited, &[0x03, b'a', b'b', b'c']),
        ];

        for (wire_type, data) in cases {
            let mut src = BytePages::default();
            src.extend_from_slice(data);
            src.extend_from_slice(&[0xff]);
            let mut buf = src.freeze();

            skip_field(wire_type, 1, &mut buf).unwrap();
            assert_eq!(buf, Bytes::from_static(&[0xff]), "{wire_type:?}");
        }
    }

    #[test]
    fn skip_group_field() {
        // group 3 { field 1 varint; group 2 { field 1 varint } }
        let mut buf = BytePages::default();
        encode_key(1, WireType::Varint, &mut buf);
        encode_varint(1, &mut buf);
        encode_key(2, WireType::StartGroup, &mut buf);
        encode_key(1, WireType::Varint, &mut buf);
        encode_varint(7, &mut buf);
        encode_key(2, WireType::EndGroup, &mut buf);
        encode_key(3, WireType::EndGroup, &mut buf);
        buf.extend_from_slice(&[0xff]);

        let mut buf = buf.freeze();
        skip_field(WireType::StartGroup, 3, &mut buf).unwrap();
        assert_eq!(buf, Bytes::from_static(&[0xff]));
    }

    #[test]
    fn split_group_body() {
        // group 3 { field 1 varint; group 2 { } } followed by 0xff
        let mut buf = BytePages::default();
        encode_key(1, WireType::Varint, &mut buf);
        encode_varint(1, &mut buf);
        encode_key(2, WireType::StartGroup, &mut buf);
        encode_key(2, WireType::EndGroup, &mut buf);
        encode_key(3, WireType::EndGroup, &mut buf);
        buf.extend_from_slice(&[0xff]);

        let mut buf = buf.freeze();
        let body = split_group(3, &mut buf).unwrap();
        assert_eq!(body, Bytes::from_static(&[0x08, 0x01, 0x13, 0x14]));
        assert_eq!(buf, Bytes::from_static(&[0xff]));

        // empty group
        let mut buf = Bytes::from_static(&[0x1c]);
        assert!(split_group(3, &mut buf).unwrap().is_empty());
        assert!(buf.is_empty());

        // buffer is not advanced on error
        let mut buf = Bytes::from_static(&[0x08, 0x01, 0x14]);
        assert!(split_group(3, &mut buf).is_err());
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn skip_field_errors() {
        // end group tag does not match the group being skipped
        let mut buf = BytePages::default();
        encode_key(4, WireType::EndGroup, &mut buf);
        let err = skip_field(WireType::StartGroup, 3, &mut buf.freeze()).unwrap_err();
        assert!(err.to_string().contains("unexpected end group tag"));

        // a bare end group is always an error
        let err = skip_field(WireType::EndGroup, 1, &mut Bytes::new()).unwrap_err();
        assert!(err.to_string().contains("unexpected end group tag"));

        // length delimited field longer than the buffer
        let mut buf = Bytes::from_static(&[0x05, 1, 2]);
        let err = skip_field(WireType::LengthDelimited, 1, &mut buf).unwrap_err();
        assert!(err.to_string().contains("Not enough data"));

        // truncated group
        let mut buf = Bytes::from_static(&[0x08]);
        assert!(skip_field(WireType::StartGroup, 3, &mut buf).is_err());
    }

    #[test]
    fn decode_error_stack() {
        let err = DecodeError::new("boom");
        assert_eq!(err.to_string(), "failed to decode Protobuf message: boom");

        // the error is not shared, the stack is pushed in place
        let err = err.push("Msg", "field");
        assert_eq!(
            err.to_string(),
            "failed to decode Protobuf message: Msg.field: boom"
        );

        // the error is shared, push has to clone the inner state
        let shared = err.clone();
        let err = err.push("Outer", "inner");
        assert_eq!(
            err.to_string(),
            "failed to decode Protobuf message: Msg.field: Outer.inner: boom"
        );
        assert_eq!(
            shared.to_string(),
            "failed to decode Protobuf message: Msg.field: boom"
        );
        assert_ne!(err, shared);
        assert_eq!(shared, shared.clone());

        let dbg = format!("{err:?}");
        assert!(dbg.contains("DecodeError"));
        assert!(dbg.contains("boom"));
        assert!(dbg.contains("Outer"));
    }

    #[test]
    fn decode_error_incomplete() {
        assert_eq!(
            DecodeError::incomplete().to_string(),
            "failed to decode Protobuf message: Not enough data"
        );
    }
}
