use std::convert::TryFrom;

use ntex_bytes::{ByteString, Bytes, BytesMut};
use prost::encoding::{self, decode_varint, encode_key, encode_varint, encoded_len_varint};
use prost::encoding::{DecodeContext, WireType};

use crate::{error::DecodeError, types::BytesAdapter, Message};

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

// pub mod group {
//     use super::*;

//     pub fn encode<M, B>(tag: u32, msg: &M, buf: &mut B)
//     where
//         M: Message,
//         B: BufMut,
//     {
//         encode_key(tag, WireType::StartGroup, buf);
//         msg.encode_raw(buf);
//         encode_key(tag, WireType::EndGroup, buf);
//     }

//     pub fn merge<M, B>(
//         tag: u32,
//         wire_type: WireType,
//         msg: &mut M,
//         buf: &mut B,
//         ctx: DecodeContext,
//     ) -> Result<(), DecodeError>
//     where
//         M: Message,
//         B: Buf,
//     {
//         check_wire_type(WireType::StartGroup, wire_type)?;

//         ctx.limit_reached()?;
//         loop {
//             let (field_tag, field_wire_type) = decode_key(buf)?;
//             if field_wire_type == WireType::EndGroup {
//                 if field_tag != tag {
//                     return Err(DecodeError::new("unexpected end group tag"));
//                 }
//                 return Ok(());
//             }

//             M::merge_field(msg, field_tag, field_wire_type, buf, ctx.enter_recursion())?;
//         }
//     }

//     pub fn encode_repeated<M, B>(tag: u32, messages: &[M], buf: &mut B)
//     where
//         M: Message,
//         B: BufMut,
//     {
//         for msg in messages {
//             encode(tag, msg, buf);
//         }
//     }

//     pub fn merge_repeated<M, B>(
//         tag: u32,
//         wire_type: WireType,
//         messages: &mut Vec<M>,
//         buf: &mut B,
//         ctx: DecodeContext,
//     ) -> Result<(), DecodeError>
//     where
//         M: Message + Default,
//         B: Buf,
//     {
//         check_wire_type(WireType::StartGroup, wire_type)?;
//         let mut msg = M::default();
//         merge(tag, WireType::StartGroup, &mut msg, buf, ctx)?;
//         messages.push(msg);
//         Ok(())
//     }

//     #[inline]
//     pub fn encoded_len<M>(tag: u32, msg: &M) -> usize
//     where
//         M: Message,
//     {
//         2 * key_len(tag) + msg.encoded_len()
//     }

//     #[inline]
//     pub fn encoded_len_repeated<M>(tag: u32, messages: &[M]) -> usize
//     where
//         M: Message,
//     {
//         2 * key_len(tag) * messages.len()
//             + messages.iter().map(Message::encoded_len).sum::<usize>()
//     }
// }
