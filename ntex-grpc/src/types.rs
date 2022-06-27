use std::fmt::Debug;

pub use ntex_bytes::{ByteString, Bytes, BytesMut};

use prost::encoding::{decode_key, encode_varint, encoded_len_varint, DecodeContext, WireType};
use prost::{DecodeError, EncodeError};

/// A Protocol Buffers message.
pub trait Message: Debug + Default {
    /// Encodes the message to a buffer.
    ///
    /// This method will panic if the buffer has insufficient capacity.
    ///
    /// Meant to be used only by `Message` implementations.
    #[doc(hidden)]
    fn encode_raw(&self, buf: &mut BytesMut)
    where
        Self: Sized;

    /// Decodes a field from a buffer, and merges it into `self`.
    ///
    /// Meant to be used only by `Message` implementations.
    #[doc(hidden)]
    fn merge_field(
        &mut self,
        tag: u32,
        wire_type: WireType,
        buf: &mut Bytes,
        ctx: DecodeContext,
    ) -> Result<(), DecodeError>
    where
        Self: Sized;

    /// Returns the encoded length of the message without a length delimiter.
    fn encoded_len(&self) -> usize;

    /// Encodes the message to a buffer.
    ///
    /// An error will be returned if the buffer does not have sufficient capacity.
    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError>
    where
        Self: Sized,
    {
        buf.reserve(self.encoded_len());
        self.encode_raw(buf);
        Ok(())
    }

    /// Encodes the message with a length-delimiter to a buffer.
    ///
    /// An error will be returned if the buffer does not have sufficient capacity.
    fn encode_length_delimited(&self, buf: &mut BytesMut) -> Result<(), EncodeError>
    where
        Self: Sized,
    {
        let len = self.encoded_len();
        let required = len + encoded_len_varint(len as u64);
        buf.reserve(required);
        encode_varint(len as u64, buf);
        self.encode_raw(buf);
        Ok(())
    }

    /// Decodes an instance of the message from a buffer.
    ///
    /// The entire buffer will be consumed.
    fn decode(buf: &mut Bytes) -> Result<Self, DecodeError> {
        let mut message = Self::default();
        Self::merge(&mut message, buf).map(|_| message)
    }

    /// Decodes a length-delimited instance of the message from the buffer.
    fn decode_length_delimited(buf: &mut Bytes) -> Result<Self, DecodeError> {
        let mut message = Self::default();
        message.merge_length_delimited(buf)?;
        Ok(message)
    }

    /// Decodes an instance of the message from a buffer, and merges it into `self`.
    ///
    /// The entire buffer will be consumed.
    fn merge(&mut self, buf: &mut Bytes) -> Result<(), DecodeError>
    where
        Self: Sized,
    {
        let ctx = DecodeContext::default();
        while !buf.is_empty() {
            let (tag, wire_type) = decode_key(buf)?;
            self.merge_field(tag, wire_type, buf, ctx.clone())?;
        }
        Ok(())
    }

    /// Decodes a length-delimited instance of the message from buffer, and
    /// merges it into `self`.
    fn merge_length_delimited(&mut self, buf: &mut Bytes) -> Result<(), DecodeError>
    where
        Self: Sized,
    {
        let ctx = DecodeContext::default();
        prost::encoding::merge_loop(self, buf, ctx, |msg: &mut Self, buf: &mut Bytes, ctx| {
            let (tag, wire_type) = decode_key(buf)?;
            msg.merge_field(tag, wire_type, buf, ctx)
        })
    }

    /// Clears the message, resetting all fields to their default.
    fn clear(&mut self);
}

pub trait BytesAdapter: Default + Sized {
    fn len(&self) -> usize;

    /// Replace contents of this buffer with the contents of another buffer.
    fn replace_with(&mut self, buf: Bytes) -> Result<(), DecodeError>;

    /// Appends this buffer to the (contents of) other buffer.
    fn append_to(&self, buf: &mut BytesMut);

    /// Clear content
    fn clear(&mut self);

    fn is_equal(&self, val: &[u8]) -> bool;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
