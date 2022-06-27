use ntex::util::{Bytes, BytesMut};
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Debug)]
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

impl ntex_grpc::BytesAdapter for UniqueId {
    #[inline]
    fn len(&self) -> usize {
        16
    }

    #[inline]
    fn replace_with(&mut self, buf: Bytes) -> Result<(), ntex_grpc::DecodeError> {
        *self = UniqueId::try_from_bytes(&buf)
            .map_err(|_| ntex_grpc::DecodeError::new("Cannot parse UUID from Bytes"))?;
        Ok(())
    }

    #[inline]
    fn append_to(&self, buf: &mut BytesMut) {
        buf.extend_from_slice(self.0.as_bytes())
    }

    #[inline]
    fn clear(&mut self) {
        *self = Self::nil();
    }

    #[inline]
    fn is_equal(&self, val: &[u8]) -> bool {
        self.as_bytes() == val
    }
}
