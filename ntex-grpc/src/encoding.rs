use std::convert::TryFrom;

use ntex_bytes::{ByteString, Bytes, BytesMut};
use prost::encoding::{self, decode_varint, encode_key, encode_varint, encoded_len_varint};
use prost::encoding::{DecodeContext, WireType};

use crate::{error::DecodeError, types::BytesAdapter};

impl BytesAdapter for Vec<u8> {
    fn len(&self) -> usize {
        Vec::len(self)
    }

    fn replace_with(&mut self, buf: Bytes) -> Result<(), DecodeError> {
        self.clear();
        self.reserve(buf.len());
        self.extend(&buf);
        Ok(())
    }

    fn append_to(&self, buf: &mut BytesMut) {
        buf.extend_from_slice(self.as_slice())
    }

    fn clear(&mut self) {
        self.clear()
    }
}

impl BytesAdapter for Bytes {
    fn len(&self) -> usize {
        Bytes::len(self)
    }

    fn replace_with(&mut self, buf: Bytes) -> Result<(), DecodeError> {
        *self = buf;
        Ok(())
    }

    fn append_to(&self, buf: &mut BytesMut) {
        buf.extend_from_slice(self)
    }

    fn clear(&mut self) {
        self.clear()
    }
}

impl BytesAdapter for String {
    fn len(&self) -> usize {
        String::len(self)
    }

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

    fn append_to(&self, buf: &mut BytesMut) {
        buf.extend_from_slice(self.as_bytes())
    }

    fn clear(&mut self) {
        self.clear()
    }
}

impl BytesAdapter for ByteString {
    fn len(&self) -> usize {
        self.as_slice().len()
    }

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

    fn append_to(&self, buf: &mut BytesMut) {
        buf.extend_from_slice(self.as_bytes())
    }

    fn clear(&mut self) {
        *self = ByteString::new()
    }
}

pub mod bytes {
    use super::*;

    #[inline]
    pub fn clear<A: BytesAdapter>(value: &mut A) {
        value.clear()
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
