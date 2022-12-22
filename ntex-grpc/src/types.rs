use std::{collections::HashMap, convert::TryFrom, fmt, hash::BuildHasher, hash::Hash, mem};

use ntex_bytes::{ByteString, Bytes, BytesMut};

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

/// Default type value
pub enum DefaultValue<T> {
    Unknown,
    Default,
    Value(T),
}

/// Protobuf type serializer
pub trait NativeType: PartialEq + Default + Sized + fmt::Debug {
    const TYPE: WireType;

    #[inline]
    /// Returns the encoded length of the message without a length delimiter.
    fn value_len(&self) -> usize {
        0
    }

    /// Deserialize from the input
    fn merge(&mut self, src: &mut Bytes) -> Result<(), DecodeError>;

    /// Check if value is default
    fn is_default(&self) -> bool {
        false
    }

    /// Encode field value
    fn encode_value(&self, dst: &mut BytesMut);

    #[inline]
    /// Encode field tag and length
    fn encode_type(&self, tag: u32, dst: &mut BytesMut) {
        encoding::encode_key(tag, Self::TYPE, dst);
        if Self::TYPE != WireType::Varint {
            encoding::encode_varint(self.value_len() as u64, dst);
        }
    }

    #[inline]
    /// Protobuf field length
    fn encoded_len(&self, tag: u32) -> usize {
        encoding::key_len(tag)
            + encoding::encoded_len_varint(self.value_len() as u64)
            + self.value_len()
    }

    #[inline]
    /// Serialize protobuf field
    fn serialize(&self, tag: u32, default: DefaultValue<&Self>, dst: &mut BytesMut) {
        let default = match default {
            DefaultValue::Unknown => false,
            DefaultValue::Default => self.is_default(),
            DefaultValue::Value(d) => self == d,
        };

        if !default {
            self.encode_type(tag, dst);
            self.encode_value(dst);
        }
    }

    #[inline]
    /// Protobuf field length
    fn serialized_len(&self, tag: u32, default: DefaultValue<&Self>) -> usize {
        let default = match default {
            DefaultValue::Unknown => false,
            DefaultValue::Default => self.is_default(),
            DefaultValue::Value(d) => self == d,
        };

        if default {
            0
        } else {
            self.encoded_len(tag)
        }
    }

    #[inline]
    /// Deserialize protobuf field
    fn deserialize(
        &mut self,
        _: u32,
        wtype: WireType,
        src: &mut Bytes,
    ) -> Result<(), DecodeError> {
        encoding::check_wire_type(Self::TYPE, wtype)?;
        if Self::TYPE == WireType::Varint {
            self.merge(src)
        } else {
            let len = encoding::decode_varint(src)? as usize;
            if len > src.len() {
                Err(DecodeError::new(format!(
                    "Not enough data, message size {} buffer size {}",
                    len,
                    src.len()
                )))
            } else {
                let mut buf = src.split_to(len);
                self.merge(&mut buf)
            }
        }
    }

    #[inline]
    /// Deserialize protobuf field to default value
    fn deserialize_default(
        tag: u32,
        wtype: WireType,
        src: &mut Bytes,
    ) -> Result<Self, DecodeError> {
        let mut value = Self::default();
        value.deserialize(tag, wtype, src)?;
        Ok(value)
    }
}

/// Protobuf struct read/write operations
impl Message for () {
    fn read(_: &mut Bytes) -> Result<Self, DecodeError> {
        Ok(())
    }
    fn write(&self, _: &mut BytesMut) {}
    fn encoded_len(&self) -> usize {
        0
    }
}

impl<T: Message + PartialEq> NativeType for T {
    const TYPE: WireType = WireType::LengthDelimited;

    fn value_len(&self) -> usize {
        Message::encoded_len(self)
    }

    #[inline]
    /// Encode message to the buffer
    fn encode_value(&self, dst: &mut BytesMut) {
        self.write(dst)
    }

    /// Deserialize from the input
    fn merge(&mut self, src: &mut Bytes) -> Result<(), DecodeError> {
        *self = Message::read(src)?;
        Ok(())
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
    fn encode_value(&self, dst: &mut BytesMut) {
        dst.extend_from_slice(self);
    }

    #[inline]
    /// Deserialize from the input
    fn merge(&mut self, src: &mut Bytes) -> Result<(), DecodeError> {
        *self = mem::take(src);
        Ok(())
    }

    #[inline]
    fn is_default(&self) -> bool {
        self.is_empty()
    }
}

impl NativeType for String {
    const TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    fn value_len(&self) -> usize {
        self.len()
    }

    #[inline]
    fn merge(&mut self, src: &mut Bytes) -> Result<(), DecodeError> {
        if let Ok(s) = ByteString::try_from(mem::take(src)) {
            *self = s.as_str().to_string();
            Ok(())
        } else {
            Err(DecodeError::new(
                "invalid string value: data is not UTF-8 encoded",
            ))
        }
    }

    #[inline]
    fn encode_value(&self, dst: &mut BytesMut) {
        dst.extend_from_slice(self.as_bytes());
    }

    #[inline]
    fn is_default(&self) -> bool {
        self.is_empty()
    }
}

impl NativeType for ByteString {
    const TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    fn value_len(&self) -> usize {
        self.as_slice().len()
    }

    #[inline]
    fn merge(&mut self, src: &mut Bytes) -> Result<(), DecodeError> {
        if let Ok(s) = ByteString::try_from(mem::take(src)) {
            *self = s;
            Ok(())
        } else {
            Err(DecodeError::new(
                "invalid string value: data is not UTF-8 encoded",
            ))
        }
    }

    #[inline]
    fn encode_value(&self, dst: &mut BytesMut) {
        dst.extend_from_slice(self.as_bytes());
    }

    #[inline]
    fn is_default(&self) -> bool {
        self.is_empty()
    }
}

impl<T: NativeType> NativeType for Option<T> {
    const TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    fn is_default(&self) -> bool {
        self.is_none()
    }

    #[inline]
    /// Serialize field value
    fn encode_value(&self, _: &mut BytesMut) {}

    #[inline]
    /// Deserialize from the input
    fn merge(&mut self, _: &mut Bytes) -> Result<(), DecodeError> {
        Err(DecodeError::new(
            "Cannot directly call deserialize for Vec<T>",
        ))
    }

    #[inline]
    /// Deserialize protobuf field
    fn deserialize(
        &mut self,
        tag: u32,
        wtype: WireType,
        src: &mut Bytes,
    ) -> Result<(), DecodeError> {
        let mut value: T = Default::default();
        value.deserialize(tag, wtype, src)?;
        *self = Some(value);
        Ok(())
    }

    #[inline]
    /// Serialize protobuf field
    fn serialize(&self, tag: u32, _: DefaultValue<&Self>, dst: &mut BytesMut) {
        if let Some(ref value) = self {
            value.serialize(tag, DefaultValue::Unknown, dst);
        }
    }

    #[inline]
    /// Protobuf field length
    fn serialized_len(&self, tag: u32, _: DefaultValue<&Self>) -> usize {
        if let Some(ref value) = self {
            value.serialized_len(tag, DefaultValue::Unknown)
        } else {
            0
        }
    }

    #[inline]
    /// Protobuf field length
    fn encoded_len(&self, tag: u32) -> usize {
        self.as_ref()
            .map(|value| value.encoded_len(tag))
            .unwrap_or(0)
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
    fn encode_value(&self, dst: &mut BytesMut) {
        dst.extend_from_slice(self.as_slice());
    }

    #[inline]
    /// Deserialize from the input
    fn merge(&mut self, src: &mut Bytes) -> Result<(), DecodeError> {
        *self = Vec::from(&src[..]);
        Ok(())
    }

    #[inline]
    fn is_default(&self) -> bool {
        self.is_empty()
    }
}

impl<T: NativeType> NativeType for Vec<T> {
    const TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    /// Serialize field value
    fn encode_value(&self, _: &mut BytesMut) {}

    #[inline]
    /// Deserialize from the input
    fn merge(&mut self, _: &mut Bytes) -> Result<(), DecodeError> {
        Err(DecodeError::new("Cannot directly call merge for Vec<T>"))
    }

    /// Deserialize protobuf field
    fn deserialize(
        &mut self,
        tag: u32,
        wtype: WireType,
        src: &mut Bytes,
    ) -> Result<(), DecodeError> {
        let mut value: T = Default::default();
        value.deserialize(tag, wtype, src)?;
        self.push(value);
        Ok(())
    }

    /// Serialize protobuf field
    fn serialize(&self, tag: u32, _: DefaultValue<&Self>, dst: &mut BytesMut) {
        for item in self.iter() {
            item.serialize(tag, DefaultValue::Default, dst);
        }
    }

    #[inline]
    fn is_default(&self) -> bool {
        self.is_empty()
    }

    /// Protobuf field length
    fn encoded_len(&self, tag: u32) -> usize {
        self.iter().map(|value| value.encoded_len(tag)).sum()
    }
}

impl<K: NativeType + Eq + Hash, V: NativeType, S: BuildHasher + Default> NativeType
    for HashMap<K, V, S>
{
    const TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    /// Deserialize from the input
    fn merge(&mut self, _: &mut Bytes) -> Result<(), DecodeError> {
        Err(DecodeError::new("Cannot directly call merge for Map<K, V>"))
    }

    #[inline]
    /// Serialize field value
    fn encode_value(&self, _: &mut BytesMut) {}

    #[inline]
    fn is_default(&self) -> bool {
        self.is_empty()
    }

    /// Deserialize protobuf field
    fn deserialize(
        &mut self,
        _: u32,
        wtype: WireType,
        src: &mut Bytes,
    ) -> Result<(), DecodeError> {
        encoding::check_wire_type(Self::TYPE, wtype)?;
        let len = encoding::decode_varint(src)? as usize;
        if len > src.len() {
            Err(DecodeError::new(format!(
                "Not enough data for HashMap, message size {}, buf size {}",
                len,
                src.len()
            )))
        } else {
            let mut buf = src.split_to(len);
            let mut key = Default::default();
            let mut val = Default::default();

            while !buf.is_empty() {
                let (tag, wire_type) = encoding::decode_key(&mut buf)?;
                match tag {
                    1 => NativeType::deserialize(&mut key, 1, wire_type, &mut buf)?,
                    2 => NativeType::deserialize(&mut val, 2, wire_type, &mut buf)?,
                    _ => return Err(DecodeError::new("Map deserialization error")),
                }
            }
            self.insert(key, val);
            Ok(())
        }
    }

    /// Serialize protobuf field
    fn serialize(&self, tag: u32, _: DefaultValue<&Self>, dst: &mut BytesMut) {
        for item in self.iter() {
            let skip_key = item.0.is_default();
            let skip_val = item.1.is_default();

            let len = (if skip_key { 0 } else { item.0.encoded_len(1) })
                + (if skip_val { 0 } else { item.1.encoded_len(2) });

            encoding::encode_key(tag, WireType::LengthDelimited, dst);
            encoding::encode_varint(len as u64, dst);
            if !skip_key {
                item.0.serialize(1, DefaultValue::Default, dst);
            }
            if !skip_val {
                item.1.serialize(2, DefaultValue::Default, dst);
            }
        }
    }

    /// Generic protobuf map encode function with an overridden value default.
    fn encoded_len(&self, tag: u32) -> usize {
        let key_default = K::default();
        let val_default = V::default();

        self.iter()
            .map(|(key, val)| {
                let len = (if key == &key_default {
                    0
                } else {
                    key.encoded_len(1)
                }) + (if val == &val_default {
                    0
                } else {
                    val.encoded_len(2)
                });

                encoding::key_len(tag) + encoding::encoded_len_varint(len as u64) + len
            })
            .sum::<usize>()
    }
}

/// Macro which emits a module containing a set of encoding functions for a
/// variable width numeric type.
macro_rules! varint {
    ($ty:ident, $default:expr) => (
        varint!($ty, $default, to_uint64(self) { *self as u64 }, from_uint64(v) { v as $ty });
    );

    ($ty:ty, $default:expr, to_uint64($slf:ident) $to_uint64:expr, from_uint64($val:ident) $from_uint64:expr) => (

        impl NativeType for $ty {
            const TYPE: WireType = WireType::Varint;

            #[inline]
            fn is_default(&self) -> bool {
                *self == $default
            }

            #[inline]
            fn encode_value(&$slf, dst: &mut BytesMut) {
                encoding::encode_varint($to_uint64, dst);
            }

            #[inline]
            fn encoded_len(&$slf, tag: u32) -> usize {
                encoding::key_len(tag) + encoding::encoded_len_varint($to_uint64)
            }

            #[inline]
            fn merge(&mut self, src: &mut Bytes) -> Result<(), DecodeError> {
                *self = encoding::decode_varint(src).map(|$val| $from_uint64)?;
                Ok(())
            }
        }
    );
}

varint!(bool, false,
        to_uint64(self) u64::from(*self),
        from_uint64(value) value != 0);
varint!(i32, 0i32);
varint!(i64, 0i64);
varint!(u32, 0u32);
varint!(u64, 0u64);
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
