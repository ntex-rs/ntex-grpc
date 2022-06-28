use std::convert::TryFrom;

use ntex_bytes::{ByteString, Bytes, BytesMut};
use prost::encoding::{self, decode_varint, encode_key, encode_varint, encoded_len_varint};
use prost::encoding::{DecodeContext, WireType};
use prost::DecodeError;

use crate::{types::BytesAdapter, Message};

impl BytesAdapter for Vec<u8> {
    #[inline]
    fn len(&self) -> usize {
        Vec::len(self)
    }

    #[inline]
    fn replace_with(&mut self, buf: Bytes) -> Result<(), DecodeError> {
        self.clear();
        self.reserve(buf.len());
        self.extend(&buf);
        Ok(())
    }

    #[inline]
    fn append_to(&self, buf: &mut BytesMut) {
        buf.extend_from_slice(self.as_slice())
    }

    #[inline]
    fn clear(&mut self) {
        self.clear()
    }

    #[inline]
    fn is_equal(&self, val: &[u8]) -> bool {
        self == val
    }
}

impl BytesAdapter for Bytes {
    #[inline]
    fn len(&self) -> usize {
        Bytes::len(self)
    }

    #[inline]
    fn replace_with(&mut self, buf: Bytes) -> Result<(), DecodeError> {
        *self = buf;
        Ok(())
    }

    #[inline]
    fn append_to(&self, buf: &mut BytesMut) {
        buf.extend_from_slice(self)
    }

    #[inline]
    fn clear(&mut self) {
        self.clear()
    }

    #[inline]
    fn is_equal(&self, val: &[u8]) -> bool {
        self == val
    }
}

impl BytesAdapter for String {
    #[inline]
    fn len(&self) -> usize {
        String::len(self)
    }

    #[inline]
    fn replace_with(&mut self, buf: Bytes) -> Result<(), DecodeError> {
        if let Ok(s) = ByteString::try_from(buf) {
            self.push_str(s.as_str());
            Ok(())
        } else {
            Err(DecodeError::new(
                "invalid string value: data is not UTF-8 encoded",
            ))
        }
    }

    #[inline]
    fn append_to(&self, buf: &mut BytesMut) {
        buf.extend_from_slice(self.as_bytes())
    }

    #[inline]
    fn clear(&mut self) {
        self.clear()
    }

    #[inline]
    fn is_equal(&self, val: &[u8]) -> bool {
        self.as_bytes() == val
    }
}

impl BytesAdapter for ByteString {
    #[inline]
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    #[inline]
    fn replace_with(&mut self, buf: Bytes) -> Result<(), DecodeError> {
        if let Ok(s) = ByteString::try_from(buf) {
            *self = s;
            Ok(())
        } else {
            Err(DecodeError::new(
                "invalid string value: data is not UTF-8 encoded",
            ))
        }
    }

    #[inline]
    fn append_to(&self, buf: &mut BytesMut) {
        buf.extend_from_slice(self.as_bytes())
    }

    #[inline]
    fn clear(&mut self) {
        *self = ByteString::new()
    }

    #[inline]
    fn is_equal(&self, val: &[u8]) -> bool {
        self.as_bytes() == val
    }
}

pub mod bytes {
    use super::*;

    #[inline]
    pub fn clear<A: BytesAdapter>(value: &mut A) {
        value.clear()
    }

    #[inline]
    pub fn is_equal<A: BytesAdapter>(value: &A, default: &[u8]) -> bool {
        value.is_equal(default)
    }

    #[inline]
    pub fn encode<A>(tag: u32, value: &A, buf: &mut BytesMut)
    where
        A: BytesAdapter,
    {
        encode_key(tag, WireType::LengthDelimited, buf);
        encode_varint(value.len() as u64, buf);
        value.append_to(buf);
    }

    #[inline]
    pub fn encode_repeated<A>(tag: u32, values: &[A], buf: &mut BytesMut)
    where
        A: BytesAdapter,
    {
        for value in values {
            encode(tag, value, buf);
        }
    }

    #[inline]
    pub fn merge<A>(
        wire_type: WireType,
        value: &mut A,
        buf: &mut Bytes,
        _ctx: DecodeContext,
    ) -> Result<(), DecodeError>
    where
        A: BytesAdapter,
    {
        encoding::check_wire_type(WireType::LengthDelimited, wire_type)?;
        let len = decode_varint(buf)? as usize;

        value.replace_with(buf.split_to(len))
    }

    pub fn merge_repeated<A>(
        wire_type: WireType,
        values: &mut Vec<A>,
        buf: &mut Bytes,
        ctx: DecodeContext,
    ) -> Result<(), DecodeError>
    where
        A: BytesAdapter,
    {
        encoding::check_wire_type(WireType::LengthDelimited, wire_type)?;
        let mut value = Default::default();
        merge(wire_type, &mut value, buf, ctx)?;
        values.push(value);
        Ok(())
    }

    #[inline]
    pub fn encoded_len<A>(tag: u32, value: &A) -> usize
    where
        A: BytesAdapter,
    {
        encoding::key_len(tag) + encoded_len_varint(value.len() as u64) + value.len()
    }

    #[inline]
    pub fn encoded_len_repeated<A>(tag: u32, values: &[A]) -> usize
    where
        A: BytesAdapter,
    {
        encoding::key_len(tag) * values.len()
            + values
                .iter()
                .map(|value| encoded_len_varint(value.len() as u64) + value.len())
                .sum::<usize>()
    }
}

pub mod message {
    use super::*;

    pub fn encode<M>(tag: u32, msg: &M, buf: &mut BytesMut)
    where
        M: Message,
    {
        encode_key(tag, WireType::LengthDelimited, buf);
        encode_varint(msg.encoded_len() as u64, buf);
        msg.encode_raw(buf);
    }

    pub fn encode_repeated<M>(tag: u32, messages: &[M], buf: &mut BytesMut)
    where
        M: Message,
    {
        for msg in messages {
            encode(tag, msg, buf);
        }
    }

    pub fn merge<M>(
        wire_type: WireType,
        msg: &mut M,
        buf: &mut Bytes,
        _ctx: DecodeContext,
    ) -> Result<(), DecodeError>
    where
        M: Message,
    {
        encoding::check_wire_type(WireType::LengthDelimited, wire_type)?;
        msg.merge(buf)
    }

    pub fn merge_repeated<M>(
        wire_type: WireType,
        messages: &mut Vec<M>,
        buf: &mut Bytes,
        ctx: DecodeContext,
    ) -> Result<(), DecodeError>
    where
        M: Message,
    {
        encoding::check_wire_type(WireType::LengthDelimited, wire_type)?;
        let mut msg = M::default();
        merge(WireType::LengthDelimited, &mut msg, buf, ctx)?;
        messages.push(msg);
        Ok(())
    }

    #[inline]
    pub fn encoded_len<M>(tag: u32, msg: &M) -> usize
    where
        M: Message,
    {
        let len = msg.encoded_len();
        encoding::key_len(tag) + encoded_len_varint(len as u64) + len
    }

    #[inline]
    pub fn encoded_len_repeated<M>(tag: u32, messages: &[M]) -> usize
    where
        M: Message,
    {
        encoding::key_len(tag) * messages.len()
            + messages
                .iter()
                .map(Message::encoded_len)
                .map(|len| len + encoded_len_varint(len as u64))
                .sum::<usize>()
    }
}

/// Rust doesn't have a `Map` trait, so macros are currently the best way to be
/// generic over `HashMap` and `BTreeMap`.
macro_rules! map {
    ($map_ty:ident) => {
        use crate::encoding::*;
        use core::hash::Hash;

        /// Generic protobuf map encode function.
        pub fn encode<K, V, KE, KL, VE, VL>(
            key_encode: KE,
            key_encoded_len: KL,
            val_encode: VE,
            val_encoded_len: VL,
            tag: u32,
            values: &$map_ty<K, V>,
            buf: &mut BytesMut,
        ) where
            K: Default + Eq + Hash + Ord,
            V: Default + PartialEq,
            KE: Fn(u32, &K, &mut BytesMut),
            KL: Fn(u32, &K) -> usize,
            VE: Fn(u32, &V, &mut BytesMut),
            VL: Fn(u32, &V) -> usize,
        {
            encode_with_default(
                key_encode,
                key_encoded_len,
                val_encode,
                val_encoded_len,
                &V::default(),
                tag,
                values,
                buf,
            )
        }

        /// Generic protobuf map encode function.
        pub fn encoded_len<K, V, KL, VL>(
            key_encoded_len: KL,
            val_encoded_len: VL,
            tag: u32,
            values: &$map_ty<K, V>,
        ) -> usize
        where
            K: Default + Eq + Hash + Ord,
            V: Default + PartialEq,
            KL: Fn(u32, &K) -> usize,
            VL: Fn(u32, &V) -> usize,
        {
            encoded_len_with_default(key_encoded_len, val_encoded_len, &V::default(), tag, values)
        }

        #[allow(clippy::too_many_arguments)]
        /// Generic protobuf map encode function with an overridden value default.
        ///
        /// This is necessary because enumeration values can have a default value other
        /// than 0 in proto2.
        pub fn encode_with_default<K, V, KE, KL, VE, VL>(
            key_encode: KE,
            key_encoded_len: KL,
            val_encode: VE,
            val_encoded_len: VL,
            val_default: &V,
            tag: u32,
            values: &$map_ty<K, V>,
            buf: &mut BytesMut,
        ) where
            K: Default + Eq + Hash + Ord,
            V: PartialEq,
            KE: Fn(u32, &K, &mut BytesMut),
            KL: Fn(u32, &K) -> usize,
            VE: Fn(u32, &V, &mut BytesMut),
            VL: Fn(u32, &V) -> usize,
        {
            for (key, val) in values.iter() {
                let skip_key = key == &K::default();
                let skip_val = val == val_default;

                let len = (if skip_key { 0 } else { key_encoded_len(1, key) })
                    + (if skip_val { 0 } else { val_encoded_len(2, val) });

                encode_key(tag, WireType::LengthDelimited, buf);
                encode_varint(len as u64, buf);
                if !skip_key {
                    key_encode(1, key, buf);
                }
                if !skip_val {
                    val_encode(2, val, buf);
                }
            }
        }

        /// Generic protobuf map merge function.
        pub fn merge<K, V, KM, VM>(
            key_merge: KM,
            val_merge: VM,
            values: &mut $map_ty<K, V>,
            buf: &mut Bytes,
            ctx: DecodeContext,
        ) -> Result<(), DecodeError>
        where
            K: Default + Eq + Hash + Ord,
            V: Default,
            KM: Fn(WireType, &mut K, &mut Bytes, DecodeContext) -> Result<(), DecodeError>,
            VM: Fn(WireType, &mut V, &mut Bytes, DecodeContext) -> Result<(), DecodeError>,
        {
            merge_with_default(key_merge, val_merge, V::default(), values, buf, ctx)
        }

        /// Generic protobuf map merge function with an overridden value default.
        ///
        /// This is necessary because enumeration values can have a default value other
        /// than 0 in proto2.
        pub fn merge_with_default<K, V, KM, VM>(
            key_merge: KM,
            val_merge: VM,
            val_default: V,
            values: &mut $map_ty<K, V>,
            buf: &mut Bytes,
            ctx: DecodeContext,
        ) -> Result<(), DecodeError>
        where
            K: Default + Eq + Hash + Ord,
            KM: Fn(WireType, &mut K, &mut Bytes, DecodeContext) -> Result<(), DecodeError>,
            VM: Fn(WireType, &mut V, &mut Bytes, DecodeContext) -> Result<(), DecodeError>,
        {
            let mut key = Default::default();
            let mut val = val_default;

            while !buf.is_empty() {
                let (tag, wire_type) = encoding::decode_key(buf)?;
                match tag {
                    1 => key_merge(wire_type, &mut key, buf, ctx.clone())?,
                    2 => val_merge(wire_type, &mut val, buf, ctx.clone())?,
                    _ => encoding::skip_field(wire_type, tag, buf, ctx.clone())?,
                }
            }
            values.insert(key, val);

            Ok(())
        }

        /// Generic protobuf map encode function with an overridden value default.
        ///
        /// This is necessary because enumeration values can have a default value other
        /// than 0 in proto2.
        pub fn encoded_len_with_default<K, V, KL, VL>(
            key_encoded_len: KL,
            val_encoded_len: VL,
            val_default: &V,
            tag: u32,
            values: &$map_ty<K, V>,
        ) -> usize
        where
            K: Default + Eq + Hash + Ord,
            V: PartialEq,
            KL: Fn(u32, &K) -> usize,
            VL: Fn(u32, &V) -> usize,
        {
            encoding::key_len(tag) * values.len()
                + values
                    .iter()
                    .map(|(key, val)| {
                        let len = (if key == &K::default() {
                            0
                        } else {
                            key_encoded_len(1, key)
                        }) + (if val == val_default {
                            0
                        } else {
                            val_encoded_len(2, val)
                        });
                        encoded_len_varint(len as u64) + len
                    })
                    .sum::<usize>()
        }
    };
}

pub mod hash_map {
    use std::collections::HashMap;
    map!(HashMap);
}

pub mod btree_map {
    use std::collections::BTreeMap;
    map!(BTreeMap);
}
