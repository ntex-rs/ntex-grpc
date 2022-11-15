use ntex::util::{Bytes, BytesMut};
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct UniqueId(Uuid);

impl UniqueId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub const fn nil() -> Self {
        Self(Uuid::from_bytes([0; 16]))
    }

    pub fn try_from_bytes(b: &[u8]) -> Result<Self, uuid::Error> {
        Uuid::from_slice(b).map(Self)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }
}

impl Default for UniqueId {
    fn default() -> Self {
        Self::nil()
    }
}

impl ntex_grpc::NativeType for UniqueId {
    const TYPE: ntex_grpc::types::WireType = ntex_grpc::types::WireType::LengthDelimited;

    #[inline]
    fn value_len(&self) -> usize {
        16
    }

    #[inline]
    fn merge(&mut self, src: &mut Bytes) -> Result<(), ntex_grpc::DecodeError> {
        *self = UniqueId::try_from_bytes(src)
            .map_err(|_| ntex_grpc::DecodeError::new("Cannot parse UUID from Bytes"))?;
        Ok(())
    }

    #[inline]
    fn encode_value(&self, dst: &mut BytesMut) {
        dst.extend_from_slice(self.0.as_bytes())
    }

    #[inline]
    fn is_default(&self) -> bool {
        self.as_bytes().is_empty()
    }
}
