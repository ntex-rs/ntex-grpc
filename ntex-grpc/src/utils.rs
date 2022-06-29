use std::mem;

use ntex_bytes::{Bytes, BytesMut};

pub(crate) enum Data {
    Chunk(Bytes),
    MutChunk(BytesMut),
    Empty,
}

impl Data {
    pub(crate) fn get(&mut self) -> Bytes {
        match mem::replace(self, Data::Empty) {
            Data::Chunk(data) => data,
            Data::MutChunk(data) => data.freeze(),
            Data::Empty => Bytes::new(),
        }
    }

    pub(crate) fn push(&mut self, data: Bytes) {
        if !data.is_empty() {
            *self = match mem::replace(self, Data::Empty) {
                Data::Chunk(d) => {
                    let mut d = BytesMut::from(d);
                    d.extend_from_slice(&data);
                    Data::MutChunk(d)
                }
                Data::MutChunk(mut d) => {
                    d.extend_from_slice(&data);
                    Data::MutChunk(d)
                }
                Data::Empty => Data::Chunk(data),
            };
        }
    }
}
