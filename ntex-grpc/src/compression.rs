use std::io::{self, Cursor, Read};
use std::{cell::Cell, fmt, iter, mem::MaybeUninit, slice, thread::LocalKey};

use flate2::{Compress, Crc, FlushCompress, Status, bufread::GzDecoder};
use ntex_bytes::{BufMut, BytePage, BytePages, Bytes, BytesMut};
use ntex_http::HeaderValue;
use zstd::zstd_safe::zstd_sys::ZSTD_EndDirective;
use zstd::zstd_safe::{CCtx, CParameter, DCtx, InBuffer, OutBuffer, ResetDirective, WriteBuf};

use crate::GrpcStatus;

/// Message compression.
///
/// The client compresses the request message if
/// [`Request::compression()`](crate::client::Request::compression) is set
/// and accepts responses compressed with any of these. The server accepts
/// requests compressed with any of these and compresses the response with
/// the encoding of the request.
///
/// Messages under 64 bytes, and messages that do not get smaller, are sent
/// uncompressed. Large messages are compressed and decompressed on the
/// blocking thread pool of the runtime: with gzip from 16 KiB when
/// compressing and 128 KiB of compressed data when decompressing, with zstd
/// from 512 KiB.
///
/// Each thread keeps the encoder and the decoder of its last message for
/// the next one, creating them takes as long as compressing a small
/// message. The gzip encoder takes about 320 KiB and the decoder 43 KiB,
/// zstd ones are kept up to 256 KiB each.
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

    /// Compresses `data` with the default level of the encoding.
    ///
    /// Returns `false` and leaves `data` as it is if the message is smaller
    /// than [`MIN_SIZE`] or does not get smaller, it is sent uncompressed
    /// then. The pages are passed to the encoder one by one, they are not
    /// joined into a single buffer first.
    pub(crate) async fn compress(
        self,
        data: &mut BytePages,
    ) -> Result<bool, (GrpcStatus, HeaderValue)> {
        let len = data.len();
        if len < MIN_SIZE {
            return Ok(false);
        }
        let pages: Vec<BytePage> = iter::from_fn(|| data.take()).collect();
        let (pages, res) = offload(len >= self.compress_limit(), move || {
            let res = self.compress_sync(len, &pages);
            (pages, res)
        })
        .await
        .map_err(|err| error(GrpcStatus::Internal, "grpc: error while compressing", err))?;
        match res {
            Ok(Some(out)) => {
                *data = out;
                Ok(true)
            }
            Ok(None) => {
                for page in pages {
                    data.append(page);
                }
                Ok(false)
            }
            Err(err) => Err(error(
                GrpcStatus::Internal,
                "grpc: error while compressing",
                err,
            )),
        }
    }

    /// Returns the compressed `pages`, or `None` if they do not get smaller.
    fn compress_sync(self, len: usize, pages: &[BytePage]) -> io::Result<Option<BytePages>> {
        match self {
            Compression::Gzip => with_context(
                &GZIP_ENCODER,
                || Ok(Compress::new(flate2::Compression::default(), false)),
                |ctx| gzip_compress(ctx, pages, len),
                |_| true,
            ),
            Compression::Zstd => with_context(
                &ZSTD_ENCODER,
                || {
                    let mut ctx = CCtx::try_create().ok_or(io::ErrorKind::OutOfMemory)?;
                    ctx.set_parameter(CParameter::WindowLog(ZSTD_WINDOW_LOG))
                        .map_err(zstd_error)?;
                    Ok(ctx)
                },
                |ctx| zstd_compress(ctx, pages, len),
                |ctx| ctx.sizeof() <= CONTEXT_LIMIT,
            ),
        }
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

    /// The decompressed size stored in `data`, at most `limit`.
    ///
    /// The size is not checked until the data is decompressed, so at most
    /// [`RESERVE_RATIO`] times the size of `data` is trusted. gzip stores the
    /// size of the last member only.
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
        size.min(limit)
            .min(data.len().saturating_mul(RESERVE_RATIO))
    }

    /// Decompresses `data`, reading at most one byte over `max_size`.
    fn decompress_sync(
        self,
        data: &Bytes,
        max_size: usize,
    ) -> Result<Bytes, (GrpcStatus, HeaderValue)> {
        let limit = max_size.saturating_add(1);
        let hint = self.size_hint(data, limit);
        let out = match self {
            Compression::Gzip => with_context(
                &GZIP_DECODER,
                || Ok(GzDecoder::new(Cursor::default())),
                |dec| read(Members::new(dec, data), hint, limit),
                |dec| {
                    // the decoder does not keep the message
                    dec.reset(Cursor::default());
                    true
                },
            ),
            Compression::Zstd => with_context(
                &ZSTD_DECODER,
                || Ok(DCtx::try_create().ok_or(io::ErrorKind::OutOfMemory)?),
                |ctx| {
                    ctx.reset(ResetDirective::SessionOnly).map_err(zstd_error)?;
                    zstd_read(ctx, data, hint, limit)
                },
                |ctx| ctx.sizeof() <= CONTEXT_LIMIT,
            ),
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

/// Messages smaller than this are sent uncompressed, they rarely get smaller.
pub(crate) const MIN_SIZE: usize = 64;

/// The largest zstd window, the default level uses 2 MiB for messages over
/// 1 MiB. The window takes memory on both sides.
const ZSTD_WINDOW_LOG: u32 = 19;

/// The buffer of a decompressed message is reserved up front for at most
/// this many times the compressed size. A message that compresses better
/// grows the buffer as it is decompressed.
const RESERVE_RATIO: usize = 64;

/// The header `flate2::write::GzEncoder` writes, without a name or time, for
/// an unknown system.
const GZIP_HEADER: [u8; 10] = [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];

/// A zstd context larger than this is not kept for the next message. The
/// encoder of an 8 KiB message takes about 180 KiB, the decoder about 96 KiB
/// unless it buffers the window.
const CONTEXT_LIMIT: usize = 256 * 1024;

thread_local! {
    static GZIP_ENCODER: Cell<Option<Compress>> = const { Cell::new(None) };
    static GZIP_DECODER: Cell<Option<GzDecoder<Cursor<Bytes>>>> = const { Cell::new(None) };
    static ZSTD_ENCODER: Cell<Option<CCtx<'static>>> = const { Cell::new(None) };
    static ZSTD_DECODER: Cell<Option<DCtx<'static>>> = const { Cell::new(None) };
}

/// Runs `f` with the context the thread keeps in `key`, or with a new one.
/// The context is kept for the next message if `keep` returns `true`.
fn with_context<T, R>(
    key: &'static LocalKey<Cell<Option<T>>>,
    new: impl FnOnce() -> io::Result<T>,
    f: impl FnOnce(&mut T) -> io::Result<R>,
    keep: impl FnOnce(&mut T) -> bool,
) -> io::Result<R> {
    let mut ctx = match key.try_with(Cell::take).ok().flatten() {
        Some(ctx) => ctx,
        None => new()?,
    };
    let res = f(&mut ctx);
    if keep(&mut ctx) {
        let _ = key.try_with(|cell| cell.set(Some(ctx)));
    }
    res
}

/// Compresses `pages` into the gzip member `flate2::write::GzEncoder`
/// writes, returns `None` once it is as large as the `len` bytes of the
/// input.
fn gzip_compress(
    ctx: &mut Compress,
    pages: &[BytePage],
    len: usize,
) -> io::Result<Option<BytePages>> {
    ctx.reset();
    let mut deflate = |out: &mut BytePages, input: &[u8], flush| {
        if out.len() >= len {
            return Ok(None);
        }
        fill(out, len - out.len(), |buf| {
            let (read, written) = (ctx.total_in(), ctx.total_out());
            let status = ctx
                .compress_uninit(input, buf, flush)
                .map_err(io::Error::other)?;
            let read = delta(ctx.total_in(), read);
            Ok((delta(ctx.total_out(), written), Some((read, status))))
        })
    };

    let mut out = BytePages::default();
    out.extend_from_slice(&GZIP_HEADER);
    let mut crc = Crc::new();
    for page in pages {
        crc.update(page);
        let mut input = &page[..];
        while !input.is_empty() {
            let Some((read, _)) = deflate(&mut out, input, FlushCompress::None)? else {
                return Ok(None);
            };
            input = &input[read..];
        }
    }
    loop {
        match deflate(&mut out, &[], FlushCompress::Finish)? {
            None => return Ok(None),
            Some((_, Status::StreamEnd)) => break,
            Some(_) => {}
        }
    }
    if out.len() + 8 >= len {
        return Ok(None);
    }
    out.extend_from_slice(&crc.sum().to_le_bytes());
    out.extend_from_slice(&crc.amount().to_le_bytes());
    Ok(Some(out))
}

/// The bytes between two totals of the gzip encoder, at most the size of a
/// buffer.
fn delta(after: u64, before: u64) -> usize {
    usize::try_from(after - before).unwrap_or(usize::MAX)
}

/// Compresses `pages` into a zstd frame, returns `None` once it is as large
/// as the `len` bytes of the input.
fn zstd_compress(
    ctx: &mut CCtx<'_>,
    pages: &[BytePage],
    len: usize,
) -> io::Result<Option<BytePages>> {
    ctx.reset(ResetDirective::SessionOnly).map_err(zstd_error)?;
    // the frame stores the size, the receiver allocates it at once
    ctx.set_pledged_src_size(u64::try_from(len).ok())
        .map_err(zstd_error)?;
    // returns the bytes left to flush
    let mut step = |out: &mut BytePages, src: &mut InBuffer<'_>, end| {
        if out.len() >= len {
            return Ok(None);
        }
        fill(out, len - out.len(), |buf| {
            let mut buf = Uninit::new(buf);
            let mut dst = OutBuffer::around(&mut buf);
            let left = ctx
                .compress_stream2(&mut dst, src, end)
                .map_err(zstd_error)?;
            Ok((dst.pos(), Some(left)))
        })
    };

    let mut out = BytePages::default();
    for page in pages {
        let mut src = InBuffer::around(page);
        while src.pos() < page.len() {
            if step(&mut out, &mut src, ZSTD_EndDirective::ZSTD_e_continue)?.is_none() {
                return Ok(None);
            }
        }
    }
    let mut src = InBuffer::around(&[]);
    loop {
        match step(&mut out, &mut src, ZSTD_EndDirective::ZSTD_e_end)? {
            None => return Ok(None),
            Some(0) => return Ok(Some(out)),
            Some(_) => {}
        }
    }
}

fn zstd_error(code: usize) -> io::Error {
    io::Error::other(zstd::zstd_safe::get_error_name(code))
}

/// Reads at most `limit` bytes from `r` into a buffer of `hint` bytes, which
/// grows if the data does not fit.
fn read(r: impl Read, hint: usize, limit: usize) -> io::Result<BytesMut> {
    let mut out = BytesMut::with_capacity(hint);
    io::copy(&mut r.take(limit as u64), &mut out)?;
    Ok(out)
}

/// The gzip members of a message, read with a decoder the thread keeps as
/// `MultiGzDecoder` reads them.
struct Members<'a>(&'a mut GzDecoder<Cursor<Bytes>>);

impl<'a> Members<'a> {
    fn new(dec: &'a mut GzDecoder<Cursor<Bytes>>, data: &Bytes) -> Self {
        dec.reset(Cursor::new(data.clone()));
        Members(dec)
    }
}

impl Read for Members<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let n = self.0.read(buf)?;
            let input = self.0.get_ref();
            if n > 0 || buf.is_empty() || input.position() >= input.get_ref().len() as u64 {
                return Ok(n);
            }
            // the member is complete, the next one starts where it ends
            let input = input.clone();
            self.0.reset(input);
        }
    }
}

/// Decodes the zstd frames of `data` into a buffer of `hint` bytes, which
/// grows if the data does not fit, and stops after `limit` bytes.
///
/// zstd writes straight into the buffer. A frame that fits into it is
/// decoded in a single pass, without the window buffer of the decoder.
fn zstd_read(ctx: &mut DCtx<'_>, data: &[u8], hint: usize, limit: usize) -> io::Result<BytesMut> {
    let mut out = BytesMut::with_capacity(hint);
    let mut src = InBuffer::around(data);
    // the last frame is complete
    let mut done = false;
    while out.len() < limit && !(done && src.pos() == data.len()) {
        if out.len() == out.capacity() {
            out.reserve(8 * 1024);
        }
        let room = limit - out.len();
        let full = fill(&mut out, room, |buf| {
            let mut buf = Uninit::new(buf);
            let mut dst = OutBuffer::around(&mut buf);
            // a new frame starts after the previous one is complete
            done = ctx
                .decompress_stream(&mut dst, &mut src)
                .map_err(zstd_error)?
                == 0;
            Ok((dst.pos(), dst.pos() == dst.capacity()))
        })?;
        if !done && src.pos() == data.len() && !full {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete frame",
            ));
        }
    }
    Ok(out)
}

/// Lets `f` write into at most `room` bytes of the spare capacity of `out`,
/// the codecs write their output directly into it. `f` returns the number
/// of bytes it has written.
fn fill<B: BufMut, R>(
    out: &mut B,
    room: usize,
    f: impl FnOnce(&mut [MaybeUninit<u8>]) -> io::Result<(usize, R)>,
) -> io::Result<R> {
    // SAFETY: the codecs only write initialized bytes
    let spare = unsafe { out.chunk_mut().as_uninit_slice_mut() };
    let room = room.min(spare.len());
    let (n, res) = f(&mut spare[..room])?;
    assert!(n <= room);
    // SAFETY: `f` has written `n` bytes
    unsafe { out.advance_mut(n) };
    Ok(res)
}

/// A buffer of uninitialized bytes zstd writes into.
struct Uninit<'a> {
    buf: &'a mut [MaybeUninit<u8>],
    /// The bytes written.
    len: usize,
}

impl<'a> Uninit<'a> {
    fn new(buf: &'a mut [MaybeUninit<u8>]) -> Self {
        Uninit { buf, len: 0 }
    }
}

// SAFETY: `len` only covers the bytes zstd has written.
unsafe impl WriteBuf for Uninit<'_> {
    fn as_slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.buf.as_ptr().cast(), self.len) }
    }

    fn capacity(&self) -> usize {
        self.buf.len()
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.buf.as_mut_ptr().cast()
    }

    unsafe fn filled_until(&mut self, n: usize) {
        self.len = n;
    }
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
    use std::{io::Write, thread};

    use super::*;

    const ALL: [Compression; 2] = [Compression::Gzip, Compression::Zstd];

    /// Compresses `data` stored in 4 KiB pages, it must get smaller.
    async fn compress(enc: Compression, data: &[u8]) -> Bytes {
        let mut pages = BytePages::new(ntex_bytes::BytePageSize::Size4);
        pages.extend_from_slice(data);
        assert!(enc.compress(&mut pages).await.unwrap(), "{enc:?}");
        pages.freeze()
    }

    /// Compresses `data` however small it is, as another peer would.
    fn frame(enc: Compression, data: &[u8]) -> Bytes {
        match enc {
            Compression::Gzip => {
                let mut e =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                Bytes::from(e.finish().unwrap())
            }
            Compression::Zstd => Bytes::from(zstd::encode_all(data, 0).unwrap()),
        }
    }

    /// Pseudo-random bytes, they do not compress.
    fn random(len: usize) -> Vec<u8> {
        let mut x = 0x2545_f491_u32;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x.to_le_bytes()[0]
            })
            .collect()
    }

    /// Pseudo-random letters, they compress to about half.
    fn text(len: usize) -> Vec<u8> {
        random(len).into_iter().map(|b| b'a' + b % 16).collect()
    }

    #[ntex::test]
    async fn compress_pages() {
        let data: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        for enc in ALL {
            let mut pages = BytePages::new(ntex_bytes::BytePageSize::Size4);
            pages.extend_from_slice(&data);
            assert!(pages.num_pages() > 1);
            assert!(enc.compress(&mut pages).await.unwrap());
            assert!(pages.len() < data.len());
            assert_eq!(
                enc.decompress(pages.freeze(), data.len()).await.unwrap(),
                data,
                "{enc:?}"
            );
        }
    }

    #[ntex::test]
    async fn compress_into_pages() {
        // the output is written into pages, it is not one growing buffer
        let data = text(256 * 1024);
        for enc in ALL {
            let mut pages = BytePages::default();
            pages.extend_from_slice(&data);
            assert!(enc.compress(&mut pages).await.unwrap());
            assert!(pages.len() > 64 * 1024, "{enc:?} {}", pages.len());
            assert!(pages.num_pages() > 1, "{enc:?}");
            assert_eq!(
                enc.decompress(pages.freeze(), data.len()).await.unwrap(),
                data
            );
        }
    }

    #[ntex::test]
    async fn uncompressed() {
        for enc in ALL {
            // small messages
            for len in [0, 1, MIN_SIZE - 1] {
                let data = vec![b'a'; len];
                let mut pages = BytePages::default();
                pages.extend_from_slice(&data);
                let (res, used) = offloaded(enc.compress(&mut pages)).await;
                assert!(!res.unwrap());
                assert!(!used);
                assert_eq!(pages.freeze(), data);
            }
            assert!(
                enc.compress_sync(MIN_SIZE, &[BytePage::from(vec![b'a'; MIN_SIZE])])
                    .unwrap()
                    .is_some()
            );

            // messages that do not get smaller, in several pages
            for len in [MIN_SIZE, 4096, 20_000, 1024 * 1024] {
                let data = random(len);
                let mut pages = BytePages::new(ntex_bytes::BytePageSize::Size4);
                pages.extend_from_slice(&data);
                assert!(!enc.compress(&mut pages).await.unwrap(), "{enc:?} {len}");
                assert_eq!(pages.freeze(), data, "{enc:?} {len}");
            }
        }

        // gzip output as large as the input is not used, zstd needs the
        // real size of the input
        let pages = [BytePage::from(text(1000))];
        let gzip = |len| Compression::Gzip.compress_sync(len, &pages).unwrap();
        let size = gzip(1000).unwrap().len();
        assert!(gzip(size).is_none());
        assert_eq!(gzip(size + 1).unwrap().len(), size);
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
            let data = random(limit * 2);
            let compressed = frame(enc, &data);
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
            let compressed = frame(enc, b"");
            assert!(enc.decompress(compressed, 0).await.unwrap().is_empty());
        }
    }

    #[ntex::test]
    async fn members() {
        // concatenated gzip members and zstd frames are one message, as in Go
        for enc in ALL {
            let data = [frame(enc, b"abc"), frame(enc, b"def")].concat();
            assert_eq!(
                enc.decompress(Bytes::from(data.clone()), 6).await.unwrap(),
                b"abcdef"[..]
            );
            let (status, _) = enc.decompress(Bytes::from(data), 5).await.unwrap_err();
            assert_eq!(status, GrpcStatus::ResourceExhausted);
        }
    }

    #[ntex::test]
    async fn truncated() {
        for enc in ALL {
            let data = frame(enc, &text(1000));
            for len in [0, 1, data.len() / 2, data.len() - 1] {
                let (status, msg) = enc
                    .decompress(data.slice(..len), usize::MAX)
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
        let (_, msg) = Compression::Zstd
            .decompress(frame(Compression::Zstd, b"abc").slice(..5), usize::MAX)
            .await
            .unwrap_err();
        assert_eq!(
            msg,
            "grpc: failed to read decompressed data: incomplete frame"
        );
    }

    #[test]
    fn zstd_single_pass() {
        let data = text(1024 * 1024);
        let compressed = Compression::Zstd
            .compress_sync(data.len(), &[BytePage::from(data.clone())])
            .unwrap()
            .unwrap()
            .freeze();
        // the whole message fits, zstd decodes it without buffers of its own
        let mut ctx = DCtx::create();
        let size = ctx.sizeof();
        let out = zstd_read(&mut ctx, &compressed, data.len(), usize::MAX).unwrap();
        assert_eq!(out, data);
        assert_eq!(out.capacity(), data.len());
        assert_eq!(ctx.sizeof(), size);

        // it does not, zstd buffers the window
        for hint in [0, data.len() - 1] {
            let mut ctx = DCtx::create();
            let out = zstd_read(&mut ctx, &compressed, hint, usize::MAX).unwrap();
            assert_eq!(out, data);
            assert!(ctx.sizeof() > size + 512 * 1024, "{hint}");
        }
    }

    #[test]
    fn zstd_limit() {
        let data = text(100_000);
        let compressed = frame(Compression::Zstd, &data);
        for hint in [0, 1, 1000, 100_000] {
            for limit in [1, 1000, 9000, 99_999, 100_000, usize::MAX] {
                let out = zstd_read(&mut DCtx::create(), &compressed, hint, limit).unwrap();
                assert_eq!(out, data[..limit.min(data.len())], "{hint} {limit}");
            }
        }
    }

    #[test]
    fn zstd_window() {
        let data = text(2 * 1024 * 1024);
        let compressed = Compression::Zstd
            .compress_sync(data.len(), &[BytePage::from(data.clone())])
            .unwrap()
            .unwrap()
            .freeze();
        let mut dec = zstd::Decoder::with_buffer(&compressed[..]).unwrap();
        dec.window_log_max(ZSTD_WINDOW_LOG).unwrap();
        let mut out = Vec::new();
        dec.read_to_end(&mut out).unwrap();
        assert_eq!(out, data);
    }

    /// Drops the contexts the thread keeps.
    fn clear() {
        GZIP_ENCODER.take();
        GZIP_DECODER.take();
        ZSTD_ENCODER.take();
        ZSTD_DECODER.take();
    }

    /// Whether the thread keeps a context in `key`.
    fn kept<T>(key: &'static LocalKey<Cell<Option<T>>>) -> bool {
        let ctx = key.take();
        let kept = ctx.is_some();
        key.set(ctx);
        kept
    }

    /// Compresses `data` in pages of `size` bytes.
    fn compress_in(enc: Compression, data: &[u8], size: usize) -> Option<Bytes> {
        let pages: Vec<_> = data
            .chunks(size)
            .map(|page| BytePage::from(page.to_vec()))
            .collect();
        let out = enc.compress_sync(data.len(), &pages).unwrap();
        out.map(|mut out| out.freeze())
    }

    #[test]
    fn same_output() {
        // as the encoders of flate2 and zstd write it
        let zstd = |data: &[u8]| {
            let mut enc = zstd::Encoder::new(Vec::new(), 0).unwrap();
            enc.set_pledged_src_size(Some(data.len() as u64)).unwrap();
            enc.set_parameter(CParameter::WindowLog(ZSTD_WINDOW_LOG))
                .unwrap();
            enc.write_all(data).unwrap();
            enc.finish().unwrap()
        };
        let mut data = vec![b'a'; MIN_SIZE];
        for len in [1000, 4096, 4097, 100_000, 3 * 1024 * 1024] {
            data.push(b'b');
            data.extend_from_slice(&text(len));
            for size in [1, 100, 4096, usize::MAX] {
                let data = &data[..data.len().min(size.saturating_mul(1000))];
                let out = compress_in(Compression::Gzip, data, size).unwrap();
                assert_eq!(out, frame(Compression::Gzip, data), "{len} {size}");
                let out = compress_in(Compression::Zstd, data, size).unwrap();
                assert_eq!(out, zstd(data), "{len} {size}");
            }
        }
    }

    #[test]
    fn reuse() {
        for enc in ALL {
            clear();
            let data = text(1000);
            let first = compress_in(enc, &data, 100).unwrap();
            let encoders = (kept(&GZIP_ENCODER), kept(&ZSTD_ENCODER));
            assert_eq!(
                encoders,
                (enc == Compression::Gzip, enc == Compression::Zstd)
            );
            for _ in 0..3 {
                assert_eq!(compress_in(enc, &data, 100).unwrap(), first, "{enc:?}");
                // the frame is left open
                assert!(compress_in(enc, &random(5000), 100).is_none());
            }
            assert_eq!(compress_in(enc, &data, 100).unwrap(), first, "{enc:?}");

            for _ in 0..3 {
                let out = enc.decompress_sync(&first, usize::MAX).unwrap();
                assert_eq!(out, data, "{enc:?}");
                // errors in the middle of the message
                let half = first.slice(..first.len() / 2);
                assert!(enc.decompress_sync(&half, usize::MAX).is_err());
                let bad = Bytes::from([&first[..], b"abc"].concat());
                assert!(enc.decompress_sync(&bad, usize::MAX).is_err());
                assert!(enc.decompress_sync(&first, 10).is_err());
            }
            let out = enc.decompress_sync(&first, usize::MAX).unwrap();
            assert_eq!(out, data, "{enc:?}");
            let decoders = (kept(&GZIP_DECODER), kept(&ZSTD_DECODER));
            assert_eq!(
                decoders,
                (enc == Compression::Gzip, enc == Compression::Zstd)
            );
        }
        // the decoder does not keep the message
        clear();
        let compressed = frame(Compression::Gzip, &text(1000));
        Compression::Gzip
            .decompress_sync(&compressed, usize::MAX)
            .unwrap();
        let dec = GZIP_DECODER.take().unwrap();
        assert!(dec.get_ref().get_ref().is_empty());
    }

    #[test]
    fn reuse_memory() {
        let data = text(1000);
        for enc in ALL {
            clear();
            let compress = || compress_in(enc, &data, usize::MAX).unwrap();
            // the output page is reused once it is dropped
            let cold = counting::peak(compress).1;
            let (compressed, warm) = counting::peak(compress);
            let decompress = || enc.decompress_sync(&compressed, usize::MAX).unwrap();
            let cold_dec = counting::peak(decompress).1;
            let warm_dec = counting::peak(decompress).1;
            assert!(warm < 2 * 1024, "{enc:?} {warm}");
            assert!(warm_dec < 2 * 1024, "{enc:?} {warm_dec}");
            if enc == Compression::Gzip {
                assert!(
                    cold > 128 * 1024 && cold_dec > 32 * 1024,
                    "{cold} {cold_dec}"
                );
            }
        }
        // zstd allocates its contexts itself
        let encoder = ZSTD_ENCODER.take().unwrap().sizeof();
        let decoder = ZSTD_DECODER.take().unwrap().sizeof();
        assert!(
            encoder < 64 * 1024 && decoder < 128 * 1024,
            "{encoder} {decoder}"
        );
    }

    #[test]
    fn reuse_limit() {
        clear();
        // a context that has grown is dropped
        let data = text(64 * 1024);
        let compressed = compress_in(Compression::Zstd, &data, usize::MAX).unwrap();
        assert!(!kept(&ZSTD_ENCODER));
        compress_in(Compression::Zstd, &text(8 * 1024), usize::MAX).unwrap();
        let ctx = ZSTD_ENCODER.take().unwrap();
        assert!(ctx.sizeof() < CONTEXT_LIMIT, "{}", ctx.sizeof());
        ZSTD_ENCODER.set(Some(ctx));

        // the message fits into the output, zstd needs no window
        let out = Compression::Zstd
            .decompress_sync(&compressed, usize::MAX)
            .unwrap();
        assert_eq!(out, data);
        assert!(kept(&ZSTD_DECODER));
        // it does not, the decoder buffers the window
        let compressed = frame(Compression::Zstd, &data);
        let out = Compression::Zstd
            .decompress_sync(&compressed, usize::MAX)
            .unwrap();
        assert_eq!(out, data);
        assert!(!kept(&ZSTD_DECODER));

        // gzip contexts do not grow
        let data = text(1024 * 1024);
        let compressed = compress_in(Compression::Gzip, &data, usize::MAX).unwrap();
        let out = Compression::Gzip
            .decompress_sync(&compressed, usize::MAX)
            .unwrap();
        assert_eq!(out, data);
        assert!(kept(&GZIP_ENCODER) && kept(&GZIP_DECODER));
    }

    #[test]
    fn gzip_members() {
        // the same as MultiGzDecoder
        let (abc, def) = (
            frame(Compression::Gzip, b"abc"),
            frame(Compression::Gzip, b"def"),
        );
        let both = [&abc[..], &def[..]].concat();
        let cases = [
            Vec::new(),
            abc.to_vec(),
            both.clone(),
            [&both[..], &abc[..]].concat(),
            [&abc[..], b"x"].concat(),
            abc[..abc.len() - 1].to_vec(),
            both[..both.len() - 1].to_vec(),
            [&abc[..], &[0; 10]].concat(),
            vec![0x1f, 0x8b],
        ];
        let mut dec = GzDecoder::new(Cursor::default());
        for data in cases {
            let mut expected = Vec::new();
            let res = flate2::bufread::MultiGzDecoder::new(&data[..]).read_to_end(&mut expected);
            let mut out = Vec::new();
            let data = Bytes::from(data);
            let mut members = Members::new(&mut dec, &data);
            match res {
                Ok(_) => {
                    members.read_to_end(&mut out).unwrap();
                    assert_eq!(out, expected, "{data:?}");
                }
                Err(err) => {
                    let res = members.read_to_end(&mut out).unwrap_err();
                    assert_eq!(res.kind(), err.kind(), "{data:?}");
                }
            }
        }
        // an empty read does not move to the next member
        let data = Bytes::from(both);
        let mut members = Members::new(&mut dec, &data);
        let mut out = [0; 3];
        members.read_exact(&mut out).unwrap();
        assert_eq!(members.read(&mut []).unwrap(), 0);
        let mut out = Vec::new();
        members.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"def");
    }

    #[test]
    fn gzip_memory() {
        let data = text(1000);
        let compressed = frame(Compression::Gzip, &data);
        let (out, peak) = counting::peak(|| {
            Compression::Gzip
                .decompress_sync(&compressed, usize::MAX)
                .unwrap()
        });
        assert_eq!(out, data);
        // the inflate state, without a copy of the input
        assert!(peak < 64 * 1024, "{peak}");
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
            let data = text(10_000);
            let compressed = compress(enc, &data).await;
            assert_eq!(enc.size_hint(&compressed, usize::MAX), data.len());
            assert_eq!(enc.size_hint(&compressed, 100), 100);
            assert_eq!(enc.size_hint(b"", usize::MAX), 0);

            // only 64 times the compressed size is trusted
            let data = vec![b'a'; 100_000];
            let compressed = compress(enc, &data).await;
            assert_eq!(
                enc.size_hint(&compressed, usize::MAX),
                compressed.len() * 64
            );
            assert_eq!(enc.decompress(compressed, data.len()).await.unwrap(), data);
        }
        // zstd stores the size in the frame
        let compressed = compress(Compression::Zstd, &[b'a'; 100]).await;
        assert_eq!(
            zstd::zstd_safe::get_frame_content_size(&compressed).ok(),
            Some(Some(100))
        );
        // the size of a small message is limited
        let data = [1, 2, 3, 4, 0xff, 0xff, 0xff, 0xff];
        assert_eq!(Compression::Gzip.size_hint(&data, usize::MAX), 8 * 64);
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

    /// Counts the memory the current thread allocates.
    mod counting {
        use std::alloc::{GlobalAlloc, Layout, System};
        use std::cell::Cell;

        thread_local! {
            static USED: Cell<isize> = const { Cell::new(0) };
            static PEAK: Cell<isize> = const { Cell::new(0) };
        }

        struct Counting;

        #[global_allocator]
        static ALLOC: Counting = Counting;

        fn add(size: usize, sub: usize) {
            let _ = USED.try_with(|used| {
                let n = used.get() + size.cast_signed() - sub.cast_signed();
                used.set(n);
                let _ = PEAK.try_with(|peak| peak.set(peak.get().max(n)));
            });
        }

        // SAFETY: the calls are passed to the system allocator.
        unsafe impl GlobalAlloc for Counting {
            unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                add(layout.size(), 0);
                unsafe { System.alloc(layout) }
            }

            unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
                add(layout.size(), 0);
                unsafe { System.alloc_zeroed(layout) }
            }

            unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
                add(0, layout.size());
                unsafe { System.dealloc(ptr, layout) }
            }

            unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
                add(size, layout.size());
                unsafe { System.realloc(ptr, layout, size) }
            }
        }

        /// Returns the result of `f` and the most memory it held at once.
        pub(super) fn peak<R>(f: impl FnOnce() -> R) -> (R, usize) {
            let base = USED.get();
            PEAK.set(base);
            let res = f();
            (res, (PEAK.get() - base).cast_unsigned())
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
