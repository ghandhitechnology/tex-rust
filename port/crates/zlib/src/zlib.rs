//! Thin zlib-named wrappers for straight ports of C callers.
//!
//! `z_stream` keeps C's raw `next_in`/`next_out` pointers, so the stream
//! functions that read or write through them are `unsafe`: the caller must
//! guarantee `next_in` points to `avail_in` readable bytes and `next_out` to
//! `avail_out` writable bytes, exactly as in C. Everything else is safe.
//!
//! `uLong` is the platform's C `unsigned long` (32 bits on Windows, 64 bits
//! on 64-bit Unix), as in C, so `total_in`/`total_out` wrap and
//! `deflateBound()`/`compressBound()` saturate exactly like the C library.

#![allow(non_camel_case_types, non_snake_case)]

use std::fs::File;
use std::slice;

use crate::deflate::{bound_no_state, DeflateState, Io, DEF_MEM_LEVEL, MAX_WBITS};
use crate::gz::{GzReader, GzWriter};
use crate::inflate::InflateState;
use crate::{err_msg, GzHeader};

pub use crate::adler32::{adler32, adler32_combine, adler32_z};
pub use crate::crc32::{crc32, crc32_combine, crc32_combine_gen, crc32_combine_op, crc32_z};
pub use crate::deflate::MAX_MEM_LEVEL;
pub use crate::{
    Z_ASCII, Z_BEST_COMPRESSION, Z_BEST_SPEED, Z_BINARY, Z_BLOCK, Z_BUF_ERROR, Z_DATA_ERROR,
    Z_DEFAULT_COMPRESSION, Z_DEFAULT_STRATEGY, Z_DEFLATED, Z_ERRNO, Z_FILTERED, Z_FINISH, Z_FIXED,
    Z_FULL_FLUSH, Z_HUFFMAN_ONLY, Z_MEM_ERROR, Z_NEED_DICT, Z_NO_COMPRESSION, Z_NO_FLUSH, Z_OK,
    Z_PARTIAL_FLUSH, Z_RLE, Z_STREAM_END, Z_STREAM_ERROR, Z_SYNC_FLUSH, Z_TEXT, Z_TREES, Z_UNKNOWN,
    Z_VERSION_ERROR, ZLIB_VERNUM, ZLIB_VERSION,
};

/// Internal state attached to a `z_stream` (opaque).
#[derive(Clone)]
pub struct ZState(Inner);

#[derive(Clone)]
enum Inner {
    Deflate(Box<DeflateState>),
    Inflate(Box<InflateState>),
}

/// `z_stream`
pub struct z_stream {
    pub next_in: *const u8,
    pub avail_in: uInt,
    pub total_in: uLong,
    pub next_out: *mut u8,
    pub avail_out: uInt,
    pub total_out: uLong,
    pub msg: Option<&'static str>,
    pub state: Option<ZState>,
    pub data_type: i32,
    pub adler: uLong,
    pub reserved: uLong,
}

impl Default for z_stream {
    fn default() -> Self {
        z_stream {
            next_in: std::ptr::null(),
            avail_in: 0,
            total_in: 0,
            next_out: std::ptr::null_mut(),
            avail_out: 0,
            total_out: 0,
            msg: None,
            state: None,
            data_type: 0,
            adler: 0,
            reserved: 0,
        }
    }
}

pub type z_streamp<'a> = &'a mut z_stream;
/// C `uLong` (`unsigned long`).
pub type uLong = std::os::raw::c_ulong;
/// C `uInt`.
pub type uInt = u32;

/// `(uLong)bound != bound ? (uLong)-1 : (uLong)bound`
pub fn ulong_saturate(v: u64) -> uLong {
    if v as uLong as u64 != v {
        uLong::MAX
    } else {
        v as uLong
    }
}
pub type gz_header = GzHeader;

macro_rules! sync_in {
    ($strm:expr, $s:expr) => {{
        $s.total_in = $strm.total_in as u64;
        $s.total_out = $strm.total_out as u64;
        $s.adler = $strm.adler as u32;
        $s.data_type = $strm.data_type;
        $s.msg = $strm.msg;
    }};
}
macro_rules! sync_out {
    ($strm:expr, $s:expr) => {{
        $strm.total_in = $s.total_in as uLong;
        $strm.total_out = $s.total_out as uLong;
        $strm.adler = $s.adler as uLong;
        $strm.data_type = $s.data_type;
        $strm.msg = $s.msg;
    }};
}

/// Run `f` with an `Io` built from the stream's pointers, then advance the
/// pointers. Null pointers with zero length give empty slices.
unsafe fn with_io<R>(strm: &mut z_stream, f: impl FnOnce(&mut Io) -> R) -> R {
    let input: &[u8] = if strm.avail_in == 0 || strm.next_in.is_null() {
        &[]
    } else {
        slice::from_raw_parts(strm.next_in, strm.avail_in as usize)
    };
    let output: &mut [u8] = if strm.avail_out == 0 || strm.next_out.is_null() {
        &mut []
    } else {
        slice::from_raw_parts_mut(strm.next_out, strm.avail_out as usize)
    };
    let mut io = Io::new(input, output);
    let r = f(&mut io);
    if !strm.next_in.is_null() {
        strm.next_in = strm.next_in.add(io.next_in);
    }
    strm.avail_in -= io.next_in as u32;
    if !strm.next_out.is_null() {
        strm.next_out = strm.next_out.add(io.next_out);
    }
    strm.avail_out -= io.next_out as u32;
    r
}

fn dstate(strm: &mut z_stream) -> Option<&mut DeflateState> {
    match strm.state.as_mut() {
        Some(ZState(Inner::Deflate(s))) => Some(s),
        _ => None,
    }
}

fn istate(strm: &mut z_stream) -> Option<&mut InflateState> {
    match strm.state.as_mut() {
        Some(ZState(Inner::Inflate(s))) => Some(s),
        _ => None,
    }
}

// ---------------------------------------------------------------- deflate

/// `deflateInit()`
pub fn deflateInit(strm: &mut z_stream, level: i32) -> i32 {
    deflateInit2(strm, level, Z_DEFLATED, MAX_WBITS, DEF_MEM_LEVEL, Z_DEFAULT_STRATEGY)
}

/// `deflateInit2()`
pub fn deflateInit2(
    strm: &mut z_stream,
    level: i32,
    method: i32,
    windowBits: i32,
    memLevel: i32,
    strategy: i32,
) -> i32 {
    strm.msg = None;
    match DeflateState::init2(level, method, windowBits, memLevel, strategy) {
        Ok(s) => {
            sync_out!(strm, s);
            strm.state = Some(ZState(Inner::Deflate(s)));
            Z_OK
        }
        Err(e) => e,
    }
}

/// `deflate()`
///
/// # Safety
/// `next_in`/`avail_in` and `next_out`/`avail_out` must describe valid
/// buffers, as in C.
pub unsafe fn deflate(strm: &mut z_stream, flush: i32) -> i32 {
    if dstate(strm).is_none() || !(0..=Z_BLOCK).contains(&flush) {
        return Z_STREAM_ERROR;
    }
    if strm.next_out.is_null() || (strm.avail_in != 0 && strm.next_in.is_null()) {
        strm.msg = Some(err_msg(Z_STREAM_ERROR));
        return Z_STREAM_ERROR;
    }
    let mut st = strm.state.take();
    let s = match st.as_mut() {
        Some(ZState(Inner::Deflate(s))) => s,
        _ => unreachable!(),
    };
    sync_in!(strm, s);
    let r = with_io(strm, |io| s.deflate(io, flush));
    sync_out!(strm, s);
    strm.state = st;
    r
}

/// `deflateEnd()`
pub fn deflateEnd(strm: &mut z_stream) -> i32 {
    match strm.state.take() {
        Some(ZState(Inner::Deflate(s))) => s.end_status(),
        other => {
            strm.state = other;
            Z_STREAM_ERROR
        }
    }
}

/// `deflateReset()`
pub fn deflateReset(strm: &mut z_stream) -> i32 {
    let Some(ZState(Inner::Deflate(s))) = strm.state.as_mut() else {
        return Z_STREAM_ERROR;
    };
    s.reset();
    let s = s.clone();
    sync_out!(strm, s);
    Z_OK
}

/// `deflateResetKeep()`
pub fn deflateResetKeep(strm: &mut z_stream) -> i32 {
    let Some(ZState(Inner::Deflate(s))) = strm.state.as_mut() else {
        return Z_STREAM_ERROR;
    };
    s.reset_keep();
    let (ti, to, a, d, m) = (s.total_in, s.total_out, s.adler, s.data_type, s.msg);
    strm.total_in = ti as uLong;
    strm.total_out = to as uLong;
    strm.adler = a as uLong;
    strm.data_type = d;
    strm.msg = m;
    Z_OK
}

/// `deflateParams()`
///
/// # Safety
/// As for [`deflate`]: it may call `deflate(strm, Z_BLOCK)`.
pub unsafe fn deflateParams(strm: &mut z_stream, level: i32, strategy: i32) -> i32 {
    let mut st = strm.state.take();
    let Some(ZState(Inner::Deflate(s))) = st.as_mut() else {
        strm.state = st;
        return Z_STREAM_ERROR;
    };
    sync_in!(strm, s);
    let r = with_io(strm, |io| s.params(io, level, strategy));
    sync_out!(strm, s);
    strm.state = st;
    r
}

/// `deflateTune()`
pub fn deflateTune(strm: &mut z_stream, good: i32, lazy: i32, nice: i32, chain: i32) -> i32 {
    match dstate(strm) {
        Some(s) => {
            s.tune(good, lazy, nice, chain);
            Z_OK
        }
        None => Z_STREAM_ERROR,
    }
}

/// `deflateBound()` (`None` for `Z_NULL`).
pub fn deflateBound(strm: Option<&z_stream>, sourceLen: uLong) -> uLong {
    ulong_saturate(deflateBound_z(strm, sourceLen as u64))
}

/// `deflateBound_z()`
pub fn deflateBound_z(strm: Option<&z_stream>, sourceLen: u64) -> u64 {
    match strm.and_then(|s| s.state.as_ref()) {
        Some(ZState(Inner::Deflate(s))) => s.bound(sourceLen),
        _ => bound_no_state(sourceLen),
    }
}

/// `deflateSetDictionary()`
pub fn deflateSetDictionary(strm: &mut z_stream, dictionary: &[u8]) -> i32 {
    let mut st = strm.state.take();
    let Some(ZState(Inner::Deflate(s))) = st.as_mut() else {
        strm.state = st;
        return Z_STREAM_ERROR;
    };
    sync_in!(strm, s);
    let r = s.set_dictionary(dictionary);
    sync_out!(strm, s);
    strm.state = st;
    r
}

/// `deflateGetDictionary()`: returns the dictionary bytes.
pub fn deflateGetDictionary(strm: &mut z_stream) -> Result<Vec<u8>, i32> {
    dstate(strm).map(|s| s.get_dictionary()).ok_or(Z_STREAM_ERROR)
}

/// `deflatePending()`
pub fn deflatePending(strm: &mut z_stream, pending: Option<&mut u32>, bits: Option<&mut i32>) -> i32 {
    let Some(s) = dstate(strm) else {
        return Z_STREAM_ERROR;
    };
    if let Some(b) = bits {
        *b = s.bi_valid;
    }
    if let Some(p) = pending {
        *p = s.pending as u32;
        if *p as usize != s.pending {
            *p = u32::MAX;
            return Z_BUF_ERROR;
        }
    }
    Z_OK
}

/// `deflateUsed()`
pub fn deflateUsed(strm: &mut z_stream, bits: Option<&mut i32>) -> i32 {
    let Some(s) = dstate(strm) else {
        return Z_STREAM_ERROR;
    };
    if let Some(b) = bits {
        *b = s.bi_used;
    }
    Z_OK
}

/// `deflatePrime()`
pub fn deflatePrime(strm: &mut z_stream, bits: i32, value: i32) -> i32 {
    dstate(strm).map_or(Z_STREAM_ERROR, |s| s.prime(bits, value))
}

/// `deflateSetHeader()`
pub fn deflateSetHeader(strm: &mut z_stream, head: GzHeader) -> i32 {
    match dstate(strm) {
        Some(s) if s.wrap == 2 => {
            s.gzhead = Some(head);
            Z_OK
        }
        _ => Z_STREAM_ERROR,
    }
}

/// `deflateCopy()`
pub fn deflateCopy(dest: &mut z_stream, source: &z_stream) -> i32 {
    match source.state.as_ref() {
        Some(ZState(Inner::Deflate(s))) => {
            *dest = z_stream {
                next_in: source.next_in,
                avail_in: source.avail_in,
                total_in: source.total_in,
                next_out: source.next_out,
                avail_out: source.avail_out,
                total_out: source.total_out,
                msg: source.msg,
                state: Some(ZState(Inner::Deflate(s.clone()))),
                data_type: source.data_type,
                adler: source.adler,
                reserved: source.reserved,
            };
            Z_OK
        }
        _ => Z_STREAM_ERROR,
    }
}

// ---------------------------------------------------------------- inflate

/// `inflateInit()`
pub fn inflateInit(strm: &mut z_stream) -> i32 {
    inflateInit2(strm, 15)
}

/// `inflateInit2()`
pub fn inflateInit2(strm: &mut z_stream, windowBits: i32) -> i32 {
    strm.msg = None;
    match InflateState::init2(windowBits) {
        Ok(s) => {
            sync_out!(strm, s);
            strm.state = Some(ZState(Inner::Inflate(s)));
            Z_OK
        }
        Err(e) => e,
    }
}

/// `inflate()`
///
/// # Safety
/// `next_in`/`avail_in` and `next_out`/`avail_out` must describe valid
/// buffers, as in C.
pub unsafe fn inflate(strm: &mut z_stream, flush: i32) -> i32 {
    if istate(strm).is_none()
        || strm.next_out.is_null()
        || (strm.next_in.is_null() && strm.avail_in != 0)
    {
        return Z_STREAM_ERROR;
    }
    let mut st = strm.state.take();
    let s = match st.as_mut() {
        Some(ZState(Inner::Inflate(s))) => s,
        _ => unreachable!(),
    };
    sync_in!(strm, s);
    let r = with_io(strm, |io| s.inflate(io, flush));
    sync_out!(strm, s);
    strm.state = st;
    r
}

/// `inflateEnd()`
pub fn inflateEnd(strm: &mut z_stream) -> i32 {
    match strm.state.take() {
        Some(ZState(Inner::Inflate(_))) => Z_OK,
        other => {
            strm.state = other;
            Z_STREAM_ERROR
        }
    }
}

fn inflate_call(strm: &mut z_stream, f: impl FnOnce(&mut InflateState) -> i32) -> i32 {
    let mut st = strm.state.take();
    let Some(ZState(Inner::Inflate(s))) = st.as_mut() else {
        strm.state = st;
        return Z_STREAM_ERROR;
    };
    sync_in!(strm, s);
    let r = f(s);
    sync_out!(strm, s);
    strm.state = st;
    r
}

/// `inflateReset()`
pub fn inflateReset(strm: &mut z_stream) -> i32 {
    inflate_call(strm, |s| s.reset())
}
/// `inflateResetKeep()`
pub fn inflateResetKeep(strm: &mut z_stream) -> i32 {
    inflate_call(strm, |s| s.reset_keep())
}
/// `inflateReset2()`
pub fn inflateReset2(strm: &mut z_stream, windowBits: i32) -> i32 {
    inflate_call(strm, |s| s.reset2(windowBits))
}
/// `inflateSetDictionary()`
pub fn inflateSetDictionary(strm: &mut z_stream, dictionary: &[u8]) -> i32 {
    inflate_call(strm, |s| s.set_dictionary(dictionary))
}
/// `inflateGetDictionary()`
pub fn inflateGetDictionary(strm: &mut z_stream) -> Result<Vec<u8>, i32> {
    istate(strm).map(|s| s.get_dictionary()).ok_or(Z_STREAM_ERROR)
}
/// `inflatePrime()`
pub fn inflatePrime(strm: &mut z_stream, bits: i32, value: i32) -> i32 {
    inflate_call(strm, |s| s.prime(bits, value))
}
/// `inflateSyncPoint()`
pub fn inflateSyncPoint(strm: &mut z_stream) -> i32 {
    istate(strm).map_or(Z_STREAM_ERROR, |s| s.sync_point())
}
/// `inflateMark()` (C `long`).
pub fn inflateMark(strm: &mut z_stream) -> std::os::raw::c_long {
    istate(strm).map_or(-(1 << 16), |s| s.mark() as std::os::raw::c_long)
}
/// `inflateCodesUsed()`
pub fn inflateCodesUsed(strm: &mut z_stream) -> uLong {
    istate(strm).map_or(uLong::MAX, |s| s.next as uLong)
}
/// `inflateValidate()`
pub fn inflateValidate(strm: &mut z_stream, check: i32) -> i32 {
    inflate_call(strm, |s| {
        s.validate(check != 0);
        Z_OK
    })
}
/// `inflateUndermine()`
pub fn inflateUndermine(strm: &mut z_stream, _subvert: i32) -> i32 {
    inflate_call(strm, |s| {
        s.sane = true;
        Z_DATA_ERROR
    })
}
/// `inflateGetHeader()`
pub fn inflateGetHeader(strm: &mut z_stream, mut head: GzHeader) -> i32 {
    inflate_call(strm, |s| {
        if (s.wrap & 2) == 0 {
            return Z_STREAM_ERROR;
        }
        head.done = 0;
        s.head = Some(head);
        Z_OK
    })
}
/// The header registered with `inflateGetHeader()`.
pub fn inflateHeader(strm: &z_stream) -> Option<&GzHeader> {
    match strm.state.as_ref() {
        Some(ZState(Inner::Inflate(s))) => s.head.as_ref(),
        _ => None,
    }
}

/// `inflateSync()`
///
/// # Safety
/// `next_in`/`avail_in` must describe a valid buffer.
pub unsafe fn inflateSync(strm: &mut z_stream) -> i32 {
    let mut st = strm.state.take();
    let Some(ZState(Inner::Inflate(s))) = st.as_mut() else {
        strm.state = st;
        return Z_STREAM_ERROR;
    };
    sync_in!(strm, s);
    let r = with_io(strm, |io| s.sync(io));
    sync_out!(strm, s);
    strm.state = st;
    r
}

/// `inflateCopy()`
pub fn inflateCopy(dest: &mut z_stream, source: &z_stream) -> i32 {
    match source.state.as_ref() {
        Some(ZState(Inner::Inflate(s))) => {
            *dest = z_stream {
                next_in: source.next_in,
                avail_in: source.avail_in,
                total_in: source.total_in,
                next_out: source.next_out,
                avail_out: source.avail_out,
                total_out: source.total_out,
                msg: source.msg,
                state: Some(ZState(Inner::Inflate(s.clone()))),
                data_type: source.data_type,
                adler: source.adler,
                reserved: source.reserved,
            };
            Z_OK
        }
        _ => Z_STREAM_ERROR,
    }
}

// --------------------------------------------------------- utility functions

/// `compress()`: `*destLen` is the size of `dest` on entry and the
/// compressed size on return.
pub fn compress(dest: &mut [u8], destLen: &mut uLong, source: &[u8]) -> i32 {
    compress2(dest, destLen, source, Z_DEFAULT_COMPRESSION)
}

/// `compress2()`
pub fn compress2(dest: &mut [u8], destLen: &mut uLong, source: &[u8], level: i32) -> i32 {
    let n = (*destLen as usize).min(dest.len());
    let (r, got) = crate::compress::compress2(&mut dest[..n], source, level);
    *destLen = got as uLong;
    r
}

/// `compressBound()`
pub fn compressBound(sourceLen: uLong) -> uLong {
    ulong_saturate(crate::compress::compress_bound(sourceLen as u64))
}

/// `compressBound_z()`
pub fn compressBound_z(sourceLen: u64) -> u64 {
    crate::compress::compress_bound(sourceLen)
}

/// `uncompress()`
pub fn uncompress(dest: &mut [u8], destLen: &mut uLong, source: &[u8]) -> i32 {
    let mut used = source.len() as uLong;
    uncompress2(dest, destLen, source, &mut used)
}

/// `uncompress2()`: `*sourceLen` is the input length on entry and the
/// number of bytes consumed on return.
pub fn uncompress2(dest: &mut [u8], destLen: &mut uLong, source: &[u8], sourceLen: &mut uLong) -> i32 {
    let n = (*destLen as usize).min(dest.len());
    let m = (*sourceLen as usize).min(source.len());
    let (r, d, s) = crate::compress::uncompress2(&mut dest[..n], &source[..m]);
    *destLen = d as uLong;
    *sourceLen = s as uLong;
    r
}

/// `zError()`
pub fn zError(err: i32) -> &'static str {
    err_msg(err)
}

/// `zlibVersion()`
pub fn zlibVersion() -> &'static str {
    ZLIB_VERSION
}

// ------------------------------------------------------------- gzip files

/// The state behind a `gzFile`.
pub enum gzFile_s {
    Read(GzReader<File>),
    Write(GzWriter<File>),
}
/// `gzFile`
pub type gzFile = Box<gzFile_s>;

/// `gzopen()`
pub fn gzopen(path: &str, mode: &str) -> Option<gzFile> {
    if mode.contains('r') && !mode.contains('w') && !mode.contains('a') {
        let f = File::open(path).ok()?;
        GzReader::with_mode(f, mode, path).map(|r| Box::new(gzFile_s::Read(r)))
    } else {
        GzWriter::create(path, mode).ok().map(|w| Box::new(gzFile_s::Write(w)))
    }
}

/// `gzdopen()` on an open file (instead of a descriptor). The name used
/// in error messages is `"<fd:N>"` in C; here it is `"<file>"`.
pub fn gzdopen(file: File, mode: &str) -> Option<gzFile> {
    let reading = mode.contains('r');
    if reading {
        GzReader::with_mode(file, mode, "<file>").map(|r| Box::new(gzFile_s::Read(r)))
    } else {
        GzWriter::with_mode(file, mode, "<file>").map(|w| Box::new(gzFile_s::Write(w)))
    }
}

/// `gzbuffer()`
pub fn gzbuffer(file: &mut gzFile, size: u32) -> i32 {
    match &mut **file {
        gzFile_s::Read(r) => r.gzbuffer(size),
        gzFile_s::Write(w) => w.gzbuffer(size),
    }
}

/// `gzsetparams()`
pub fn gzsetparams(file: &mut gzFile, level: i32, strategy: i32) -> i32 {
    match &mut **file {
        gzFile_s::Write(w) => w.gzsetparams(level, strategy),
        gzFile_s::Read(_) => Z_STREAM_ERROR,
    }
}

/// `gzread()`
pub fn gzread(file: &mut gzFile, buf: &mut [u8]) -> i32 {
    match &mut **file {
        gzFile_s::Read(r) => r.gzread(buf),
        gzFile_s::Write(_) => -1,
    }
}

/// `gzfread()`
pub fn gzfread(buf: &mut [u8], size: usize, nitems: usize, file: &mut gzFile) -> usize {
    match &mut **file {
        gzFile_s::Read(r) => r.gzfread(buf, size, nitems),
        gzFile_s::Write(_) => 0,
    }
}

/// `gzwrite()`
pub fn gzwrite(file: &mut gzFile, buf: &[u8]) -> i32 {
    match &mut **file {
        gzFile_s::Write(w) => w.gzwrite(buf),
        gzFile_s::Read(_) => 0,
    }
}

/// `gzfwrite()`
pub fn gzfwrite(buf: &[u8], size: usize, nitems: usize, file: &mut gzFile) -> usize {
    match &mut **file {
        gzFile_s::Write(w) => w.gzfwrite(buf, size, nitems),
        gzFile_s::Read(_) => 0,
    }
}

/// `gzprintf()` with a preformatted string.
pub fn gzprintf(file: &mut gzFile, text: &str) -> i32 {
    match &mut **file {
        gzFile_s::Write(w) => w.gzprintf(text),
        gzFile_s::Read(_) => Z_STREAM_ERROR,
    }
}

/// `gzputs()`
pub fn gzputs(file: &mut gzFile, s: &[u8]) -> i32 {
    match &mut **file {
        gzFile_s::Write(w) => w.gzputs(s),
        gzFile_s::Read(_) => -1,
    }
}

/// `gzgets()`
pub fn gzgets(file: &mut gzFile, buf: &mut [u8]) -> Option<usize> {
    match &mut **file {
        gzFile_s::Read(r) => r.gzgets(buf),
        gzFile_s::Write(_) => None,
    }
}

/// `gzputc()`
pub fn gzputc(file: &mut gzFile, c: i32) -> i32 {
    match &mut **file {
        gzFile_s::Write(w) => w.gzputc(c),
        gzFile_s::Read(_) => -1,
    }
}

/// `gzgetc()`
pub fn gzgetc(file: &mut gzFile) -> i32 {
    match &mut **file {
        gzFile_s::Read(r) => r.gzgetc(),
        gzFile_s::Write(_) => -1,
    }
}

/// `gzungetc()`
pub fn gzungetc(c: i32, file: &mut gzFile) -> i32 {
    match &mut **file {
        gzFile_s::Read(r) => r.gzungetc(c),
        gzFile_s::Write(_) => -1,
    }
}

/// `gzflush()`
pub fn gzflush(file: &mut gzFile, flush: i32) -> i32 {
    match &mut **file {
        gzFile_s::Write(w) => w.gzflush(flush),
        gzFile_s::Read(_) => Z_STREAM_ERROR,
    }
}

/// `gzseek()` (whence 0 = SEEK_SET, 1 = SEEK_CUR).
pub fn gzseek(file: &mut gzFile, offset: i64, whence: i32) -> i64 {
    match &mut **file {
        gzFile_s::Write(w) => w.gzseek(offset, whence),
        gzFile_s::Read(r) => r.gzseek(offset, whence),
    }
}

/// `gzrewind()`
pub fn gzrewind(file: &mut gzFile) -> i32 {
    match &mut **file {
        gzFile_s::Read(r) => r.gzrewind(),
        gzFile_s::Write(_) => -1,
    }
}

/// `gztell()`
pub fn gztell(file: &gzFile) -> i64 {
    match &**file {
        gzFile_s::Write(w) => w.gztell(),
        gzFile_s::Read(r) => r.gztell(),
    }
}

/// `gzeof()`
pub fn gzeof(file: &gzFile) -> i32 {
    match &**file {
        gzFile_s::Read(r) => r.gzeof() as i32,
        gzFile_s::Write(_) => 0,
    }
}

/// `gzdirect()`
pub fn gzdirect(file: &mut gzFile) -> i32 {
    match &mut **file {
        gzFile_s::Read(r) => r.gzdirect() as i32,
        gzFile_s::Write(_) => 0,
    }
}

/// `gzerror()`: (message, errnum).
pub fn gzerror(file: &gzFile) -> (String, i32) {
    match &**file {
        gzFile_s::Read(r) => r.gzerror(),
        gzFile_s::Write(w) => w.gzerror(),
    }
}

/// `gzclearerr()`
pub fn gzclearerr(file: &mut gzFile) {
    match &mut **file {
        gzFile_s::Read(r) => r.gzclearerr(),
        gzFile_s::Write(w) => w.gzclearerr(),
    }
}

/// `gzclose()`
pub fn gzclose(file: gzFile) -> i32 {
    match *file {
        gzFile_s::Read(r) => r.gzclose(),
        gzFile_s::Write(w) => w.gzclose(),
    }
}
