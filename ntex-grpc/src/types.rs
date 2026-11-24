#![allow(
    clippy::cast_lossless,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
use std::hash::{BuildHasher, Hash};
use std::sync::Arc;
use std::{cell::Cell, collections::HashMap, convert::TryFrom, fmt, mem};

use ntex_bytes::{Buf, BufMut, BytePages, ByteString, Bytes};
use ntex_util::hash_map::HashMap as HashMapBase;

pub use crate::encoding::WireType;
use crate::encoding::{self, DecodeError};
pub use crate::format::{FieldFormat, Fixed, Group, Map, MapType, Native, Unpacked, ZigZag};

/// Protobuf struct read/write operations
///
/// Generated implementations skip unknown fields while decoding, they are
/// not preserved and are not written back by `write()`.
pub trait Message: Default + Sized + fmt::Debug {
    #[doc(hidden)]
    /// The message has message fields and records their lengths with
    /// [`encoded_len_with`](Self::encoded_len_with). Leaf messages are
    /// written without a length table.
    const NESTED: bool = false;

    /// Decodes an instance of the message from a buffer
    fn read(src: &mut Bytes) -> Result<Self, DecodeError>;

    /// Decodes a message from a buffer and merges it into `self`.
    ///
    /// Singular scalar fields are replaced, repeated fields are appended and
    /// message fields are merged recursively. The default implementation
    /// replaces `self` with the decoded message.
    fn merge_from(&mut self, src: &mut Bytes) -> Result<(), DecodeError> {
        *self = Self::read(src)?;
        Ok(())
    }

    /// Encodes and writes the message to a buffer
    fn write(&self, dst: &mut BytePages);

    /// Returns the encoded length of the message, without a length delimiter.
    ///
    /// This is the number of bytes [`write`](Self::write) produces.
    fn encoded_len(&self) -> usize;

    #[inline]
    /// Returns the encoded length of the message and records the lengths of
    /// its nested messages in `lens`, for [`write_with`](Self::write_with).
    ///
    /// The default implementation calls [`encoded_len`](Self::encoded_len).
    fn encoded_len_with(&self, lens: &mut EncodedLens) -> usize {
        let _ = lens;
        self.encoded_len()
    }

    #[inline]
    /// Encodes and writes the message with the nested message lengths
    /// recorded by the preceding [`encoded_len_with`](Self::encoded_len_with)
    /// call.
    ///
    /// The message must not change between the calls. The default
    /// implementation calls [`write`](Self::write).
    fn write_with(&self, lens: &mut EncodedLens, dst: &mut BytePages) {
        let _ = lens;
        self.write(dst);
    }
}

/// Lengths of the nested messages of a message being encoded.
///
/// [`Message::encoded_len_with`] records the length of every nested message
/// in pre-order, [`Message::write_with`] reads them back in the same order.
#[derive(Debug, Default)]
pub struct EncodedLens {
    lens: Vec<usize>,
    pos: usize,
}

impl EncodedLens {
    #[inline]
    /// Reserves the slot of the next nested message
    pub fn reserve(&mut self) -> usize {
        let slot = self.lens.len();
        if slot == self.lens.capacity() {
            self.grow();
        }
        self.lens.push(0);
        slot
    }

    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        // the table of the thread is taken on the first slot
        if self.lens.capacity() == 0 {
            self.lens = LENS.take();
        }
        self.lens.reserve(16);
    }

    #[inline]
    /// Stores the length of a reserved slot
    pub fn set(&mut self, slot: usize, len: usize) {
        self.lens[slot] = len;
    }

    #[inline]
    /// Returns the next recorded length
    pub fn next_len(&mut self) -> usize {
        let len = self.lens[self.pos];
        self.pos += 1;
        len
    }

    #[inline]
    /// Restarts reading from the first length
    pub fn rewind(&mut self) {
        self.pos = 0;
    }
}

thread_local! {
    static LENS: Cell<Vec<usize>> = const { Cell::new(Vec::new()) };
}

#[inline]
/// Calls `f` with an empty length table, reusing the table of the thread
pub fn with_encoded_lens<R>(f: impl FnOnce(&mut EncodedLens) -> R) -> R {
    let mut lens = EncodedLens::default();
    let res = f(&mut lens);
    if lens.lens.capacity() != 0 {
        lens.lens.clear();
        LENS.set(lens.lens);
    }
    res
}

#[inline]
/// Writes a message with nested messages, generated `write` calls it
pub fn write_nested<T: Message>(msg: &T, dst: &mut BytePages) {
    with_encoded_lens(|lens| {
        msg.encoded_len_with(lens);
        lens.rewind();
        msg.write_with(lens, dst);
    });
}

/// How a field decides it holds its default value.
///
/// Protobuf leaves fields with default values out of the encoded message.
pub enum DefaultValue<T> {
    /// Never treat the value as default, always write it. Used for oneof
    /// fields, where the chosen variant must be sent even when it is empty.
    Unknown,
    /// Skip the field when [`NativeType::is_default()`] says so.
    Default,
    /// Skip the field when it equals this value.
    Value(T),
}

impl<T: NativeType> DefaultValue<&T> {
    /// Returns true if `value` holds the default and is left out
    #[inline]
    fn skips(self, value: &T) -> bool {
        match self {
            DefaultValue::Unknown => false,
            DefaultValue::Default => value.is_default(),
            DefaultValue::Value(d) => value == d,
        }
    }
}

/// Protobuf type serializer
pub trait NativeType: PartialEq + Default + Sized + fmt::Debug {
    /// Wire type the value is encoded with.
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
    fn encode_value(&self, dst: &mut BytePages);

    #[inline]
    /// Encode field tag and length
    fn encode_type(&self, tag: u32, dst: &mut BytePages) {
        encoding::encode_key(tag, Self::TYPE, dst);
        if Self::TYPE == WireType::LengthDelimited {
            encoding::encode_varint(self.value_len() as u64, dst);
        }
    }

    #[inline]
    /// Protobuf field length
    fn encoded_len(&self, tag: u32) -> usize {
        let value_len = self.value_len();
        encoding::key_len(tag) + encoding::encoded_len_varint(value_len as u64) + value_len
    }

    #[inline]
    /// Serialize protobuf field
    fn serialize(&self, tag: u32, default: DefaultValue<&Self>, dst: &mut BytePages) {
        if !default.skips(self) {
            self.encode_type(tag, dst);
            self.encode_value(dst);
        }
    }

    #[inline]
    /// Protobuf field length
    fn serialized_len(&self, tag: u32, default: DefaultValue<&Self>) -> usize {
        if default.skips(self) {
            0
        } else {
            self.encoded_len(tag)
        }
    }

    #[inline]
    /// Serialize protobuf field with the nested message lengths recorded by
    /// the preceding [`serialized_len_with`](Self::serialized_len_with) call.
    fn serialize_with(
        &self,
        tag: u32,
        default: DefaultValue<&Self>,
        lens: &mut EncodedLens,
        dst: &mut BytePages,
    ) {
        let _ = lens;
        self.serialize(tag, default, dst);
    }

    #[inline]
    /// Protobuf field length, records the lengths of nested messages
    fn serialized_len_with(
        &self,
        tag: u32,
        default: DefaultValue<&Self>,
        lens: &mut EncodedLens,
    ) -> usize {
        let _ = lens;
        self.serialized_len(tag, default)
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

        if Self::TYPE == WireType::LengthDelimited {
            let len = encoding::decode_varint(src)? as usize;
            let mut buf = src.split_to_checked(len).ok_or_else(|| {
                DecodeError::new(format!(
                    "Not enough data, message size {} buffer size {}",
                    len,
                    src.len()
                ))
            })?;
            self.merge(&mut buf)
        } else {
            self.merge(src)
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
    fn encoded_len(&self) -> usize {
        0
    }

    fn read(_: &mut Bytes) -> Result<Self, DecodeError> {
        Ok(())
    }

    fn write(&self, _: &mut BytePages) {}
}

/// Boxed message, generated code uses it for recursive message fields
impl<T: Message> Message for Box<T> {
    const NESTED: bool = T::NESTED;

    #[inline]
    fn read(src: &mut Bytes) -> Result<Self, DecodeError> {
        T::read(src).map(Box::new)
    }

    #[inline]
    fn merge_from(&mut self, src: &mut Bytes) -> Result<(), DecodeError> {
        T::merge_from(self, src)
    }

    #[inline]
    fn write(&self, dst: &mut BytePages) {
        T::write(self, dst);
    }

    #[inline]
    fn encoded_len(&self) -> usize {
        T::encoded_len(self)
    }

    #[inline]
    fn encoded_len_with(&self, lens: &mut EncodedLens) -> usize {
        T::encoded_len_with(self, lens)
    }

    #[inline]
    fn write_with(&self, lens: &mut EncodedLens, dst: &mut BytePages) {
        T::write_with(self, lens, dst);
    }
}

impl<T: Message + PartialEq> NativeType for T {
    const TYPE: WireType = WireType::LengthDelimited;

    fn value_len(&self) -> usize {
        Message::encoded_len(self)
    }

    #[inline]
    /// Encode message to the buffer
    fn encode_value(&self, dst: &mut BytePages) {
        self.write(dst);
    }

    /// Deserialize from the input
    fn merge(&mut self, src: &mut Bytes) -> Result<(), DecodeError> {
        encoding::merge_nested(|| Message::merge_from(self, src))
    }

    #[inline]
    fn serialize_with(
        &self,
        tag: u32,
        default: DefaultValue<&Self>,
        lens: &mut EncodedLens,
        dst: &mut BytePages,
    ) {
        if !T::NESTED {
            self.serialize(tag, default, dst);
        } else if !default.skips(self) {
            encoding::encode_key(tag, WireType::LengthDelimited, dst);
            encoding::encode_varint(lens.next_len() as u64, dst);
            self.write_with(lens, dst);
        }
    }

    #[inline]
    fn serialized_len_with(
        &self,
        tag: u32,
        default: DefaultValue<&Self>,
        lens: &mut EncodedLens,
    ) -> usize {
        if !T::NESTED {
            self.serialized_len(tag, default)
        } else if default.skips(self) {
            0
        } else {
            let slot = lens.reserve();
            let len = self.encoded_len_with(lens);
            lens.set(slot, len);
            encoding::key_len(tag) + encoding::encoded_len_varint(len as u64) + len
        }
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
    fn encode_value(&self, dst: &mut BytePages) {
        dst.append(self.clone());
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
    fn encode_value(&self, dst: &mut BytePages) {
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
    fn encode_value(&self, dst: &mut BytePages) {
        dst.append(self.clone());
    }

    #[inline]
    fn is_default(&self) -> bool {
        self.is_empty()
    }
}

impl NativeType for Arc<str> {
    const TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    fn value_len(&self) -> usize {
        self.len()
    }

    #[inline]
    fn merge(&mut self, src: &mut Bytes) -> Result<(), DecodeError> {
        if let Ok(s) = ByteString::try_from(mem::take(src)) {
            *self = Arc::from(s.as_str());
            Ok(())
        } else {
            Err(DecodeError::new(
                "invalid string value: data is not UTF-8 encoded",
            ))
        }
    }

    #[inline]
    fn encode_value(&self, dst: &mut BytePages) {
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
    fn encode_value(&self, _: &mut BytePages) {}

    #[inline]
    /// Deserialize from the input
    fn merge(&mut self, _: &mut Bytes) -> Result<(), DecodeError> {
        Err(DecodeError::new(
            "Cannot directly call deserialize for Option<T>",
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
        if let Some(value) = self {
            value.deserialize(tag, wtype, src)
        } else {
            let mut value: T = Default::default();
            value.deserialize(tag, wtype, src)?;
            *self = Some(value);
            Ok(())
        }
    }

    #[inline]
    /// Serialize protobuf field
    fn serialize(&self, tag: u32, _: DefaultValue<&Self>, dst: &mut BytePages) {
        if let Some(value) = self {
            value.serialize(tag, DefaultValue::Unknown, dst);
        }
    }

    #[inline]
    fn serialize_with(
        &self,
        tag: u32,
        _: DefaultValue<&Self>,
        lens: &mut EncodedLens,
        dst: &mut BytePages,
    ) {
        if let Some(value) = self {
            value.serialize_with(tag, DefaultValue::Unknown, lens, dst);
        }
    }

    #[inline]
    fn serialized_len_with(
        &self,
        tag: u32,
        _: DefaultValue<&Self>,
        lens: &mut EncodedLens,
    ) -> usize {
        if let Some(value) = self {
            value.serialized_len_with(tag, DefaultValue::Unknown, lens)
        } else {
            0
        }
    }

    #[inline]
    /// Protobuf field length
    fn serialized_len(&self, tag: u32, _: DefaultValue<&Self>) -> usize {
        if let Some(value) = self {
            value.serialized_len(tag, DefaultValue::Unknown)
        } else {
            0
        }
    }

    #[inline]
    /// Protobuf field length
    fn encoded_len(&self, tag: u32) -> usize {
        self.as_ref().map_or(0, |value| value.encoded_len(tag))
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
    fn encode_value(&self, dst: &mut BytePages) {
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
    fn encode_value(&self, _: &mut BytePages) {}

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
        // packed and unpacked scalar fields must both be accepted
        if T::TYPE != WireType::LengthDelimited && wtype == WireType::LengthDelimited {
            let len = encoding::decode_varint(src)? as usize;
            let mut buf = src
                .split_to_checked(len)
                .ok_or_else(DecodeError::incomplete)?;
            while !buf.is_empty() {
                let mut value: T = Default::default();
                value.merge(&mut buf)?;
                self.push(value);
            }
        } else {
            let mut value: T = Default::default();
            value.deserialize(tag, wtype, src)?;
            self.push(value);
        }
        Ok(())
    }

    /// Serialize protobuf field
    fn serialize(&self, tag: u32, _: DefaultValue<&Self>, dst: &mut BytePages) {
        if self.is_empty() {
            return;
        }
        if T::TYPE == WireType::LengthDelimited {
            for item in self {
                item.serialize(tag, DefaultValue::Unknown, dst);
            }
        } else {
            encoding::encode_key(tag, WireType::LengthDelimited, dst);
            encoding::encode_varint(
                self.iter().map(NativeType::value_len).sum::<usize>() as u64,
                dst,
            );
            for item in self {
                item.encode_value(dst);
            }
        }
    }

    fn serialize_with(
        &self,
        tag: u32,
        default: DefaultValue<&Self>,
        lens: &mut EncodedLens,
        dst: &mut BytePages,
    ) {
        if T::TYPE == WireType::LengthDelimited {
            for item in self {
                item.serialize_with(tag, DefaultValue::Unknown, lens, dst);
            }
        } else {
            self.serialize(tag, default, dst);
        }
    }

    fn serialized_len_with(
        &self,
        tag: u32,
        default: DefaultValue<&Self>,
        lens: &mut EncodedLens,
    ) -> usize {
        if T::TYPE == WireType::LengthDelimited {
            self.iter()
                .map(|item| item.serialized_len_with(tag, DefaultValue::Unknown, lens))
                .sum()
        } else {
            self.serialized_len(tag, default)
        }
    }

    #[inline]
    fn is_default(&self) -> bool {
        self.is_empty()
    }

    /// Protobuf field length
    fn encoded_len(&self, tag: u32) -> usize {
        if T::TYPE == WireType::LengthDelimited {
            self.iter().map(|value| value.encoded_len(tag)).sum()
        } else {
            let len = self.iter().map(NativeType::value_len).sum::<usize>();
            len + encoding::key_len(tag) + encoding::encoded_len_varint(len as u64)
        }
    }
}

macro_rules! hashmap {
    ($ty:ident) => {
        impl<K: NativeType + Eq + Hash, V: NativeType, S: BuildHasher + Default> NativeType
            for $ty<K, V, S>
        {
            const TYPE: WireType = WireType::LengthDelimited;

            #[inline]
            /// Deserialize from the input
            fn merge(&mut self, _: &mut Bytes) -> Result<(), DecodeError> {
                Err(DecodeError::new("Cannot directly call merge for Map<K, V>"))
            }

            #[inline]
            /// Serialize field value
            fn encode_value(&self, _: &mut BytePages) {}

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
                let mut buf = src.split_to_checked(len).ok_or_else(|| {
                    DecodeError::new(format!(
                        "Not enough data for HashMap, message size {}, buf size {}",
                        len,
                        src.len()
                    ))
                })?;
                let mut key = Default::default();
                let mut val = Default::default();

                while !buf.is_empty() {
                    let (tag, wire_type) = encoding::decode_key(&mut buf)?;
                    match tag {
                        1 => NativeType::deserialize(&mut key, 1, wire_type, &mut buf)?,
                        2 => NativeType::deserialize(&mut val, 2, wire_type, &mut buf)?,
                        _ => encoding::skip_field(wire_type, tag, &mut buf)?,
                    }
                }
                self.insert(key, val);
                Ok(())
            }

            /// Serialize protobuf field
            fn serialize(&self, tag: u32, _: DefaultValue<&Self>, dst: &mut BytePages) {
                serialize_map(self, tag, dst);
            }

            fn serialize_with(
                &self,
                tag: u32,
                _: DefaultValue<&Self>,
                lens: &mut EncodedLens,
                dst: &mut BytePages,
            ) {
                serialize_map_with(self, tag, lens, dst);
            }

            fn serialized_len_with(
                &self,
                tag: u32,
                _: DefaultValue<&Self>,
                lens: &mut EncodedLens,
            ) -> usize {
                map_len_with(self, tag, lens)
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
    };
}

hashmap!(HashMap);
hashmap!(HashMapBase);

/// Writes the map entries
fn serialize_map<'a, K, V>(
    map: impl IntoIterator<Item = (&'a K, &'a V)>,
    tag: u32,
    dst: &mut BytePages,
) where
    K: NativeType + 'a,
    V: NativeType + 'a,
{
    let key_default = K::default();
    let val_default = V::default();

    for (key, val) in map {
        let skip_key = key == &key_default;
        let skip_val = val == &val_default;

        let len = (if skip_key { 0 } else { key.encoded_len(1) })
            + (if skip_val { 0 } else { val.encoded_len(2) });

        encoding::encode_key(tag, WireType::LengthDelimited, dst);
        encoding::encode_varint(len as u64, dst);
        if !skip_key {
            key.serialize(1, DefaultValue::Default, dst);
        }
        if !skip_val {
            val.serialize(2, DefaultValue::Default, dst);
        }
    }
}

/// Map entries length, records the entry lengths and nested message lengths
fn map_len_with<'a, K, V>(
    map: impl IntoIterator<Item = (&'a K, &'a V)>,
    tag: u32,
    lens: &mut EncodedLens,
) -> usize
where
    K: NativeType + 'a,
    V: NativeType + 'a,
{
    let key_default = K::default();
    let val_default = V::default();

    let mut total = 0;
    for (key, val) in map {
        let slot = lens.reserve();
        let len = (if key == &key_default {
            0
        } else {
            key.encoded_len(1)
        }) + (if val == &val_default {
            0
        } else {
            val.serialized_len_with(2, DefaultValue::Unknown, lens)
        });
        lens.set(slot, len);
        total += encoding::key_len(tag) + encoding::encoded_len_varint(len as u64) + len;
    }
    total
}

/// Writes the map entries with the lengths recorded by `map_len_with`
fn serialize_map_with<'a, K, V>(
    map: impl IntoIterator<Item = (&'a K, &'a V)>,
    tag: u32,
    lens: &mut EncodedLens,
    dst: &mut BytePages,
) where
    K: NativeType + 'a,
    V: NativeType + 'a,
{
    let key_default = K::default();
    let val_default = V::default();

    for (key, val) in map {
        encoding::encode_key(tag, WireType::LengthDelimited, dst);
        encoding::encode_varint(lens.next_len() as u64, dst);
        if key != &key_default {
            key.serialize(1, DefaultValue::Default, dst);
        }
        if val != &val_default {
            val.serialize_with(2, DefaultValue::Default, lens, dst);
        }
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
            fn encode_value(&$slf, dst: &mut BytePages) {
                encoding::encode_varint($to_uint64, dst);
            }

            #[inline]
            fn encoded_len(&$slf, tag: u32) -> usize {
                encoding::key_len(tag) + encoding::encoded_len_varint($to_uint64)
            }

            #[inline]
            fn value_len(&$slf) -> usize {
                encoding::encoded_len_varint($to_uint64)
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

/// Macro which emits a module containing a set of encoding functions for a
/// fixed width numeric type.
macro_rules! fixed_width {
    ($ty:ty,
     $width:expr,
     $wire_type:expr,
     $default:expr,
     $put:expr,
     $get:expr) => {
        impl NativeType for $ty {
            const TYPE: WireType = $wire_type;

            #[inline]
            fn is_default(&self) -> bool {
                // -0.0 is not the default value, it must be written
                self.to_bits() == $default.to_bits()
            }

            #[inline]
            fn encode_value(&self, dst: &mut BytePages) {
                $put(dst, *self);
            }

            #[inline]
            fn encoded_len(&self, tag: u32) -> usize {
                encoding::key_len(tag) + $width
            }

            #[inline]
            fn value_len(&self) -> usize {
                $width
            }

            #[inline]
            fn merge(&mut self, src: &mut Bytes) -> Result<(), DecodeError> {
                if src.len() < $width {
                    return Err(DecodeError::new("Buffer underflow"));
                }
                *self = $get(src);
                Ok(())
            }
        }
    };
}

fixed_width!(
    f32,
    4,
    WireType::ThirtyTwoBit,
    0f32,
    BufMut::put_f32_le,
    Buf::get_f32_le
);
fixed_width!(
    f64,
    8,
    WireType::SixtyFourBit,
    0f64,
    BufMut::put_f64_le,
    Buf::get_f64_le
);

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, PartialEq, Debug, Default)]
    pub struct TestMessage {
        f: f64,
        props: HashMap<String, u32>,
        b: bool,
        opt: Option<String>,
    }

    impl Message for TestMessage {
        fn write(&self, dst: &mut BytePages) {
            NativeType::serialize(&self.f, 1, DefaultValue::Default, dst);
            NativeType::serialize(&self.props, 2, DefaultValue::Default, dst);
            NativeType::serialize(&self.b, 3, DefaultValue::Default, dst);
            NativeType::serialize(&self.opt, 4, DefaultValue::Default, dst);
        }

        #[inline]
        fn read(src: &mut Bytes) -> Result<Self, DecodeError> {
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = encoding::decode_key(src)?;
                match tag {
                    1 => NativeType::deserialize(&mut msg.f, tag, wire_type, src)?,
                    2 => NativeType::deserialize(&mut msg.props, tag, wire_type, src)?,
                    3 => NativeType::deserialize(&mut msg.b, tag, wire_type, src)?,
                    4 => NativeType::deserialize(&mut msg.opt, tag, wire_type, src)?,
                    _ => encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[allow(clippy::identity_op)]
        #[inline]
        fn encoded_len(&self) -> usize {
            0 + NativeType::serialized_len(&self.f, 1, DefaultValue::Default)
                + NativeType::serialized_len(&self.props, 2, DefaultValue::Default)
                + NativeType::serialized_len(&self.b, 3, DefaultValue::Default)
                + NativeType::serialized_len(&self.opt, 4, DefaultValue::Default)
        }
    }

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn test_hashmap_default_values() {
        let mut msg = TestMessage::default();

        msg.f = 382.8263;
        msg.b = true;
        msg.props.insert("test1".to_string(), 1);
        msg.props.insert("test2".to_string(), 0);
        msg.props.insert(String::new(), 0);

        let mut buf = BytePages::default();
        msg.write(&mut buf);
        assert_eq!(Message::encoded_len(&msg), 33);
        assert_eq!(buf.len(), 33);

        let mut buf2 = BytePages::default();
        msg.serialize(1, DefaultValue::Default, &mut buf2);
        assert_eq!(NativeType::encoded_len(&msg, 1), 35);
        assert_eq!(buf2.len(), 35);

        let msg2 = TestMessage::read(&mut buf.freeze()).unwrap();
        assert_eq!(Message::encoded_len(&msg2), 33);
        assert_eq!(msg, msg2);

        let mut buf2 = buf2.freeze();
        let mut msg3 = TestMessage::default();
        let (tag, wire_type) = encoding::decode_key(&mut buf2).unwrap();
        msg3.deserialize(tag, wire_type, &mut buf2).unwrap();
        assert_eq!(msg, msg3);
    }

    #[test]
    fn packed_varint_len() {
        let v: Vec<u32> = vec![1, 300, 70000];
        let mut buf = BytePages::default();
        v.serialize(5, DefaultValue::Default, &mut buf);
        // key + length + 1 + 2 + 3 bytes of values
        assert_eq!(NativeType::encoded_len(&v, 5), 8);
        assert_eq!(buf.len(), 8);
    }

    fn encode<T: NativeType>(value: &T, tag: u32, default: DefaultValue<&T>) -> Bytes {
        let mut buf = BytePages::default();
        value.serialize(tag, default, &mut buf);
        buf.freeze()
    }

    /// Serializes with no default, checks the bytes and the length, then
    /// reads the value back.
    fn roundtrip<T: NativeType>(value: &T, tag: u32, expected: &[u8]) {
        let buf = encode(value, tag, DefaultValue::Unknown);
        assert_eq!(buf.as_ref(), expected, "{value:?}");
        assert_eq!(
            value.serialized_len(tag, DefaultValue::Unknown),
            expected.len(),
            "{value:?}"
        );

        let mut src = buf;
        let (decoded_tag, wire_type) = encoding::decode_key(&mut src).unwrap();
        assert_eq!(decoded_tag, tag);
        assert_eq!(
            &T::deserialize_default(decoded_tag, wire_type, &mut src).unwrap(),
            value
        );
        assert!(src.is_empty(), "{value:?}");
    }

    #[test]
    fn varint_roundtrip() {
        let mut max = vec![0x08_u8];
        max.extend_from_slice(&[0xff; 9]);
        max.push(0x01);

        roundtrip(&true, 1, &[0x08, 0x01]);
        roundtrip(&false, 1, &[0x08, 0x00]);
        roundtrip(&150_i32, 2, &[0x10, 0x96, 0x01]);
        roundtrip(&-1_i32, 1, &max);
        roundtrip(&150_i64, 1, &[0x08, 0x96, 0x01]);
        roundtrip(&-1_i64, 1, &max);
        roundtrip(&u32::MAX, 1, &[0x08, 0xff, 0xff, 0xff, 0xff, 0x0f]);
        roundtrip(&u64::MAX, 1, &max);
        roundtrip(&0_u64, 16, &[0x80, 0x01, 0x00]);
    }

    #[test]
    fn bool_merges_any_non_zero_varint() {
        let mut value = false;
        value.merge(&mut Bytes::from_static(&[0x02])).unwrap();
        assert!(value);
        assert_eq!(NativeType::value_len(&true), 1);
    }

    #[test]
    fn length_delimited_roundtrip() {
        roundtrip(
            &Bytes::from_static(b"abc"),
            1,
            &[0x0a, 0x03, b'a', b'b', b'c'],
        );
        roundtrip(&"abc".to_string(), 1, &[0x0a, 0x03, b'a', b'b', b'c']);
        roundtrip(&ByteString::from("abc"), 1, &[0x0a, 0x03, b'a', b'b', b'c']);
        roundtrip(&Arc::<str>::from("abc"), 1, &[0x0a, 0x03, b'a', b'b', b'c']);
        roundtrip(&vec![1_u8, 2, 3], 1, &[0x0a, 0x03, 1, 2, 3]);
    }

    #[test]
    fn fixed_width_roundtrip() {
        // fixed width fields carry no length prefix
        roundtrip(&1.5_f64, 1, &[0x09, 0, 0, 0, 0, 0, 0, 0xf8, 0x3f]);
        roundtrip(&1.5_f32, 1, &[0x0d, 0, 0, 0xc0, 0x3f]);
        assert_eq!(NativeType::value_len(&1.5_f64), 8);
        assert_eq!(NativeType::value_len(&1.5_f32), 4);

        // only +0.0 is the default value, -0.0 is written
        assert!(encode(&0_f64, 1, DefaultValue::Default).is_empty());
        assert!(encode(&0_f32, 1, DefaultValue::Default).is_empty());
        let buf = encode(&-0_f64, 1, DefaultValue::Default);
        assert_eq!(buf.as_ref(), &[0x09, 0, 0, 0, 0, 0, 0, 0, 0x80]);
        assert_eq!((-0_f64).serialized_len(1, DefaultValue::Default), 9);
        let buf = encode(&-0_f32, 1, DefaultValue::Default);
        assert_eq!(buf.as_ref(), &[0x0d, 0, 0, 0, 0x80]);
        assert_eq!((-0_f32).serialized_len(1, DefaultValue::Default), 5);

        // repeated fixed width fields are packed
        let v = vec![1.5_f32, -1.5];
        let expected = [0x0a, 0x08, 0, 0, 0xc0, 0x3f, 0, 0, 0xc0, 0xbf];
        assert_eq!(encode(&v, 1, DefaultValue::Default).as_ref(), &expected);
        assert_eq!(v.serialized_len(1, DefaultValue::Default), expected.len());
        assert_eq!(decode_repeated::<f32>(&expected).unwrap(), v);

        // unpacked and mixed input
        let unpacked = [0x0d, 0, 0, 0xc0, 0x3f, 0x0d, 0, 0, 0xc0, 0xbf];
        assert_eq!(decode_repeated::<f32>(&unpacked).unwrap(), v);
        let mixed = [
            0x09, 0, 0, 0, 0, 0, 0, 0xf8, 0x3f, 0x0a, 0x08, 0, 0, 0, 0, 0, 0, 0xf8, 0xbf,
        ];
        assert_eq!(decode_repeated::<f64>(&mixed).unwrap(), [1.5, -1.5]);

        // wrong wire type and truncated packed value
        assert!(decode_repeated::<f32>(&[0x08, 0x01]).is_err());
        assert!(decode_repeated::<f64>(&[0x0d, 0, 0, 0, 0]).is_err());
        assert!(decode_repeated::<f32>(&[0x0a, 0x03, 0, 0, 0]).is_err());
    }

    fn decode_repeated<T: NativeType>(bytes: &[u8]) -> Result<Vec<T>, DecodeError> {
        let mut src = Bytes::copy_from_slice(bytes);
        let mut decoded = Vec::<T>::new();
        while !src.is_empty() {
            let (tag, wire_type) = encoding::decode_key(&mut src)?;
            decoded.deserialize(tag, wire_type, &mut src)?;
        }
        Ok(decoded)
    }

    #[test]
    fn repeated_varint_unpacked() {
        // unpacked and mixed input of packed fields
        assert_eq!(
            decode_repeated::<u32>(&[0x08, 0x01, 0x0a, 0x02, 0x02, 0x03, 0x08, 0x04]).unwrap(),
            [1, 2, 3, 4]
        );
        assert_eq!(
            decode_repeated::<bool>(&[0x08, 0x01, 0x08, 0x00]).unwrap(),
            [true, false]
        );
        assert!(decode_repeated::<i64>(&[0x0d, 0, 0, 0, 0]).is_err());
        assert!(decode_repeated::<i64>(&[0x08, 0xff]).is_err());
    }

    #[test]
    fn message_merge() {
        use crate::google_types::Duration;

        let mut d = Duration::read(&mut Bytes::from_static(&[0x08, 0x05])).unwrap();
        d.merge_from(&mut Bytes::from_static(&[0x10, 0x07]))
            .unwrap();
        assert_eq!(
            d,
            Duration {
                seconds: 5,
                nanos: 7
            }
        );

        // repeated singular message field is merged, scalar fields are replaced
        let mut src =
            Bytes::from_static(&[0x0a, 0x04, 0x08, 0x05, 0x10, 0x01, 0x0a, 0x02, 0x10, 0x07]);
        let mut v: Option<Duration> = None;
        while !src.is_empty() {
            let (tag, wtype) = encoding::decode_key(&mut src).unwrap();
            v.deserialize(tag, wtype, &mut src).unwrap();
        }
        assert_eq!(
            v,
            Some(Duration {
                seconds: 5,
                nanos: 7
            })
        );

        // `merge` of a message field merges as well
        let mut d = Duration {
            seconds: 3,
            nanos: 0,
        };
        NativeType::merge(&mut d, &mut Bytes::from_static(&[0x10, 0x09])).unwrap();
        assert_eq!(
            d,
            Duration {
                seconds: 3,
                nanos: 9
            }
        );
    }

    #[test]
    fn default_value_handling() {
        // `Default` skips the type default
        assert!(encode(&0_u32, 1, DefaultValue::Default).is_empty());
        assert_eq!(0_u32.serialized_len(1, DefaultValue::Default), 0);
        assert_eq!(
            encode(&1_u32, 1, DefaultValue::Default).as_ref(),
            &[0x08, 0x01]
        );
        assert_eq!(1_u32.serialized_len(1, DefaultValue::Default), 2);

        // `Value` skips an explicit default
        assert!(encode(&7_u32, 1, DefaultValue::Value(&7)).is_empty());
        assert_eq!(7_u32.serialized_len(1, DefaultValue::Value(&7)), 0);
        assert_eq!(
            encode(&8_u32, 1, DefaultValue::Value(&7)).as_ref(),
            &[0x08, 0x08]
        );
        assert_eq!(8_u32.serialized_len(1, DefaultValue::Value(&7)), 2);

        // `Unknown` always writes
        assert_eq!(
            encode(&0_u32, 1, DefaultValue::Unknown).as_ref(),
            &[0x08, 0x00]
        );
        assert_eq!(0_u32.serialized_len(1, DefaultValue::Unknown), 2);
    }

    #[test]
    fn is_default_values() {
        assert!(0_i32.is_default() && !1_i32.is_default());
        assert!(0_i64.is_default() && 0_u32.is_default() && 0_u64.is_default());
        assert!(!true.is_default() && false.is_default());
        assert!(0_f32.is_default() && !1_f32.is_default());
        assert!(0_f64.is_default() && !1_f64.is_default());
        assert!(!(-0_f32).is_default() && !(-0_f64).is_default());
        assert!(!f32::NAN.is_default() && !f64::NAN.is_default());
        assert!(String::new().is_default() && !"a".to_string().is_default());
        assert!(Bytes::new().is_default());
        assert!(ByteString::new().is_default());
        assert!(Arc::<str>::from("").is_default());
        assert!(Vec::<u8>::new().is_default());
        assert!(Vec::<u32>::new().is_default());
        assert!(Option::<u32>::None.is_default() && !Some(0_u32).is_default());
        assert!(HashMap::<String, u32>::default().is_default());
        assert!(HashMapBase::<String, u32>::default().is_default());
        // messages have no default, they are always written
        assert!(!NativeType::is_default(&TestMessage::default()));
    }

    #[test]
    fn option_field() {
        let none: Option<u32> = None;
        assert!(encode(&none, 1, DefaultValue::Default).is_empty());
        assert_eq!(none.serialized_len(1, DefaultValue::Default), 0);
        assert_eq!(NativeType::encoded_len(&none, 1), 0);
        // `Option` uses the default `value_len` and writes nothing by itself
        assert_eq!(NativeType::value_len(&none), 0);
        let mut buf = BytePages::default();
        none.encode_value(&mut buf);
        assert!(buf.is_empty());

        // `Some` of a default value is still written
        let some = Some(0_u32);
        assert_eq!(
            encode(&some, 1, DefaultValue::Default).as_ref(),
            &[0x08, 0x00]
        );
        assert_eq!(some.serialized_len(1, DefaultValue::Default), 2);
        assert_eq!(NativeType::encoded_len(&some, 1), 2);

        roundtrip(&Some(150_u32), 1, &[0x08, 0x96, 0x01]);
        roundtrip(&Some(ByteString::from("hi")), 2, &[0x12, 0x02, b'h', b'i']);
    }

    #[test]
    fn packed_varint_field() {
        let v: Vec<u32> = vec![1, 300, 70000];
        let buf = encode(&v, 5, DefaultValue::Default);
        assert_eq!(
            buf.as_ref(),
            &[0x2a, 0x06, 0x01, 0xac, 0x02, 0xf0, 0xa2, 0x04]
        );

        let mut src = buf;
        let (tag, wire_type) = encoding::decode_key(&mut src).unwrap();
        assert_eq!((tag, wire_type), (5, WireType::LengthDelimited));
        assert_eq!(
            Vec::<u32>::deserialize_default(tag, wire_type, &mut src).unwrap(),
            v
        );
        assert!(src.is_empty());

        // an empty repeated field is left out
        assert!(encode(&Vec::<u32>::new(), 5, DefaultValue::Default).is_empty());
        assert_eq!(
            Vec::<u32>::new().serialized_len(5, DefaultValue::Default),
            0
        );

        // truncated packed payload
        let mut src = Bytes::from_static(&[0x05, 0x01]);
        assert!(Vec::<u32>::deserialize_default(5, WireType::LengthDelimited, &mut src).is_err());
    }

    #[test]
    fn repeated_length_delimited_field() {
        let v = vec!["a".to_string(), "bb".to_string()];
        let buf = encode(&v, 1, DefaultValue::Default);
        assert_eq!(buf.as_ref(), &[0x0a, 0x01, b'a', 0x0a, 0x02, b'b', b'b']);
        assert_eq!(NativeType::encoded_len(&v, 1), buf.len());

        let mut src = buf;
        let mut out: Vec<String> = Vec::new();
        while !src.is_empty() {
            let (tag, wire_type) = encoding::decode_key(&mut src).unwrap();
            out.deserialize(tag, wire_type, &mut src).unwrap();
        }
        assert_eq!(out, v);
    }

    #[test]
    fn map_field() {
        let mut map: HashMap<String, u32> = HashMap::default();
        map.insert("a".to_string(), 1);

        let buf = encode(&map, 1, DefaultValue::Default);
        assert_eq!(buf.as_ref(), &[0x0a, 0x05, 0x0a, 0x01, b'a', 0x10, 0x01]);
        assert_eq!(NativeType::encoded_len(&map, 1), buf.len());

        let mut src = buf;
        let (tag, wire_type) = encoding::decode_key(&mut src).unwrap();
        let out = HashMap::<String, u32>::deserialize_default(tag, wire_type, &mut src).unwrap();
        assert_eq!(out, map);
        assert!(src.is_empty());

        // the ntex hash map behaves the same
        let mut map: HashMapBase<String, u32> = HashMapBase::default();
        map.insert("a".to_string(), 1);
        let buf = encode(&map, 1, DefaultValue::Default);
        assert_eq!(buf.as_ref(), &[0x0a, 0x05, 0x0a, 0x01, b'a', 0x10, 0x01]);
        assert_eq!(NativeType::encoded_len(&map, 1), buf.len());

        let mut src = buf;
        let (tag, wire_type) = encoding::decode_key(&mut src).unwrap();
        let out =
            HashMapBase::<String, u32>::deserialize_default(tag, wire_type, &mut src).unwrap();
        assert_eq!(out, map);
    }

    #[test]
    fn map_field_errors() {
        type Map = HashMap<String, u32>;

        // maps are length delimited
        let mut src = Bytes::from_static(&[0x01]);
        let err = Map::deserialize_default(1, WireType::Varint, &mut src).unwrap_err();
        assert!(err.to_string().contains("invalid wire type"));

        // entry longer than the buffer
        let mut src = Bytes::from_static(&[0x05, 0x01]);
        let err = Map::deserialize_default(1, WireType::LengthDelimited, &mut src).unwrap_err();
        assert!(err.to_string().contains("Not enough data for HashMap"));

        // malformed unknown field inside an entry
        let mut src = Bytes::from_static(&[0x01, 0x1c]);
        let err = Map::deserialize_default(1, WireType::LengthDelimited, &mut src).unwrap_err();
        assert!(err.to_string().contains("unexpected end group tag"));
    }

    #[test]
    fn map_entry_unknown_field() {
        type Map = HashMap<String, u32>;

        // unknown fields inside an entry are skipped
        let mut src =
            Bytes::from_static(&[0x09, 0x18, 0x63, 0x0a, 0x01, b'a', 0x10, 0x07, 0x20, 0x01]);
        let map = Map::deserialize_default(1, WireType::LengthDelimited, &mut src).unwrap();
        assert_eq!(map.get("a"), Some(&7));
        assert!(src.is_empty());
    }

    #[derive(Clone, PartialEq, Debug, Default)]
    struct Node {
        value: u32,
        next: Option<Box<Node>>,
    }

    impl Message for Node {
        fn read(src: &mut Bytes) -> Result<Self, DecodeError> {
            let mut msg = Self::default();
            msg.merge_from(src)?;
            Ok(msg)
        }

        fn write(&self, dst: &mut BytePages) {
            NativeType::serialize(&self.value, 1, DefaultValue::Default, dst);
            NativeType::serialize(&self.next, 2, DefaultValue::Default, dst);
        }

        fn merge_from(&mut self, src: &mut Bytes) -> Result<(), DecodeError> {
            while !src.is_empty() {
                let (tag, wire_type) = encoding::decode_key(src)?;
                match tag {
                    1 => NativeType::deserialize(&mut self.value, tag, wire_type, src)?,
                    2 => NativeType::deserialize(&mut self.next, tag, wire_type, src)?,
                    _ => encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(())
        }

        fn encoded_len(&self) -> usize {
            NativeType::serialized_len(&self.value, 1, DefaultValue::Default)
                + NativeType::serialized_len(&self.next, 2, DefaultValue::Default)
        }
    }

    #[test]
    fn boxed_message() {
        let node = Node {
            value: 1,
            next: Some(Box::new(Node {
                value: 2,
                next: None,
            })),
        };
        let mut buf = BytePages::default();
        Message::write(&Box::new(node.clone()), &mut buf);
        let mut src = buf.freeze();
        assert_eq!(&src[..], &[0x08, 0x01, 0x12, 0x02, 0x08, 0x02]);
        assert_eq!(Message::encoded_len(&node), src.len());

        let decoded = <Box<Node> as Message>::read(&mut src).unwrap();
        assert_eq!(*decoded, node);

        // merging a boxed message merges the inner fields
        let mut boxed = Box::new(Node {
            value: 3,
            next: None,
        });
        let mut src = Bytes::from_static(&[0x12, 0x02, 0x08, 0x05]);
        Message::merge_from(&mut boxed, &mut src).unwrap();
        assert_eq!(boxed.value, 3);
        assert_eq!(boxed.next.as_ref().unwrap().value, 5);

        let boxed: Box<Node> = Box::new(node.clone());
        assert_eq!(Message::encoded_len(&boxed), Message::encoded_len(&node));
    }

    /// `Node` messages nested `depth` levels deep below the top one
    fn nested_nodes(depth: usize) -> Bytes {
        // body length of each level, from the innermost one
        let mut lens = vec![0usize];
        for _ in 0..depth {
            let len = *lens.last().unwrap();
            lens.push(1 + encoding::encoded_len_varint(len as u64) + len);
        }
        let mut buf = BytePages::default();
        for len in lens[..depth].iter().rev() {
            encoding::encode_key(2, WireType::LengthDelimited, &mut buf);
            encoding::encode_varint(*len as u64, &mut buf);
        }
        buf.freeze()
    }

    #[test]
    fn recursion_limit() {
        let limit = encoding::RECURSION_LIMIT as usize;

        let node = Node::read(&mut nested_nodes(limit)).unwrap();
        let mut depth = 0;
        let mut next = node.next.as_deref();
        while let Some(node) = next {
            depth += 1;
            next = node.next.as_deref();
        }
        assert_eq!(depth, limit);

        let err = Node::read(&mut nested_nodes(limit + 1)).unwrap_err();
        assert!(err.to_string().contains("recursion limit reached"));

        // deep input fails without overflowing the stack
        let err = Node::read(&mut nested_nodes(100_000)).unwrap_err();
        assert!(err.to_string().contains("recursion limit reached"));

        // the depth is restored after an error
        Node::read(&mut nested_nodes(limit)).unwrap();
    }

    #[test]
    fn deserialize_errors() {
        // wrong wire type
        let mut src = Bytes::from_static(&[0x01]);
        let err = u32::deserialize_default(1, WireType::LengthDelimited, &mut src).unwrap_err();
        assert!(err.to_string().contains("invalid wire type"));

        // length delimited field longer than the buffer
        let mut src = Bytes::from_static(&[0x05, b'a']);
        let err = String::deserialize_default(1, WireType::LengthDelimited, &mut src).unwrap_err();
        assert!(err.to_string().contains("Not enough data, message size 5"));

        // truncated varint
        let mut src = Bytes::from_static(&[0x80]);
        assert!(u32::deserialize_default(1, WireType::Varint, &mut src).is_err());

        // not utf-8
        for res in [
            String::deserialize_default(
                1,
                WireType::LengthDelimited,
                &mut Bytes::from_static(&[0x01, 0xff]),
            )
            .err(),
            ByteString::deserialize_default(
                1,
                WireType::LengthDelimited,
                &mut Bytes::from_static(&[0x01, 0xff]),
            )
            .err(),
            <Arc<str>>::deserialize_default(
                1,
                WireType::LengthDelimited,
                &mut Bytes::from_static(&[0x01, 0xff]),
            )
            .err(),
        ] {
            assert!(res.unwrap().to_string().contains("not UTF-8 encoded"));
        }

        // fixed width fields need all their bytes
        let mut value = 0_f32;
        assert!(value.merge(&mut Bytes::from_static(&[1, 2, 3])).is_err());
        let mut value = 0_f64;
        assert!(value.merge(&mut Bytes::from_static(&[1, 2, 3])).is_err());
    }

    #[test]
    fn merge_is_not_supported_for_containers() {
        let mut opt: Option<u32> = None;
        assert!(opt.merge(&mut Bytes::new()).is_err());

        let mut vec: Vec<u32> = Vec::new();
        assert!(vec.merge(&mut Bytes::new()).is_err());

        let mut map: HashMap<String, u32> = HashMap::default();
        assert!(map.merge(&mut Bytes::new()).is_err());

        let mut map: HashMapBase<String, u32> = HashMapBase::default();
        assert!(map.merge(&mut Bytes::new()).is_err());

        // containers write nothing through `encode_value`
        let mut buf = BytePages::default();
        Vec::<u32>::new().encode_value(&mut buf);
        HashMap::<String, u32>::default().encode_value(&mut buf);
        assert!(buf.is_empty());
    }

    #[test]
    fn unit_message() {
        assert_eq!(Message::encoded_len(&()), 0);
        let mut buf = BytePages::default();
        Message::write(&(), &mut buf);
        assert!(buf.is_empty());
        <()>::read(&mut Bytes::from_static(b"junk")).unwrap();
    }

    #[test]
    fn message_skips_unknown_fields() {
        // field 9 (unknown), field 3 (bool) = true, field 4 (optional string)
        let mut src = Bytes::from_static(&[0x48, 0x01, 0x18, 0x01, 0x22, 0x02, b'h', b'i']);
        let msg = TestMessage::read(&mut src).unwrap();
        assert!(msg.b);
        assert_eq!(msg.opt.as_deref(), Some("hi"));
        assert_eq!(Message::encoded_len(&msg), 6);

        let mut buf = BytePages::default();
        msg.write(&mut buf);
        assert_eq!(buf.len(), 6);
        assert_eq!(TestMessage::read(&mut buf.freeze()).unwrap(), msg);
    }

    /// Message with message fields, encoded without a length table
    #[derive(Clone, PartialEq, Debug, Default)]
    struct Plain {
        name: String,
        children: Vec<Plain>,
        first: Option<Box<Plain>>,
        props: HashMap<u32, Plain>,
    }

    impl Message for Plain {
        fn read(src: &mut Bytes) -> Result<Self, DecodeError> {
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = encoding::decode_key(src)?;
                match tag {
                    1 => NativeType::deserialize(&mut msg.name, tag, wire_type, src)?,
                    2 => NativeType::deserialize(&mut msg.children, tag, wire_type, src)?,
                    3 => NativeType::deserialize(&mut msg.first, tag, wire_type, src)?,
                    4 => NativeType::deserialize(&mut msg.props, tag, wire_type, src)?,
                    _ => encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        fn write(&self, dst: &mut BytePages) {
            NativeType::serialize(&self.name, 1, DefaultValue::Default, dst);
            NativeType::serialize(&self.children, 2, DefaultValue::Default, dst);
            NativeType::serialize(&self.first, 3, DefaultValue::Default, dst);
            NativeType::serialize(&self.props, 4, DefaultValue::Default, dst);
        }

        fn encoded_len(&self) -> usize {
            NativeType::serialized_len(&self.name, 1, DefaultValue::Default)
                + NativeType::serialized_len(&self.children, 2, DefaultValue::Default)
                + NativeType::serialized_len(&self.first, 3, DefaultValue::Default)
                + NativeType::serialized_len(&self.props, 4, DefaultValue::Default)
        }
    }

    thread_local! {
        static TREE_LEN_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// `Plain` with a length table, written like the generated code
    #[derive(Clone, PartialEq, Debug, Default)]
    struct Tree {
        name: String,
        children: Vec<Tree>,
        first: Option<Box<Tree>>,
        props: HashMap<u32, Tree>,
    }

    impl From<&Plain> for Tree {
        fn from(p: &Plain) -> Self {
            Tree {
                name: p.name.clone(),
                children: p.children.iter().map(Tree::from).collect(),
                first: p.first.as_deref().map(|f| Box::new(Tree::from(f))),
                props: p.props.iter().map(|(k, v)| (*k, Tree::from(v))).collect(),
            }
        }
    }

    impl Message for Tree {
        const NESTED: bool = true;

        fn read(src: &mut Bytes) -> Result<Self, DecodeError> {
            Plain::read(src).map(|p| Tree::from(&p))
        }

        fn write(&self, dst: &mut BytePages) {
            write_nested(self, dst);
        }

        fn write_with(&self, lens: &mut EncodedLens, dst: &mut BytePages) {
            NativeType::serialize(&self.name, 1, DefaultValue::Default, dst);
            NativeType::serialize_with(&self.children, 2, DefaultValue::Default, lens, dst);
            NativeType::serialize_with(&self.first, 3, DefaultValue::Default, lens, dst);
            NativeType::serialize_with(&self.props, 4, DefaultValue::Default, lens, dst);
        }

        fn encoded_len(&self) -> usize {
            NativeType::serialized_len(&self.name, 1, DefaultValue::Default)
                + NativeType::serialized_len(&self.children, 2, DefaultValue::Default)
                + NativeType::serialized_len(&self.first, 3, DefaultValue::Default)
                + NativeType::serialized_len(&self.props, 4, DefaultValue::Default)
        }

        fn encoded_len_with(&self, lens: &mut EncodedLens) -> usize {
            TREE_LEN_CALLS.with(|c| c.set(c.get() + 1));
            NativeType::serialized_len(&self.name, 1, DefaultValue::Default)
                + NativeType::serialized_len_with(&self.children, 2, DefaultValue::Default, lens)
                + NativeType::serialized_len_with(&self.first, 3, DefaultValue::Default, lens)
                + NativeType::serialized_len_with(&self.props, 4, DefaultValue::Default, lens)
        }
    }

    /// `Plain` tree `depth` levels deep, with lengths over one varint byte
    fn plain_tree(depth: u32) -> Plain {
        let mut p = Plain {
            name: "n".repeat(depth as usize * 40),
            ..Default::default()
        };
        if depth > 0 {
            p.children = vec![plain_tree(depth - 1), Plain::default()];
            p.first = Some(Box::new(plain_tree(depth - 1)));
            p.props.insert(depth, plain_tree(depth - 1));
        }
        p
    }

    /// Messages written for `p`, maps leave out default values
    fn tree_size(p: &Plain) -> usize {
        1 + p.children.iter().map(tree_size).sum::<usize>()
            + p.first.as_deref().map_or(0, tree_size)
            + p.props
                .values()
                .filter(|v| **v != Plain::default())
                .map(tree_size)
                .sum::<usize>()
    }

    /// Map entries in `p`, each takes a slot of the length table
    fn map_entries(p: &Plain) -> usize {
        p.props.len()
            + p.children.iter().map(map_entries).sum::<usize>()
            + p.first.as_deref().map_or(0, map_entries)
            + p.props.values().map(map_entries).sum::<usize>()
    }

    fn write_msg<M: Message>(msg: &M) -> Bytes {
        let mut buf = BytePages::default();
        msg.write(&mut buf);
        buf.freeze()
    }

    /// STATEMENT: A message that writes its message fields with a length
    /// table produces the plain encoding and measures each nested message
    /// once.
    ///
    /// HOW:
    /// - Build the same 4 levels deep tree as `Plain` and as `Tree`, with
    ///   repeated, boxed optional and map message fields.
    /// - Write both and compare the bytes, `encoded_len` and the decoded tree.
    /// - Count `Tree::encoded_len_with` calls during the write, one per
    ///   written message; default map values are left out and not measured.
    /// - Write `Tree` again with `write_with` after `encoded_len_with`, and
    ///   confirm the table holds a slot per written message below the root
    ///   and per map entry, and the write reads all of them.
    /// - Measure a `Vec` of leaf messages with `serialized_len_with` and
    ///   confirm it records no slots and leaves the table unallocated.
    #[test]
    fn length_table_writes_plain_bytes() {
        let plain = plain_tree(4);
        let tree = Tree::from(&plain);
        let expected = write_msg(&plain);
        assert!(expected.len() > 1000);

        TREE_LEN_CALLS.with(|c| c.set(0));
        let bytes = write_msg(&tree);
        assert_eq!(TREE_LEN_CALLS.with(std::cell::Cell::get), tree_size(&plain));
        assert_eq!(bytes, expected);
        assert_eq!(Message::encoded_len(&tree), expected.len());
        assert_eq!(Tree::read(&mut bytes.clone()).unwrap(), tree);

        let mut buf = BytePages::default();
        with_encoded_lens(|lens| {
            assert_eq!(tree.encoded_len_with(lens), expected.len());
            assert_eq!(lens.lens.len(), tree_size(&plain) - 1 + map_entries(&plain));
            lens.rewind();
            tree.write_with(lens, &mut buf);
            assert_eq!(lens.pos, lens.lens.len());
        });
        assert_eq!(buf.freeze(), expected);

        let leaves = vec![TestMessage::default(); 3];
        let mut lens = EncodedLens::default();
        assert_eq!(
            leaves.serialized_len_with(1, DefaultValue::Default, &mut lens),
            leaves.serialized_len(1, DefaultValue::Default)
        );
        assert_eq!(lens.lens.capacity(), 0);
    }

    /// STATEMENT: The length table of the thread, reused by the next write,
    /// holds no lengths of the previous write.
    ///
    /// HOW:
    /// - Write a `Tree`, then grow and remove nested messages in it and in its
    ///   `Plain` copy.
    /// - Write the changed `Tree` and compare it with the changed `Plain`
    ///   encoding.
    #[test]
    fn write_reuses_cleared_length_table() {
        let mut plain = plain_tree(3);
        let mut tree = Tree::from(&plain);
        write_msg(&tree);

        let long = "x".repeat(200);
        plain.first.as_mut().unwrap().children[0]
            .name
            .push_str(&long);
        tree.first.as_mut().unwrap().children[0]
            .name
            .push_str(&long);
        plain.props.get_mut(&3).unwrap().first = None;
        tree.props.get_mut(&3).unwrap().first = None;

        let expected = write_msg(&plain);
        assert_eq!(write_msg(&tree), expected);
    }

    /// STATEMENT: A write inside an encode that holds the length table of the
    /// thread uses its own table and leaves the outer one intact.
    ///
    /// HOW:
    /// - Record the lengths of a `Tree` in a table from `with_encoded_lens`,
    ///   which takes the table of the thread on its first slot.
    /// - Write the same `Tree` with `write`, which needs a table too, then
    ///   write it with the outer table.
    /// - Compare both writes with the `Plain` encoding.
    #[test]
    fn nested_write_uses_own_length_table() {
        let plain = plain_tree(3);
        let tree = Tree::from(&plain);
        let expected = write_msg(&plain);

        let mut buf = BytePages::default();
        with_encoded_lens(|lens| {
            tree.encoded_len_with(lens);
            assert_eq!(write_msg(&tree), expected);
            lens.rewind();
            tree.write_with(lens, &mut buf);
        });
        assert_eq!(buf.freeze(), expected);
    }

    /// STATEMENT: A message with a length table written with
    /// `NativeType::serialize`, the path of hand-written parents, is encoded
    /// correctly.
    ///
    /// HOW:
    /// - Serialize a `Tree` and its `Plain` copy as field 1, with and without
    ///   a default.
    /// - Compare the bytes and `serialized_len`, then decode the field back.
    #[test]
    fn nested_message_in_plain_parent() {
        let plain = plain_tree(3);
        let tree = Tree::from(&plain);

        let expected = encode(&plain, 1, DefaultValue::Default);
        assert_eq!(encode(&tree, 1, DefaultValue::Default), expected);
        assert_eq!(encode(&tree, 1, DefaultValue::Unknown), expected);
        assert_eq!(
            NativeType::serialized_len(&tree, 1, DefaultValue::Default),
            expected.len()
        );

        let mut src = expected;
        let (tag, wire_type) = encoding::decode_key(&mut src).unwrap();
        let mut decoded = Tree::default();
        decoded.deserialize(tag, wire_type, &mut src).unwrap();
        assert_eq!(decoded, tree);
    }
}
