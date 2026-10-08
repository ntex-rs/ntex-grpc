use std::io::{self, Read, Write};

use ntex_bytes::Bytes;
use ntex_http::HeaderValue;

use crate::GrpcStatus;

/// Message compression.
///
/// The client compresses the request message if
/// [`Request::compression()`](crate::client::Request::compression) is set
/// and accepts responses compressed with any of these. The server accepts
/// requests compressed with any of these and compresses the response with
/// the encoding of the request.
///
/// An empty message is never compressed. Large messages are compressed and
/// decompressed on the blocking thread pool of the runtime: with gzip from
/// 16 KiB when compressing and 128 KiB when decompressing, with zstd from
/// 512 KiB.
///
/// Requires the `compression` feature.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Compression {
    /// gzip, the encoding every gRPC implementation supports.
    Gzip,
    /// zstd, smaller and faster than gzip but not supported everywhere.
    Zstd,
}

impl Compression {
    /// The `grpc-encoding` value of the compression.
    pub const fn name(self) -> &'static str {
        match self {
            Compression::Gzip => "gzip",
            Compression::Zstd => "zstd",
        }
    }

    pub(crate) const fn header(self) -> HeaderValue {
        HeaderValue::from_static(self.name())
    }

    /// Returns the compression of a `grpc-encoding` value.
    pub(crate) fn from_header(val: &HeaderValue) -> Option<Self> {
        match val.as_bytes() {
            b"gzip" => Some(Compression::Gzip),
            b"zstd" => Some(Compression::Zstd),
            _ => None,
        }
    }

    /// Messages of this size or larger are compressed on the blocking
    /// thread pool, smaller ones take up to about 1 ms in place.
    const fn compress_limit(self) -> usize {
        match self {
            Compression::Gzip => 16 * 1024,
            Compression::Zstd => 512 * 1024,
        }
    }

    /// Messages that are this size or larger once decompressed are
    /// decompressed on the blocking thread pool.
    const fn decompress_limit(self) -> usize {
        match self {
            Compression::Gzip => 128 * 1024,
            Compression::Zstd => 512 * 1024,
        }
    }

    /// Compresses `data` with the default level of the encoding.
    pub(crate) async fn compress(self, data: Bytes) -> io::Result<Bytes> {
        let blocking = data.len() >= self.compress_limit();
        offload(blocking, move || self.compress_sync(&data))
            .await
            .unwrap_or_else(|err| Err(io::Error::other(err)))
            .map(Bytes::from)
    }

    fn compress_sync(self, data: &[u8]) -> io::Result<Vec<u8>> {
        match self {
            Compression::Gzip => {
                let mut enc =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                enc.write_all(data)?;
                enc.finish()
            }
            Compression::Zstd => {
                let mut enc = zstd::Encoder::new(Vec::new(), 0)?;
                enc.write_all(data)?;
                enc.finish()
            }
        }
    }

    /// Decompresses `data`, the result must not be larger than `max_size`.
    ///
    /// Small messages are decompressed in place. A message that turns out
    /// larger than the decompress limit is decompressed again on the thread
    /// pool, so a small input cannot keep the worker busy for long.
    pub(crate) async fn decompress(
        self,
        data: Bytes,
        max_size: usize,
    ) -> Result<Bytes, (GrpcStatus, HeaderValue)> {
        if data.len() < self.decompress_limit() {
            let limit = max_size.min(self.decompress_limit());
            match self.decompress_sync(&data, limit) {
                Err((GrpcStatus::ResourceExhausted, _)) if max_size > limit => {}
                res => return res.map(Bytes::from),
            }
        }
        offload(true, move || self.decompress_sync(&data, max_size))
            .await
            .unwrap_or_else(|err| {
                Err(error(
                    GrpcStatus::Internal,
                    format!("grpc: failed to decompress the message: {err}"),
                ))
            })
            .map(Bytes::from)
    }

    /// Decompresses `data`, reading at most one byte over `max_size`.
    fn decompress_sync(
        self,
        data: &[u8],
        max_size: usize,
    ) -> Result<Vec<u8>, (GrpcStatus, HeaderValue)> {
        let limit = u64::try_from(max_size)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let mut out = Vec::new();
        let res = match self {
            Compression::Gzip => flate2::read::MultiGzDecoder::new(data)
                .take(limit)
                .read_to_end(&mut out),
            Compression::Zstd => match zstd::Decoder::with_buffer(data) {
                Ok(dec) => dec.take(limit).read_to_end(&mut out),
                Err(err) => {
                    return Err(error(
                        GrpcStatus::Internal,
                        format!("grpc: failed to decompress the message: {err}"),
                    ));
                }
            },
        };
        if let Err(err) = res {
            Err(error(
                GrpcStatus::Internal,
                format!("grpc: failed to read decompressed data: {err}"),
            ))
        } else if out.len() > max_size {
            Err(error(
                GrpcStatus::ResourceExhausted,
                format!("grpc: received message after decompression larger than max {max_size}"),
            ))
        } else {
            Ok(out)
        }
    }
}

/// Runs `f` on the blocking thread pool if `blocking` is set, in place
/// otherwise.
async fn offload<F, R>(blocking: bool, f: F) -> Result<R, ntex_rt::BlockingError>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    if blocking {
        #[cfg(test)]
        OFFLOADED.set(OFFLOADED.get() + 1);
        ntex_rt::spawn_blocking(f).await
    } else {
        Ok(f())
    }
}

#[cfg(test)]
thread_local! {
    /// The number of calls run on the thread pool.
    static OFFLOADED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn error(status: GrpcStatus, msg: String) -> (GrpcStatus, HeaderValue) {
    let msg = HeaderValue::try_from(msg)
        .unwrap_or_else(|_| HeaderValue::from_static("grpc: failed to decompress the message"));
    (status, msg)
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    const ALL: [Compression; 2] = [Compression::Gzip, Compression::Zstd];

    async fn compress(enc: Compression, data: &[u8]) -> Bytes {
        enc.compress(Bytes::copy_from_slice(data)).await.unwrap()
    }

    #[ntex::test]
    async fn round_trip() {
        let data = b"hello hello hello hello hello hello".repeat(100);
        for enc in ALL {
            let compressed = compress(enc, &data).await;
            assert!(compressed.len() < data.len(), "{enc:?}");
            assert_eq!(
                enc.decompress(compressed.clone(), data.len())
                    .await
                    .unwrap(),
                data
            );

            let (status, msg) = enc
                .decompress(compressed.clone(), data.len() - 1)
                .await
                .unwrap_err();
            assert_eq!(status, GrpcStatus::ResourceExhausted);
            assert_eq!(
                msg,
                format!(
                    "grpc: received message after decompression larger than max {}",
                    data.len() - 1
                )
            );
            assert_eq!(enc.decompress(compressed, usize::MAX).await.unwrap(), data);
        }
    }

    #[ntex::test]
    async fn large() {
        for enc in ALL {
            let limit = enc.decompress_limit();
            // small when compressed, larger than the limit when not
            let data = b"0123456789".repeat(limit);
            let compressed = compress(enc, &data).await;
            assert!(compressed.len() < limit, "{enc:?}");
            assert_eq!(
                enc.decompress(compressed.clone(), data.len())
                    .await
                    .unwrap(),
                data
            );
            for max in [limit - 1, limit, data.len() - 1] {
                let (status, msg) = enc.decompress(compressed.clone(), max).await.unwrap_err();
                assert_eq!(status, GrpcStatus::ResourceExhausted);
                assert_eq!(
                    msg,
                    format!("grpc: received message after decompression larger than max {max}")
                );
            }

            // the compressed message is large too
            let mut x = 0x2545_f491_u32;
            let data: Vec<u8> = (0..limit * 2)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    x.to_le_bytes()[0]
                })
                .collect();
            let compressed = compress(enc, &data).await;
            assert!(compressed.len() >= limit, "{enc:?}");
            assert_eq!(
                enc.decompress(compressed.clone(), data.len())
                    .await
                    .unwrap(),
                data
            );
            let (status, _) = enc
                .decompress(compressed, data.len() - 1)
                .await
                .unwrap_err();
            assert_eq!(status, GrpcStatus::ResourceExhausted);
        }
    }

    #[ntex::test]
    async fn offload_blocking() {
        let id = thread::current().id();
        let run = |blocking| offload(blocking, || thread::current().id());
        assert_eq!(run(false).await.unwrap(), id);
        assert_ne!(run(true).await.unwrap(), id);
    }

    #[test]
    fn limits() {
        // gzip compression is about 40 times slower than the rest
        assert_eq!(Compression::Gzip.compress_limit(), 16 * 1024);
        assert_eq!(Compression::Gzip.decompress_limit(), 128 * 1024);
        assert_eq!(Compression::Zstd.compress_limit(), 512 * 1024);
        assert_eq!(Compression::Zstd.decompress_limit(), 512 * 1024);
    }

    /// Returns the result of `f` and whether it used the thread pool.
    async fn offloaded<R>(f: impl Future<Output = R>) -> (R, bool) {
        let before = OFFLOADED.get();
        let res = f.await;
        (res, OFFLOADED.get() != before)
    }

    #[ntex::test]
    async fn offload_limits() {
        for enc in ALL {
            let limit = enc.compress_limit();
            for (size, blocking) in [(limit - 1, false), (limit, true)] {
                let data = Bytes::from(vec![b'a'; size]);
                let (res, used) = offloaded(enc.compress(data)).await;
                assert!(res.is_ok());
                assert_eq!(used, blocking, "{enc:?} {size}");
            }

            let limit = enc.decompress_limit();
            for (size, max, blocking) in [
                // small once decompressed
                (limit - 1, usize::MAX, false),
                (limit, usize::MAX, false),
                // decompressed again on the pool
                (limit + 1, usize::MAX, true),
                // over the limit of the message, no need to go on
                (limit + 1, limit, false),
            ] {
                let data = compress(enc, &vec![b'a'; size]).await;
                let (res, used) = offloaded(enc.decompress(data, max)).await;
                assert_eq!(res.is_ok(), size <= max, "{enc:?} {size}");
                assert_eq!(used, blocking, "{enc:?} {size} {max}");
            }

            // the compressed message is large
            let data = Bytes::from(vec![1; limit]);
            let (res, used) = offloaded(enc.decompress(data, usize::MAX)).await;
            assert!(res.is_err());
            assert!(used, "{enc:?}");
        }
    }

    #[ntex::test]
    async fn empty() {
        for enc in ALL {
            let compressed = compress(enc, b"").await;
            assert!(enc.decompress(compressed, 0).await.unwrap().is_empty());
        }
    }

    #[ntex::test]
    async fn gzip_members() {
        // concatenated gzip members are one stream, as in Go
        let mut data = compress(Compression::Gzip, b"abc").await.to_vec();
        data.extend_from_slice(&compress(Compression::Gzip, b"def").await);
        assert_eq!(
            Compression::Gzip
                .decompress(Bytes::from(data), 6)
                .await
                .unwrap(),
            b"abcdef"[..]
        );
    }

    #[ntex::test]
    async fn invalid() {
        for enc in ALL {
            for data in [b"abc".to_vec(), vec![1; enc.decompress_limit()]] {
                let (status, msg) = enc
                    .decompress(Bytes::from(data), usize::MAX)
                    .await
                    .unwrap_err();
                assert_eq!(status, GrpcStatus::Internal);
                assert!(
                    msg.to_str()
                        .unwrap()
                        .starts_with("grpc: failed to read decompressed data: "),
                    "{msg:?}"
                );
            }
        }
    }

    #[test]
    fn names() {
        for enc in ALL {
            assert_eq!(Compression::from_header(&enc.header()), Some(enc));
        }
        assert_eq!(Compression::Gzip.name(), "gzip");
        assert_eq!(Compression::Zstd.name(), "zstd");
        assert_eq!(
            Compression::from_header(&HeaderValue::from_static("GZIP")),
            None
        );
        assert_eq!(
            Compression::from_header(&HeaderValue::from_static("identity")),
            None
        );
    }
}
