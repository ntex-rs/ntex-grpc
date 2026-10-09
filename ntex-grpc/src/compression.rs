use std::io::{self, Read, Write};
use std::{fmt, iter};

use ntex_bytes::{BytePage, BytePages, Bytes, BytesMut};
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
/// 16 KiB when compressing and 128 KiB of compressed data when
/// decompressing, with zstd from 512 KiB.
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

    /// Compressed messages of this size or larger are decompressed on the
    /// blocking thread pool.
    const fn decompress_limit(self) -> usize {
        match self {
            Compression::Gzip => 128 * 1024,
            Compression::Zstd => 512 * 1024,
        }
    }

    /// Compresses `data` with the default level of the encoding, `data` is
    /// left empty.
    ///
    /// The pages are passed to the encoder one by one, they are not joined
    /// into a single buffer first.
    pub(crate) async fn compress(
        self,
        data: &mut BytePages,
    ) -> Result<Bytes, (GrpcStatus, HeaderValue)> {
        let len = data.len();
        let pages: Vec<BytePage> = iter::from_fn(|| data.take()).collect();
        offload(len >= self.compress_limit(), move || {
            self.compress_sync(len, &pages)
        })
        .await
        .map_err(io::Error::other)
        .flatten()
        .map_err(|err| error(GrpcStatus::Internal, "grpc: error while compressing", err))
    }

    fn compress_sync(self, len: usize, pages: &[BytePage]) -> io::Result<Bytes> {
        fn write<W: Write>(mut enc: W, pages: &[BytePage]) -> io::Result<W> {
            for page in pages {
                enc.write_all(page)?;
            }
            Ok(enc)
        }

        let out = match self {
            Compression::Gzip => {
                let enc =
                    flate2::write::GzEncoder::new(BytesMut::new(), flate2::Compression::default());
                write(enc, pages)?.finish()?
            }
            Compression::Zstd => {
                // the frame stores the size, the receiver allocates it at once
                let mut enc = zstd::Encoder::new(BytesMut::new(), 0)?;
                enc.set_pledged_src_size(u64::try_from(len).ok())?;
                write(enc, pages)?.finish()?
            }
        };
        Ok(freeze(out))
    }

    /// Decompresses `data`, the result must not be larger than `max_size`.
    ///
    /// A large compressed message is decompressed on the thread pool, a
    /// small one in place.
    pub(crate) async fn decompress(
        self,
        data: Bytes,
        max_size: usize,
    ) -> Result<Bytes, (GrpcStatus, HeaderValue)> {
        offload(data.len() >= self.decompress_limit(), move || {
            self.decompress_sync(&data, max_size)
        })
        .await
        .unwrap_or_else(|err| {
            Err(error(
                GrpcStatus::Internal,
                "grpc: failed to decompress the message",
                err,
            ))
        })
    }

    /// The decompressed size stored in `data`, at most `limit`. gzip stores
    /// the size of the last member only, and a deflate stream expands at
    /// most 1032 times, so a small message cannot claim a large size.
    fn size_hint(self, data: &[u8], limit: usize) -> usize {
        let size = match self {
            Compression::Gzip => data
                .last_chunk()
                .map_or(0, |size: &[u8; 4]| u32::from_le_bytes(*size) as usize),
            Compression::Zstd => zstd::zstd_safe::get_frame_content_size(data)
                .ok()
                .flatten()
                .map_or(0, |size| usize::try_from(size).unwrap_or(usize::MAX)),
        };
        size.min(limit).min(data.len().saturating_mul(1032))
    }

    /// Decompresses `data`, reading at most one byte over `max_size`.
    fn decompress_sync(
        self,
        data: &[u8],
        max_size: usize,
    ) -> Result<Bytes, (GrpcStatus, HeaderValue)> {
        let limit = max_size.saturating_add(1);
        let hint = self.size_hint(data, limit);
        let out = match self {
            Compression::Gzip => read(flate2::read::MultiGzDecoder::new(data), hint, limit),
            Compression::Zstd => {
                zstd::Decoder::with_buffer(data).and_then(|dec| read(dec, hint, limit))
            }
        }
        .map_err(|err| {
            error(
                GrpcStatus::Internal,
                "grpc: failed to read decompressed data",
                err,
            )
        })?;
        if out.len() > max_size {
            let msg =
                format!("grpc: received message after decompression larger than max {max_size}");
            let msg = HeaderValue::try_from(msg).unwrap_or_else(|_| {
                HeaderValue::from_static("grpc: received message after decompression too large")
            });
            return Err((GrpcStatus::ResourceExhausted, msg));
        }
        Ok(freeze(out))
    }
}

/// Reads at most `limit` bytes from `r` into a buffer of `hint` bytes, which
/// grows if the data does not fit.
fn read(r: impl Read, hint: usize, limit: usize) -> io::Result<BytesMut> {
    let mut out = BytesMut::with_capacity(hint);
    io::copy(&mut r.take(limit as u64), &mut out)?;
    Ok(out)
}

/// Freezes `buf`, a buffer with more than a quarter of it unused is copied
/// so the message does not keep the unused memory.
fn freeze(buf: BytesMut) -> Bytes {
    if buf.capacity() - buf.len() > buf.len() / 4 {
        Bytes::copy_from_slice(&buf)
    } else {
        buf.freeze()
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

/// Returns `status` with a message of `msg: err`, or of `msg` alone if `err`
/// is not valid in a header.
fn error(
    status: GrpcStatus,
    msg: &'static str,
    err: impl fmt::Display,
) -> (GrpcStatus, HeaderValue) {
    let val = HeaderValue::try_from(format!("{msg}: {err}"))
        .unwrap_or_else(|_| HeaderValue::from_static(msg));
    (status, val)
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    const ALL: [Compression; 2] = [Compression::Gzip, Compression::Zstd];

    /// Compresses `data` stored in 4 KiB pages.
    async fn compress(enc: Compression, data: &[u8]) -> Bytes {
        let mut pages = BytePages::new(ntex_bytes::BytePageSize::Size4);
        pages.extend_from_slice(data);
        let res = enc.compress(&mut pages).await.unwrap();
        assert!(pages.is_empty());
        res
    }

    #[ntex::test]
    async fn compress_pages() {
        let data: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        for enc in ALL {
            let mut pages = BytePages::new(ntex_bytes::BytePageSize::Size4);
            pages.extend_from_slice(&data);
            assert!(pages.num_pages() > 1);
            let compressed = enc.compress(&mut pages).await.unwrap();
            assert!(pages.is_empty());
            assert_eq!(
                enc.decompress(compressed, data.len()).await.unwrap(),
                data,
                "{enc:?}"
            );
        }
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
                let mut data = BytePages::default();
                data.extend_from_slice(&vec![b'a'; size]);
                let (res, used) = offloaded(enc.compress(&mut data)).await;
                assert!(res.is_ok());
                assert_eq!(used, blocking, "{enc:?} {size}");
            }

            let limit = enc.decompress_limit();
            for (size, max, blocking) in [
                // a small compressed message stays in place, however large
                // it is once decompressed
                (limit - 1, usize::MAX, false),
                (limit, usize::MAX, false),
                (limit * 4, usize::MAX, false),
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

    #[ntex::test]
    async fn size_hint() {
        for enc in ALL {
            let data = b"0123456789".repeat(1000);
            let compressed = compress(enc, &data).await;
            assert_eq!(enc.size_hint(&compressed, usize::MAX), data.len());
            assert_eq!(enc.size_hint(&compressed, 100), 100);
            assert_eq!(enc.size_hint(b"", usize::MAX), 0);
        }
        // zstd stores the size in the frame
        let compressed = compress(Compression::Zstd, b"abc").await;
        assert_eq!(
            zstd::zstd_safe::get_frame_content_size(&compressed).ok(),
            Some(Some(3))
        );
        // the size of a small message is limited
        let data = [1, 2, 3, 4, 0xff, 0xff, 0xff, 0xff];
        assert_eq!(Compression::Gzip.size_hint(&data, usize::MAX), 8 * 1032);
        assert_eq!(Compression::Gzip.size_hint(&data[5..], usize::MAX), 0);
    }

    /// Returns the data in parts of up to `n` bytes, with an interrupted read
    /// before each part.
    struct Parts<'a>(&'a [u8], usize, bool);

    impl Read for Parts<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.2 = !self.2;
            if self.2 {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let n = buf.len().min(self.1).min(self.0.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    #[test]
    fn read_hint() {
        let data: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
        for part in [1, 7, 32, 4096, usize::MAX] {
            for hint in [0, 1, 50, 31_000, 100_000] {
                let out = read(Parts(&data, part, false), hint, usize::MAX).unwrap();
                assert_eq!(out, data, "{part} {hint}");
                if hint == data.len() {
                    // allocated once
                    assert_eq!(out.capacity(), data.len(), "{part}");
                }
            }
            // one byte over the limit
            let out = read(Parts(&data, part, false), 0, 1000).unwrap();
            assert_eq!(out, data[..1000]);
            let out = read(Parts(&data, part, false), 1000, 1000).unwrap();
            assert_eq!(out, data[..1000]);
            assert_eq!(out.capacity(), 1000);
            let out = read(Parts(&data[..10], part, false), 1000, 1000).unwrap();
            assert_eq!(out, data[..10]);
            let out = read(Parts(&data[..100], part, false), 100, usize::MAX).unwrap();
            assert_eq!(out, data[..100]);
            assert_eq!(out.capacity(), 100);
            let out = read(Parts(&data, part, false), 0, 10).unwrap();
            assert_eq!(out, data[..10]);
        }
        let out = read(Parts(b"", 1, false), 0, usize::MAX).unwrap();
        assert!(out.is_empty());
        assert_eq!(out.capacity(), 0);

        let err = read(io::repeat(0).take(5).chain(Fail), 10, 20).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let err = read(Fail, 0, 20).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    struct Fail;

    impl Read for Fail {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::InvalidData.into())
        }
    }

    #[test]
    fn freeze_slack() {
        let data = [7; 1000];
        let mut buf = BytesMut::with_capacity(1250);
        buf.extend_from_slice(&data);
        let ptr = buf.as_ptr();
        assert_eq!(freeze(buf).as_ptr(), ptr);

        let mut buf = BytesMut::with_capacity(1251);
        buf.extend_from_slice(&data);
        let ptr = buf.as_ptr();
        let out = freeze(buf);
        assert_ne!(out.as_ptr(), ptr);
        assert_eq!(out, data[..]);
    }

    #[test]
    fn errors() {
        let (status, msg) = error(GrpcStatus::Internal, "grpc: failed", "bad data");
        assert_eq!(status, GrpcStatus::Internal);
        assert_eq!(msg, "grpc: failed: bad data");
        let (_, msg) = error(GrpcStatus::Internal, "grpc: failed", "bad\ndata");
        assert_eq!(msg, "grpc: failed");
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
