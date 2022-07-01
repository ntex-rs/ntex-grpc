use std::{collections::HashMap, convert::TryFrom, fmt, hash::BuildHasher, hash::Hash};

pub use ntex_bytes::{ByteString, Bytes, BytesMut};

pub use crate::encoding::WireType;
use crate::encoding::{self, DecodeError};

/// Protobuf struct read/write operations
pub trait Message: Default + Sized + fmt::Debug {
    /// Decodes an instance of the message from a buffer
    fn read(src: &mut Bytes) -> Result<Self, DecodeError>;

    /// Encodes and writes the message to a buffer
    fn write(&self, dst: &mut BytesMut);

    /// Returns the encoded length of the message with a length delimiter
    fn encoded_len(&self) -> usize;
}

pub trait OneofType: Sized {
    /// Encodes the message to a buffer
    fn encode(&self, buf: &mut BytesMut);

    /// Decodes an instance of the oneof message from a buffer and update self
    fn merge(&mut self, tag: u32, wtype: WireType, buf: &mut Bytes) -> Result<(), DecodeError> {
        *self = Self::decode(tag, wtype, buf)?;
        Ok(())
    }

    /// Decodes an instance of the oneof message from a buffer
    fn decode(tag: u32, wtype: WireType, buf: &mut Bytes) -> Result<Self, DecodeError>;

    /// Returns the encoded length of the message without a length delimiter
    fn encoded_len(&self) -> usize;
}

/// Protobuf type serializer
pub trait NativeType: Default + Sized + fmt::Debug {
    const TYPE: WireType;

    /// Returns the encoded length of the message without a length delimiter.
    fn value_len(&self) -> usize;

    /// Deserialize from the input
    fn merge(&mut self, src: Bytes) -> Result<(), DecodeError>;

    /// Serialize field
    fn encode(&self, dst: &mut BytesMut);

    #[inline]
    /// Check if value is default
    fn is_default(&self, _: &[u8]) -> bool {
        false
    }

    #[inline]
    /// Protobuf field length
    fn field_len(&self, tag: u32) -> usize {
        encoding::key_len(tag)
            + encoding::encoded_len_varint(self.value_len() as u64)
            + self.value_len()
    }

    #[inline]
    /// Serialize protobuf field
    fn serialize(&self, tag: u32, dst: &mut BytesMut) {
        encoding::encode_key(tag, Self::TYPE, dst);
        encoding::encode_varint(self.value_len() as u64, dst);
        self.encode(dst);
    }

    #[inline]
    /// Deserialize protobuf field
    fn deserialize(&mut self, wtype: WireType, src: &mut Bytes) -> Result<(), DecodeError> {
        encoding::check_wire_type(Self::TYPE, wtype)?;
        let len = encoding::decode_varint(src)? as usize;
        if len > src.len() {
            Err(DecodeError::new("Not enough data"))
        } else {
            let buf = src.split_to(len);
            self.merge(buf)
        }
    }

    #[inline]
    /// Deserialize protobuf field to default value
    fn deserialize_default(wtype: WireType, src: &mut Bytes) -> Result<Self, DecodeError> {
        let mut value = Self::default();
        value.deserialize(wtype, src)?;
        Ok(value)
    }
}

impl<T: Message> NativeType for T {
    const TYPE: WireType = WireType::LengthDelimited;

    fn value_len(&self) -> usize {
        Message::encoded_len(self)
    }

    /// Encode message to the buffer
    fn encode(&self, dst: &mut BytesMut) {
        self.write(dst)
    }

    /// Deserialize from the input
    fn merge(&mut self, mut src: Bytes) -> Result<(), DecodeError> {
        *self = Message::read(&mut src)?;
        Ok(())
    }
}

impl<T: OneofType> OneofType for Option<T> {
    #[inline]
    fn encode(&self, buf: &mut BytesMut) {
        if let Some(ref inner) = self {
            inner.encode(buf)
        }
    }

    #[inline]
    fn merge(&mut self, tag: u32, wtype: WireType, buf: &mut Bytes) -> Result<(), DecodeError> {
        if let Some(ref mut inner) = self {
            inner.merge(tag, wtype, buf)?;
        } else {
            *self = Some(T::decode(tag, wtype, buf)?);
        }
        Ok(())
    }

    #[inline]
    fn decode(tag: u32, wtype: WireType, buf: &mut Bytes) -> Result<Self, DecodeError> {
        Ok(Some(T::decode(tag, wtype, buf)?))
    }

    #[inline]
    fn encoded_len(&self) -> usize {
        self.as_ref().map(|inner| inner.encoded_len()).unwrap_or(0)
    }
}

impl NativeType for Bytes {
    const TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    fn value_len(&self) -> usize {
        self.len()
    }

    #[inline]
    /// Serialize field value
    fn encode(&self, dst: &mut BytesMut) {
        dst.extend_from_slice(self);
    }

    #[inline]
    /// Deserialize from the input
    fn merge(&mut self, src: Bytes) -> Result<(), DecodeError> {
        *self = src;
        Ok(())
    }

    #[inline]
    fn is_default(&self, default: &[u8]) -> bool {
        self == default
    }
}

impl NativeType for String {
    const TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    fn value_len(&self) -> usize {
        self.len()
    }

    #[inline]
    fn merge(&mut self, src: Bytes) -> Result<(), DecodeError> {
        if let Ok(s) = ByteString::try_from(src) {
            *self = s.as_str().to_string();
            Ok(())
        } else {
            Err(DecodeError::new(
                "invalid string value: data is not UTF-8 encoded",
            ))
        }
    }

    #[inline]
    fn encode(&self, dst: &mut BytesMut) {
        dst.extend_from_slice(self.as_bytes());
    }

    #[inline]
    fn is_default(&self, default: &[u8]) -> bool {
        self.as_bytes() == default
    }
}

impl NativeType for ByteString {
    const TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    fn value_len(&self) -> usize {
        self.as_slice().len()
    }

    #[inline]
    fn merge(&mut self, src: Bytes) -> Result<(), DecodeError> {
        if let Ok(s) = ByteString::try_from(src) {
            *self = s;
            Ok(())
        } else {
            Err(DecodeError::new(
                "invalid string value: data is not UTF-8 encoded",
            ))
        }
    }

    #[inline]
    fn encode(&self, dst: &mut BytesMut) {
        dst.extend_from_slice(self.as_bytes());
    }

    #[inline]
    fn is_default(&self, default: &[u8]) -> bool {
        self.as_bytes() == default
    }
}

impl<T: NativeType> NativeType for Option<T> {
    const TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    fn value_len(&self) -> usize {
        panic!("Value length is not known for Vec<T>");
    }

    #[inline]
    /// Serialize field value
    fn encode(&self, _: &mut BytesMut) {}

    #[inline]
    /// Deserialize from the input
    fn merge(&mut self, _: Bytes) -> Result<(), DecodeError> {
        Err(DecodeError::new(
            "Cannot directly call deserialize for Vec<T>",
        ))
    }

    /// Deserialize protobuf field
    fn deserialize(&mut self, wtype: WireType, src: &mut Bytes) -> Result<(), DecodeError> {
        let mut value: T = Default::default();
        value.deserialize(wtype, src)?;
        *self = Some(value);
        Ok(())
    }

    /// Serialize protobuf field
    fn serialize(&self, tag: u32, dst: &mut BytesMut) {
        if let Some(ref value) = self {
            value.serialize(tag, dst);
        }
    }

    #[inline]
    fn is_default(&self, _: &[u8]) -> bool {
        false
    }

    /// Protobuf field length
    fn field_len(&self, tag: u32) -> usize {
        self.as_ref().map(|value| value.field_len(tag)).unwrap_or(0)
    }
}

impl NativeType for Vec<u8> {
    const TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    fn value_len(&self) -> usize {
        self.len()
    }

    #[inline]
    /// Serialize field value
    fn encode(&self, dst: &mut BytesMut) {
        dst.extend_from_slice(self.as_slice());
    }

    #[inline]
    /// Deserialize from the input
    fn merge(&mut self, src: Bytes) -> Result<(), DecodeError> {
        *self = Vec::from(&src[..]);
        Ok(())
    }

    #[inline]
    fn is_default(&self, default: &[u8]) -> bool {
        self == default
    }
}

impl<T: NativeType> NativeType for Vec<T> {
    const TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    fn value_len(&self) -> usize {
        panic!("Value length is not known for Vec<T>");
    }

    #[inline]
    /// Serialize field value
    fn encode(&self, _: &mut BytesMut) {}

    #[inline]
    /// Deserialize from the input
    fn merge(&mut self, _: Bytes) -> Result<(), DecodeError> {
        Err(DecodeError::new(
            "Cannot directly call deserialize for Vec<T>",
        ))
    }

    /// Deserialize protobuf field
    fn deserialize(&mut self, wtype: WireType, src: &mut Bytes) -> Result<(), DecodeError> {
        let mut value: T = Default::default();
        value.deserialize(wtype, src)?;
        self.push(value);
        Ok(())
    }

    /// Serialize protobuf field
    fn serialize(&self, tag: u32, dst: &mut BytesMut) {
        for item in self.iter() {
            item.serialize(tag, dst);
        }
    }

    #[inline]
    fn is_default(&self, _: &[u8]) -> bool {
        false
    }

    /// Protobuf field length
    fn field_len(&self, tag: u32) -> usize {
        self.iter().map(|value| value.field_len(tag)).sum()
    }
}

impl<K: NativeType + Eq + Hash, V: NativeType + Eq, S: BuildHasher + Default> NativeType
    for HashMap<K, V, S>
{
    const TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    fn value_len(&self) -> usize {
        panic!("Value length is not known for Map<K, V>");
    }

    #[inline]
    /// Deserialize from the input
    fn merge(&mut self, _: Bytes) -> Result<(), DecodeError> {
        Err(DecodeError::new(
            "Cannot directly call deserialize for Map<K, V>",
        ))
    }

    #[inline]
    /// Serialize field value
    fn encode(&self, _: &mut BytesMut) {}

    #[inline]
    fn is_default(&self, _: &[u8]) -> bool {
        false
    }

    /// Deserialize protobuf field
    fn deserialize(&mut self, wtype: WireType, src: &mut Bytes) -> Result<(), DecodeError> {
        encoding::check_wire_type(Self::TYPE, wtype)?;
        let len = encoding::decode_varint(src)? as usize;
        if len > src.len() {
            Err(DecodeError::new("Not enough data"))
        } else {
            let mut buf = src.split_to(len);
            let mut key = Default::default();
            let mut val = Default::default();

            while !buf.is_empty() {
                let (tag, wire_type) = encoding::decode_key(&mut buf)?;
                match tag {
                    1 => NativeType::deserialize(&mut key, wire_type, &mut buf)?,
                    2 => NativeType::deserialize(&mut val, wire_type, &mut buf)?,
                    _ => return Err(DecodeError::new("Map deserialization error")),
                }
            }
            self.insert(key, val);
            Ok(())
        }
    }

    /// Serialize protobuf field
    fn serialize(&self, tag: u32, dst: &mut BytesMut) {
        let key_default = K::default();
        let val_default = V::default();

        for item in self.iter() {
            let skip_key = item.0 == &key_default;
            let skip_val = item.1 == &val_default;

            let len = (if skip_key { 0 } else { item.0.field_len(1) })
                + (if skip_val { 0 } else { item.1.field_len(2) });

            encoding::encode_key(tag, WireType::LengthDelimited, dst);
            encoding::encode_varint(len as u64, dst);
            if !skip_key {
                item.0.serialize(1, dst);
            }
            if !skip_val {
                item.1.serialize(1, dst);
            }
        }
    }

    /// Generic protobuf map encode function with an overridden value default.
    fn field_len(&self, tag: u32) -> usize {
        let key_default = K::default();
        let val_default = V::default();

        self.iter()
            .map(|(key, val)| {
                let len = (if key == &key_default {
                    0
                } else {
                    key.field_len(1)
                }) + (if val == &val_default {
                    0
                } else {
                    val.field_len(2)
                });

                encoding::key_len(tag) + encoding::encoded_len_varint(len as u64) + len
            })
            .sum::<usize>()
    }
}

/// Macro which emits a module containing a set of encoding functions for a
/// variable width numeric type.
macro_rules! varint {
    ($ty:ident) => (
        varint!($ty, to_uint64(self) { *self as u64 }, from_uint64(v) { v as $ty });
    );

    ($ty:ty, to_uint64($slf:ident) $to_uint64:expr, from_uint64($val:ident) $from_uint64:expr) => (

        impl NativeType for $ty {
            const TYPE: WireType = WireType::Varint;

            #[inline]
            fn value_len(&self) -> usize {
                0
            }

            #[inline]
            fn encode(&$slf, dst: &mut BytesMut) {
                encoding::encode_varint($to_uint64, dst);
            }

            #[inline]
            fn merge(&mut self, mut src: Bytes) -> Result<(), DecodeError> {
                *self = encoding::decode_varint(&mut src).map(|$val| $from_uint64)?;
                Ok(())
            }

            #[inline]
            fn field_len(&$slf, tag: u32) -> usize {
                encoding::key_len(tag) + encoding::encoded_len_varint($to_uint64)
            }

            /// Serialize protobuf field
            fn serialize(&self, tag: u32, dst: &mut BytesMut) {
                encoding::encode_key(tag, Self::TYPE, dst);
                self.encode(dst);
            }

            /// Deserialize protobuf field
            fn deserialize(&mut self, wtype: WireType, src: &mut Bytes) -> Result<(), DecodeError> {
                encoding::check_wire_type(Self::TYPE, wtype)?;
                *self = encoding::decode_varint(src).map(|$val| $from_uint64)?;
                Ok(())
            }
        }
    );
}

varint!(bool,
        to_uint64(self) if *self { 1u64 } else { 0u64 },
        from_uint64(value) value != 0);
varint!(i32);
varint!(i64);
varint!(u32);
varint!(u64);
// varint!(i32, sint32,
// to_uint64(value) {
//     ((value << 1) ^ (value >> 31)) as u32 as u64
// },
// from_uint64(value) {
//     let value = value as u32;
//     ((value >> 1) as i32) ^ (-((value & 1) as i32))
// });
// varint!(i64, sint64,
// to_uint64(value) {
//     ((value << 1) ^ (value >> 63)) as u64
// },
// from_uint64(value) {
//     ((value >> 1) as i64) ^ (-((value & 1) as i64))
// });
