//! Field encodings that differ from the default encoding of a Rust type.
//!
//! Generated code uses [`NativeType`] for most fields. A field whose protobuf
//! type shares a Rust type with another protobuf type, such as `sint32`,
//! `sfixed32` and `int32` that are all `i32`, is encoded through a
//! [`FieldFormat`] marker.
#![allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasher, Hash};
use std::marker::PhantomData;

use ntex_bytes::{Buf, BufMut, BytePages, Bytes};
use ntex_util::hash_map::HashMap as HashMapBase;

use crate::encoding::{self, DecodeError, WireType};
use crate::types::{DefaultValue, NativeType};

/// Encoding of a protobuf field of type `T`.
///
/// Implemented by marker types: [`Native`], [`ZigZag`], [`Fixed`] and [`Map`].
pub trait FieldFormat<T> {
    /// Serialize protobuf field
    fn serialize(value: &T, tag: u32, default: DefaultValue<&T>, dst: &mut BytePages);

    /// Protobuf field length
    fn serialized_len(value: &T, tag: u32, default: DefaultValue<&T>) -> usize;

    /// Deserialize protobuf field
    fn deserialize(
        value: &mut T,
        tag: u32,
        wtype: WireType,
        src: &mut Bytes,
    ) -> Result<(), DecodeError>;

    #[inline]
    /// Deserialize protobuf field to default value
    fn deserialize_default(tag: u32, wtype: WireType, src: &mut Bytes) -> Result<T, DecodeError>
    where
        T: Default,
    {
        let mut value = T::default();
        Self::deserialize(&mut value, tag, wtype, src)?;
        Ok(value)
    }
}

/// Encoding defined by the [`NativeType`] implementation of the type.
#[derive(Copy, Clone, Debug)]
pub struct Native;

impl<T: NativeType> FieldFormat<T> for Native {
    #[inline]
    fn serialize(value: &T, tag: u32, default: DefaultValue<&T>, dst: &mut BytePages) {
        value.serialize(tag, default, dst);
    }

    #[inline]
    fn serialized_len(value: &T, tag: u32, default: DefaultValue<&T>) -> usize {
        value.serialized_len(tag, default)
    }

    #[inline]
    fn deserialize(
        value: &mut T,
        tag: u32,
        wtype: WireType,
        src: &mut Bytes,
    ) -> Result<(), DecodeError> {
        value.deserialize(tag, wtype, src)
    }
}

/// Zigzag varint encoding of `sint32` and `sint64` fields.
///
/// Supports `i32`, `i64`, `Option` and `Vec` of them. Repeated fields are
/// written packed, both packed and unpacked values are read.
#[derive(Copy, Clone, Debug)]
pub struct ZigZag;

/// Fixed width little-endian encoding of `fixed32`, `fixed64`, `sfixed32`
/// and `sfixed64` fields.
///
/// Supports `u32`, `u64`, `i32`, `i64`, `Option` and `Vec` of them. Repeated
/// fields are written packed, both packed and unpacked values are read.
#[derive(Copy, Clone, Debug)]
pub struct Fixed;

/// Encoding of a single scalar value
trait Scalar<T> {
    const WIRE_TYPE: WireType;

    fn len(value: T) -> usize;

    fn write(value: T, dst: &mut BytePages);

    fn read(src: &mut Bytes) -> Result<T, DecodeError>;
}

impl Scalar<i32> for ZigZag {
    const WIRE_TYPE: WireType = WireType::Varint;

    #[inline]
    fn len(value: i32) -> usize {
        encoding::encoded_len_varint(zigzag32(value))
    }

    #[inline]
    fn write(value: i32, dst: &mut BytePages) {
        encoding::encode_varint(zigzag32(value), dst);
    }

    #[inline]
    fn read(src: &mut Bytes) -> Result<i32, DecodeError> {
        // sint32 values are 32 bit, upper bits of the varint are ignored
        let value = encoding::decode_varint(src)? as u32;
        Ok(((value >> 1) as i32) ^ -((value & 1) as i32))
    }
}

impl Scalar<i64> for ZigZag {
    const WIRE_TYPE: WireType = WireType::Varint;

    #[inline]
    fn len(value: i64) -> usize {
        encoding::encoded_len_varint(zigzag64(value))
    }

    #[inline]
    fn write(value: i64, dst: &mut BytePages) {
        encoding::encode_varint(zigzag64(value), dst);
    }

    #[inline]
    fn read(src: &mut Bytes) -> Result<i64, DecodeError> {
        let value = encoding::decode_varint(src)?;
        Ok(((value >> 1) as i64) ^ -((value & 1) as i64))
    }
}

#[inline]
fn zigzag32(value: i32) -> u64 {
    u64::from(((value << 1) ^ (value >> 31)) as u32)
}

#[inline]
fn zigzag64(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

macro_rules! fixed {
    ($ty:ty, $wire_type:expr, $put:ident, $get:ident) => {
        impl Scalar<$ty> for Fixed {
            const WIRE_TYPE: WireType = $wire_type;

            #[inline]
            fn len(_: $ty) -> usize {
                size_of::<$ty>()
            }

            #[inline]
            fn write(value: $ty, dst: &mut BytePages) {
                dst.$put(value);
            }

            #[inline]
            fn read(src: &mut Bytes) -> Result<$ty, DecodeError> {
                if src.len() < size_of::<$ty>() {
                    Err(DecodeError::incomplete())
                } else {
                    Ok(src.$get())
                }
            }
        }
    };
}

fixed!(u32, WireType::ThirtyTwoBit, put_u32_le, get_u32_le);
fixed!(i32, WireType::ThirtyTwoBit, put_i32_le, get_i32_le);
fixed!(u64, WireType::SixtyFourBit, put_u64_le, get_u64_le);
fixed!(i64, WireType::SixtyFourBit, put_i64_le, get_i64_le);

macro_rules! scalar {
    ($fmt:ident, $ty:ty) => {
        impl FieldFormat<$ty> for $fmt {
            #[inline]
            fn serialize(value: &$ty, tag: u32, default: DefaultValue<&$ty>, dst: &mut BytePages) {
                let skip = match default {
                    DefaultValue::Unknown => false,
                    DefaultValue::Default => *value == 0,
                    DefaultValue::Value(d) => value == d,
                };
                if !skip {
                    encoding::encode_key(tag, <$fmt as Scalar<$ty>>::WIRE_TYPE, dst);
                    <$fmt as Scalar<$ty>>::write(*value, dst);
                }
            }

            #[inline]
            fn serialized_len(value: &$ty, tag: u32, default: DefaultValue<&$ty>) -> usize {
                let skip = match default {
                    DefaultValue::Unknown => false,
                    DefaultValue::Default => *value == 0,
                    DefaultValue::Value(d) => value == d,
                };
                if skip {
                    0
                } else {
                    encoding::key_len(tag) + <$fmt as Scalar<$ty>>::len(*value)
                }
            }

            #[inline]
            fn deserialize(
                value: &mut $ty,
                _: u32,
                wtype: WireType,
                src: &mut Bytes,
            ) -> Result<(), DecodeError> {
                encoding::check_wire_type(<$fmt as Scalar<$ty>>::WIRE_TYPE, wtype)?;
                *value = <$fmt as Scalar<$ty>>::read(src)?;
                Ok(())
            }
        }

        impl FieldFormat<Option<$ty>> for $fmt {
            #[inline]
            fn serialize(
                value: &Option<$ty>,
                tag: u32,
                _: DefaultValue<&Option<$ty>>,
                dst: &mut BytePages,
            ) {
                if let Some(value) = value {
                    <$fmt as FieldFormat<$ty>>::serialize(value, tag, DefaultValue::Unknown, dst);
                }
            }

            #[inline]
            fn serialized_len(
                value: &Option<$ty>,
                tag: u32,
                _: DefaultValue<&Option<$ty>>,
            ) -> usize {
                value.as_ref().map_or(0, |value| {
                    <$fmt as FieldFormat<$ty>>::serialized_len(value, tag, DefaultValue::Unknown)
                })
            }

            #[inline]
            fn deserialize(
                value: &mut Option<$ty>,
                tag: u32,
                wtype: WireType,
                src: &mut Bytes,
            ) -> Result<(), DecodeError> {
                *value = Some(<$fmt as FieldFormat<$ty>>::deserialize_default(
                    tag, wtype, src,
                )?);
                Ok(())
            }
        }

        impl FieldFormat<Vec<$ty>> for $fmt {
            fn serialize(
                value: &Vec<$ty>,
                tag: u32,
                _: DefaultValue<&Vec<$ty>>,
                dst: &mut BytePages,
            ) {
                if !value.is_empty() {
                    let len: usize = value.iter().map(|v| <$fmt as Scalar<$ty>>::len(*v)).sum();
                    encoding::encode_key(tag, WireType::LengthDelimited, dst);
                    encoding::encode_varint(len as u64, dst);
                    for item in value {
                        <$fmt as Scalar<$ty>>::write(*item, dst);
                    }
                }
            }

            fn serialized_len(value: &Vec<$ty>, tag: u32, _: DefaultValue<&Vec<$ty>>) -> usize {
                if value.is_empty() {
                    0
                } else {
                    let len: usize = value.iter().map(|v| <$fmt as Scalar<$ty>>::len(*v)).sum();
                    encoding::key_len(tag) + encoding::encoded_len_varint(len as u64) + len
                }
            }

            fn deserialize(
                value: &mut Vec<$ty>,
                _: u32,
                wtype: WireType,
                src: &mut Bytes,
            ) -> Result<(), DecodeError> {
                if wtype == WireType::LengthDelimited {
                    let len = encoding::decode_varint(src)? as usize;
                    let mut buf = src
                        .split_to_checked(len)
                        .ok_or_else(DecodeError::incomplete)?;
                    while !buf.is_empty() {
                        value.push(<$fmt as Scalar<$ty>>::read(&mut buf)?);
                    }
                } else {
                    encoding::check_wire_type(<$fmt as Scalar<$ty>>::WIRE_TYPE, wtype)?;
                    value.push(<$fmt as Scalar<$ty>>::read(src)?);
                }
                Ok(())
            }
        }
    };
}

scalar!(ZigZag, i32);
scalar!(ZigZag, i64);
scalar!(Fixed, u32);
scalar!(Fixed, i32);
scalar!(Fixed, u64);
scalar!(Fixed, i64);

/// Map container used by the [`Map`] format.
pub trait MapType: Default {
    /// Map key
    type Key: Default;
    /// Map value
    type Value: Default;

    /// Calls `f` for every entry of the map
    fn for_each_entry(&self, f: impl FnMut(&Self::Key, &Self::Value));

    /// Inserts an entry
    fn insert_entry(&mut self, key: Self::Key, value: Self::Value);
}

macro_rules! hashmap {
    ($ty:ident) => {
        impl<K, V, S> MapType for $ty<K, V, S>
        where
            K: Default + Eq + Hash,
            V: Default,
            S: BuildHasher + Default,
        {
            type Key = K;
            type Value = V;

            fn for_each_entry(&self, mut f: impl FnMut(&K, &V)) {
                for (k, v) in self {
                    f(k, v);
                }
            }

            fn insert_entry(&mut self, key: K, value: V) {
                self.insert(key, value);
            }
        }
    };
}

hashmap!(HashMap);
hashmap!(HashMapBase);

impl<K: Default + Ord, V: Default> MapType for BTreeMap<K, V> {
    type Key = K;
    type Value = V;

    fn for_each_entry(&self, mut f: impl FnMut(&K, &V)) {
        for (k, v) in self {
            f(k, v);
        }
    }

    fn insert_entry(&mut self, key: K, value: V) {
        self.insert(key, value);
    }
}

/// Map field with key format `KF` and value format `VF`.
#[derive(Copy, Clone, Debug)]
pub struct Map<KF, VF>(PhantomData<(KF, VF)>);

impl<M, KF, VF> FieldFormat<M> for Map<KF, VF>
where
    M: MapType,
    KF: FieldFormat<M::Key>,
    VF: FieldFormat<M::Value>,
{
    fn serialize(value: &M, tag: u32, _: DefaultValue<&M>, dst: &mut BytePages) {
        value.for_each_entry(|key, val| {
            let len = KF::serialized_len(key, 1, DefaultValue::Default)
                + VF::serialized_len(val, 2, DefaultValue::Default);
            encoding::encode_key(tag, WireType::LengthDelimited, dst);
            encoding::encode_varint(len as u64, dst);
            KF::serialize(key, 1, DefaultValue::Default, dst);
            VF::serialize(val, 2, DefaultValue::Default, dst);
        });
    }

    fn serialized_len(value: &M, tag: u32, _: DefaultValue<&M>) -> usize {
        let mut total = 0;
        value.for_each_entry(|key, val| {
            let len = KF::serialized_len(key, 1, DefaultValue::Default)
                + VF::serialized_len(val, 2, DefaultValue::Default);
            total += encoding::key_len(tag) + encoding::encoded_len_varint(len as u64) + len;
        });
        total
    }

    fn deserialize(
        value: &mut M,
        _: u32,
        wtype: WireType,
        src: &mut Bytes,
    ) -> Result<(), DecodeError> {
        encoding::check_wire_type(WireType::LengthDelimited, wtype)?;

        let len = encoding::decode_varint(src)? as usize;
        let mut buf = src
            .split_to_checked(len)
            .ok_or_else(DecodeError::incomplete)?;
        let mut key = M::Key::default();
        let mut val = M::Value::default();

        while !buf.is_empty() {
            let (tag, wire_type) = encoding::decode_key(&mut buf)?;
            match tag {
                1 => KF::deserialize(&mut key, 1, wire_type, &mut buf)?,
                2 => VF::deserialize(&mut val, 2, wire_type, &mut buf)?,
                _ => encoding::skip_field(wire_type, tag, &mut buf)?,
            }
        }
        value.insert_entry(key, val);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write<F: FieldFormat<T>, T>(value: &T, default: DefaultValue<&T>) -> Vec<u8> {
        let mut dst = BytePages::default();
        let len = match default {
            DefaultValue::Unknown => F::serialized_len(value, 1, DefaultValue::Unknown),
            DefaultValue::Default => F::serialized_len(value, 1, DefaultValue::Default),
            DefaultValue::Value(d) => F::serialized_len(value, 1, DefaultValue::Value(d)),
        };
        F::serialize(value, 1, default, &mut dst);
        let bytes = dst.freeze().to_vec();
        assert_eq!(len, bytes.len());
        bytes
    }

    fn read<F: FieldFormat<T>, T: Default>(bytes: &[u8]) -> Result<T, DecodeError> {
        let mut value = T::default();
        let mut src = Bytes::copy_from_slice(bytes);
        while !src.is_empty() {
            let (tag, wtype) = encoding::decode_key(&mut src)?;
            F::deserialize(&mut value, tag, wtype, &mut src)?;
        }
        Ok(value)
    }

    #[test]
    fn sint32() {
        for (value, bytes) in [
            (0i32, &[0x08, 0x00][..]),
            (-1, &[0x08, 0x01]),
            (1, &[0x08, 0x02]),
            (-2, &[0x08, 0x03]),
            (i32::MAX, &[0x08, 0xfe, 0xff, 0xff, 0xff, 0x0f]),
            (i32::MIN, &[0x08, 0xff, 0xff, 0xff, 0xff, 0x0f]),
        ] {
            assert_eq!(write::<ZigZag, _>(&value, DefaultValue::Unknown), bytes);
            assert_eq!(read::<ZigZag, i32>(bytes).unwrap(), value);
        }
        // upper bits of a sint32 varint are dropped
        let bytes = [0x08, 0x81, 0x80, 0x80, 0x80, 0x10];
        assert_eq!(read::<ZigZag, i32>(&bytes).unwrap(), -1);
    }

    #[test]
    fn sint64() {
        let max = [
            0x08, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01,
        ];
        let min = [
            0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01,
        ];
        for (value, bytes) in [
            (0i64, &[0x08, 0x00][..]),
            (-1, &[0x08, 0x01]),
            (1, &[0x08, 0x02]),
            (-2_147_483_649, &[0x08, 0x81, 0x80, 0x80, 0x80, 0x10]),
            (i64::MAX, &max),
            (i64::MIN, &min),
        ] {
            assert_eq!(write::<ZigZag, _>(&value, DefaultValue::Unknown), bytes);
            assert_eq!(read::<ZigZag, i64>(bytes).unwrap(), value);
        }
    }

    #[test]
    fn sint_default() {
        assert!(write::<ZigZag, _>(&0i32, DefaultValue::Default).is_empty());
        assert!(write::<ZigZag, _>(&0i64, DefaultValue::Default).is_empty());
        assert_eq!(
            write::<ZigZag, _>(&-1i32, DefaultValue::Default),
            [0x08, 0x01]
        );
        assert!(write::<ZigZag, _>(&5i32, DefaultValue::Value(&5)).is_empty());
        assert_eq!(
            write::<ZigZag, _>(&0i64, DefaultValue::Value(&5)),
            [0x08, 0x00]
        );
    }

    #[test]
    fn sint_wrong_wire_type() {
        assert!(read::<ZigZag, i32>(&[0x0a, 0x01, 0x01]).is_err());
        assert!(read::<ZigZag, Option<i64>>(&[0x0d, 0, 0, 0, 0]).is_err());
        assert!(read::<ZigZag, Vec<i32>>(&[0x0d, 0, 0, 0, 0]).is_err());
        assert!(read::<ZigZag, Vec<i32>>(&[0x0a, 0x03, 0x01]).is_err());
        assert!(read::<ZigZag, i32>(&[0x08, 0xff]).is_err());
    }

    #[test]
    fn sint_option() {
        assert!(write::<ZigZag, Option<i32>>(&None, DefaultValue::Default).is_empty());
        assert_eq!(
            write::<ZigZag, _>(&Some(0i32), DefaultValue::Default),
            [0x08, 0x00]
        );
        assert_eq!(
            write::<ZigZag, _>(&Some(-3i64), DefaultValue::Default),
            [0x08, 0x05]
        );
        assert_eq!(read::<ZigZag, Option<i32>>(&[]).unwrap(), None);
        assert_eq!(read::<ZigZag, Option<i32>>(&[0x08, 0x00]).unwrap(), Some(0));
        assert_eq!(
            read::<ZigZag, Option<i64>>(&[0x08, 0x05]).unwrap(),
            Some(-3)
        );
    }

    #[test]
    fn sint_repeated() {
        assert!(write::<ZigZag, Vec<i32>>(&vec![], DefaultValue::Default).is_empty());
        let bytes = [0x0a, 0x07, 0x02, 0x01, 0xff, 0xff, 0xff, 0xff, 0x0f];
        let value = vec![1i32, -1, i32::MIN];
        assert_eq!(write::<ZigZag, _>(&value, DefaultValue::Default), bytes);
        assert_eq!(read::<ZigZag, Vec<i32>>(&bytes).unwrap(), value);

        // unpacked and mixed input
        let bytes = [0x08, 0x02, 0x0a, 0x01, 0x01, 0x08, 0x03];
        assert_eq!(read::<ZigZag, Vec<i64>>(&bytes).unwrap(), [1, -1, -2]);
        assert_eq!(
            write::<ZigZag, _>(&vec![1i64, -1, -2], DefaultValue::Default),
            [0x0a, 0x03, 0x02, 0x01, 0x03]
        );
    }

    #[test]
    fn deserialize_default() {
        let mut src = Bytes::from_static(&[0x03]);
        let value: i64 =
            <ZigZag as FieldFormat<_>>::deserialize_default(1, WireType::Varint, &mut src)
                .unwrap();
        assert_eq!(value, -2);
        assert!(src.is_empty());
    }

    #[test]
    fn native() {
        assert_eq!(write::<Native, _>(&-1i32, DefaultValue::Default).len(), 11);
        assert!(write::<Native, _>(&0u32, DefaultValue::Default).is_empty());
        assert_eq!(read::<Native, u32>(&[0x08, 0x07]).unwrap(), 7);
        let mut src = Bytes::from_static(&[0x02, b'h', b'i']);
        let value: String =
            Native::deserialize_default(1, WireType::LengthDelimited, &mut src).unwrap();
        assert_eq!(value, "hi");
    }

    #[test]
    fn map_sint_key() {
        type F = Map<ZigZag, Native>;
        let mut map = HashMap::<i32, String>::default();
        map.insert(-1, "a".into());
        let bytes = [0x0a, 0x05, 0x08, 0x01, 0x12, 0x01, b'a'];
        assert_eq!(write::<F, _>(&map, DefaultValue::Default), bytes);
        assert_eq!(read::<F, HashMap<i32, String>>(&bytes).unwrap(), map);

        // default key and value are not written
        let mut map = HashMap::<i32, String>::default();
        map.insert(0, String::new());
        assert_eq!(write::<F, _>(&map, DefaultValue::Default), [0x0a, 0x00]);
        assert_eq!(read::<F, HashMap<i32, String>>(&[0x0a, 0x00]).unwrap(), map);
    }

    #[test]
    fn map_sint_value() {
        type F = Map<Native, ZigZag>;
        let mut map = HashMapBase::<String, i64>::default();
        map.insert("k".into(), i64::MIN);
        let mut bytes = vec![0x0a, 0x0e, 0x0a, 0x01, b'k', 0x10];
        bytes.extend([0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]);
        assert_eq!(write::<F, _>(&map, DefaultValue::Default), bytes);
        assert_eq!(read::<F, HashMapBase<String, i64>>(&bytes).unwrap(), map);

        // value before key, unknown field in the entry
        let bytes = [
            0x0a, 0x09, 0x10, 0x03, 0x18, 0x07, 0x0a, 0x01, b'k', 0x22, 0x00,
        ];
        let map = read::<F, BTreeMap<String, i64>>(&bytes).unwrap();
        assert_eq!(map.into_iter().collect::<Vec<_>>(), [("k".into(), -2)]);
    }

    #[test]
    fn map_errors() {
        type F = Map<ZigZag, ZigZag>;
        assert!(read::<F, BTreeMap<i32, i32>>(&[0x08, 0x01]).is_err());
        assert!(read::<F, BTreeMap<i32, i32>>(&[0x0a, 0x04, 0x08]).is_err());
        assert!(read::<F, BTreeMap<i32, i32>>(&[0x0a, 0x02, 0x0d, 0x00]).is_err());

        let mut map = BTreeMap::new();
        map.insert(-1i32, 1i32);
        map.insert(2, -2);
        let bytes = [
            0x0a, 0x04, 0x08, 0x01, 0x10, 0x02, 0x0a, 0x04, 0x08, 0x04, 0x10, 0x03,
        ];
        assert_eq!(write::<F, _>(&map, DefaultValue::Default), bytes);
        assert_eq!(read::<F, BTreeMap<i32, i32>>(&bytes).unwrap(), map);
    }

    #[test]
    fn fixed() {
        assert_eq!(
            write::<Fixed, _>(&u32::MAX, DefaultValue::Default),
            [0x0d, 0xff, 0xff, 0xff, 0xff]
        );
        assert_eq!(
            write::<Fixed, _>(&-2i32, DefaultValue::Default),
            [0x0d, 0xfe, 0xff, 0xff, 0xff]
        );
        assert_eq!(
            write::<Fixed, _>(&1u64, DefaultValue::Default),
            [0x09, 1, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            write::<Fixed, _>(&-1i64, DefaultValue::Unknown),
            [0x09, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]
        );
        assert!(write::<Fixed, _>(&0u32, DefaultValue::Default).is_empty());
        assert!(write::<Fixed, _>(&7i64, DefaultValue::Value(&7)).is_empty());
        assert_eq!(write::<Fixed, _>(&0i32, DefaultValue::Unknown).len(), 5);

        assert_eq!(
            read::<Fixed, u32>(&[0x0d, 1, 2, 3, 4]).unwrap(),
            0x0403_0201
        );
        assert_eq!(
            read::<Fixed, i32>(&[0x0d, 0xfe, 0xff, 0xff, 0xff]).unwrap(),
            -2
        );
        assert_eq!(
            read::<Fixed, u64>(&[0x09, 1, 0, 0, 0, 0, 0, 0, 0x80]).unwrap(),
            0x8000_0000_0000_0001
        );
        assert_eq!(
            read::<Fixed, i64>(&[0x09, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]).unwrap(),
            -1
        );
    }

    #[test]
    fn fixed_errors() {
        // varint and wrong width
        assert!(read::<Fixed, u32>(&[0x08, 0x01]).is_err());
        assert!(read::<Fixed, u32>(&[0x09, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(read::<Fixed, u64>(&[0x0d, 0, 0, 0, 0]).is_err());
        // truncated
        assert!(read::<Fixed, u32>(&[0x0d, 0, 0, 0]).is_err());
        assert!(read::<Fixed, i64>(&[0x09, 0, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(read::<Fixed, Option<i32>>(&[0x0d, 0]).is_err());
        assert!(read::<Fixed, Vec<u32>>(&[0x0a, 0x03, 0, 0, 0]).is_err());
        assert!(read::<Fixed, Vec<u64>>(&[0x08, 0x01]).is_err());
    }

    #[test]
    fn fixed_option() {
        assert!(write::<Fixed, Option<u32>>(&None, DefaultValue::Default).is_empty());
        assert_eq!(
            write::<Fixed, _>(&Some(0u32), DefaultValue::Default),
            [0x0d, 0, 0, 0, 0]
        );
        assert_eq!(read::<Fixed, Option<i64>>(&[]).unwrap(), None);
        assert_eq!(
            read::<Fixed, Option<i64>>(&[0x09, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap(),
            Some(0)
        );
    }

    #[test]
    fn fixed_repeated() {
        assert!(write::<Fixed, Vec<u64>>(&vec![], DefaultValue::Default).is_empty());
        let bytes = [0x0a, 0x08, 1, 0, 0, 0, 0xff, 0xff, 0xff, 0xff];
        assert_eq!(
            write::<Fixed, _>(&vec![1i32, -1], DefaultValue::Default),
            bytes
        );
        assert_eq!(read::<Fixed, Vec<i32>>(&bytes).unwrap(), [1, -1]);
        assert_eq!(read::<Fixed, Vec<u32>>(&bytes).unwrap(), [1, u32::MAX]);

        // unpacked and mixed input
        let bytes = [
            0x09, 1, 0, 0, 0, 0, 0, 0, 0, 0x0a, 0x08, 2, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(read::<Fixed, Vec<u64>>(&bytes).unwrap(), [1, 2]);
        assert_eq!(
            write::<Fixed, _>(&vec![1u64], DefaultValue::Default),
            [0x0a, 0x08, 1, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn map_fixed() {
        type F = Map<Fixed, ZigZag>;
        type G = Map<Native, Fixed>;

        let mut map = BTreeMap::new();
        map.insert(5u64, -3i32);
        let bytes = [0x0a, 0x0b, 0x09, 5, 0, 0, 0, 0, 0, 0, 0, 0x10, 0x05];
        assert_eq!(write::<F, _>(&map, DefaultValue::Default), bytes);
        assert_eq!(read::<F, BTreeMap<u64, i32>>(&bytes).unwrap(), map);

        let mut map = HashMap::<String, i32>::default();
        map.insert("k".into(), -1);
        let bytes = [0x0a, 0x08, 0x0a, 0x01, b'k', 0x15, 0xff, 0xff, 0xff, 0xff];
        assert_eq!(write::<G, _>(&map, DefaultValue::Default), bytes);
        assert_eq!(read::<G, HashMap<String, i32>>(&bytes).unwrap(), map);
    }
}
