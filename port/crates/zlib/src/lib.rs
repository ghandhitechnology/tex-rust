//! Pure-Rust port of zlib 1.3.2, as bundled with TeX Live 2026.
//!
//! The goal is byte-identical behaviour with the C library: the same input
//! and the same sequence of calls (including the input and output buffer
//! sizes passed to each call) give the same output bytes, return codes,
//! counters and checksums as C zlib 1.3.2.
//!
//! The safe API mirrors `z_stream`: each call to [`Deflate::deflate`] or
//! [`Inflate::inflate`] receives the caller's input slice (`next_in` /
//! `avail_in`) and output slice (`next_out` / `avail_out`) and reports how
//! many bytes were consumed and produced. Advancing the slices by those
//! amounts reproduces C's pointer updates exactly. The [`zlib`] module adds
//! thin C-named wrappers (`deflateInit2`, `inflate`, `gzwrite`, ...) for
//! straight ports of C callers, and [`gz`] ports the gzip file layer.

// The code mirrors the C sources statement by statement; these lints would
// rewrite C idioms (range checks, ternaries, manual loops) away from the
// original structure.
#![allow(clippy::too_many_arguments)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::manual_range_contains)]
#![allow(clippy::if_same_then_else)]
#![allow(clippy::manual_div_ceil)]
#![allow(clippy::manual_saturating_arithmetic)]
#![allow(clippy::unnecessary_map_or)]
#![allow(clippy::manual_is_multiple_of)]
#![allow(clippy::chunks_exact_to_as_chunks)]
#![allow(clippy::comparison_chain)]

pub mod adler32;
mod compress;
pub mod crc32;
mod deflate;
pub mod gz;
mod inffast;
mod inflate;
mod inftrees;
mod tables;
mod trees;
pub mod zlib;

pub use adler32::{adler32, adler32_combine, adler32_z};
pub use compress::{
    compress, compress2, compress_bound, compress_to_vec, uncompress, uncompress2,
    uncompress_to_vec,
};
pub use crc32::{crc32, crc32_combine, crc32_combine_gen, crc32_combine_op, crc32_z, get_crc_table};
pub use deflate::{DEF_MEM_LEVEL, MAX_MEM_LEVEL, MAX_WBITS, OS_CODE};
pub use gz::{GzReader, GzWriter};

use deflate::{DeflateState, Io};
use inflate::InflateState;

/// `ZLIB_VERSION`
pub const ZLIB_VERSION: &str = "1.3.2";
/// `ZLIB_VERNUM`
pub const ZLIB_VERNUM: u32 = 0x1320;

// Allowed flush values; see deflate() and inflate() in zlib.h.
pub const Z_NO_FLUSH: i32 = 0;
pub const Z_PARTIAL_FLUSH: i32 = 1;
pub const Z_SYNC_FLUSH: i32 = 2;
pub const Z_FULL_FLUSH: i32 = 3;
pub const Z_FINISH: i32 = 4;
pub const Z_BLOCK: i32 = 5;
pub const Z_TREES: i32 = 6;

// Return codes for the compression/decompression functions.
pub const Z_OK: i32 = 0;
pub const Z_STREAM_END: i32 = 1;
pub const Z_NEED_DICT: i32 = 2;
pub const Z_ERRNO: i32 = -1;
pub const Z_STREAM_ERROR: i32 = -2;
pub const Z_DATA_ERROR: i32 = -3;
pub const Z_MEM_ERROR: i32 = -4;
pub const Z_BUF_ERROR: i32 = -5;
pub const Z_VERSION_ERROR: i32 = -6;

// Compression levels.
pub const Z_NO_COMPRESSION: i32 = 0;
pub const Z_BEST_SPEED: i32 = 1;
pub const Z_BEST_COMPRESSION: i32 = 9;
pub const Z_DEFAULT_COMPRESSION: i32 = -1;

// Compression strategies.
pub const Z_FILTERED: i32 = 1;
pub const Z_HUFFMAN_ONLY: i32 = 2;
pub const Z_RLE: i32 = 3;
pub const Z_FIXED: i32 = 4;
pub const Z_DEFAULT_STRATEGY: i32 = 0;

// Possible values of the data_type field for deflate().
pub const Z_BINARY: i32 = 0;
pub const Z_TEXT: i32 = 1;
pub const Z_ASCII: i32 = Z_TEXT;
pub const Z_UNKNOWN: i32 = 2;

/// The deflate compression method (the only one supported in this version).
pub const Z_DEFLATED: i32 = 8;

static Z_ERRMSG: [&str; 10] = [
    "need dictionary",      // Z_NEED_DICT       2
    "stream end",           // Z_STREAM_END      1
    "",                     // Z_OK              0
    "file error",           // Z_ERRNO         (-1)
    "stream error",         // Z_STREAM_ERROR  (-2)
    "data error",           // Z_DATA_ERROR    (-3)
    "insufficient memory",  // Z_MEM_ERROR     (-4)
    "buffer error",         // Z_BUF_ERROR     (-5)
    "incompatible version", // Z_VERSION_ERROR (-6)
    "",
];

/// `ERR_MSG(err)` / `zError(err)`.
pub fn err_msg(err: i32) -> &'static str {
    if !(-6..=2).contains(&err) {
        Z_ERRMSG[9]
    } else {
        Z_ERRMSG[(2 - err) as usize]
    }
}

/// `zlibVersion()`
pub fn zlib_version() -> &'static str {
    ZLIB_VERSION
}

/// Flush modes for [`Deflate::deflate`] and [`Inflate::inflate`].
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flush {
    NoFlush = 0,
    PartialFlush = 1,
    SyncFlush = 2,
    FullFlush = 3,
    Finish = 4,
    Block = 5,
    Trees = 6,
}

/// zlib return codes.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    Ok = 0,
    StreamEnd = 1,
    NeedDict = 2,
    Errno = -1,
    StreamError = -2,
    DataError = -3,
    MemError = -4,
    BufError = -5,
    VersionError = -6,
}

impl Status {
    /// Map a C return code to a `Status`.
    pub fn from_code(code: i32) -> Status {
        match code {
            0 => Status::Ok,
            1 => Status::StreamEnd,
            2 => Status::NeedDict,
            -1 => Status::Errno,
            -2 => Status::StreamError,
            -3 => Status::DataError,
            -4 => Status::MemError,
            -5 => Status::BufError,
            -6 => Status::VersionError,
            _ => Status::StreamError,
        }
    }
    /// The C return code.
    pub fn code(self) -> i32 {
        self as i32
    }
    /// True for negative (error) codes.
    pub fn is_error(self) -> bool {
        (self as i32) < 0
    }
}

/// Result of one deflate()/inflate() call: the return code and how far
/// `next_in` / `next_out` advanced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamResult {
    pub status: Status,
    /// Bytes consumed from the input slice (`avail_in` decrease).
    pub consumed: usize,
    /// Bytes written to the output slice (`avail_out` decrease).
    pub produced: usize,
}

/// gzip header information (`gz_header`), used by
/// [`Deflate::set_header`] and [`Inflate::get_header`].
///
/// For deflate, `extra` holds the extra field (its first `extra_len` bytes
/// are written), and `name` / `comment` are written up to and including the
/// first NUL (a missing NUL is treated as the terminator). For inflate the
/// `extra`, `name` and `comment` vectors are the caller's buffers: up to
/// `extra_max` / `name_max` / `comm_max` bytes are stored, as in C.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GzHeader {
    pub text: i32,
    pub time: u32,
    pub xflags: i32,
    pub os: i32,
    pub extra: Option<Vec<u8>>,
    pub extra_len: u32,
    pub extra_max: u32,
    pub name: Option<Vec<u8>>,
    pub name_max: u32,
    pub comment: Option<Vec<u8>>,
    pub comm_max: u32,
    pub hcrc: i32,
    pub done: i32,
}

/// A deflate (compression) stream: `deflateInit2()` .. `deflateEnd()`.
#[derive(Clone)]
pub struct Deflate {
    s: Box<DeflateState>,
}

impl std::fmt::Debug for Deflate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deflate")
            .field("total_in", &self.s.total_in)
            .field("total_out", &self.s.total_out)
            .field("level", &self.s.level)
            .finish()
    }
}

impl Deflate {
    /// `deflateInit(level)`: zlib wrapper, 32K window, memLevel 8.
    pub fn new(level: i32) -> Result<Deflate, Status> {
        Self::with_params(level, Z_DEFLATED, MAX_WBITS, DEF_MEM_LEVEL, Z_DEFAULT_STRATEGY)
    }

    /// `deflateInit2(level, method, windowBits, memLevel, strategy)`.
    /// windowBits 8..15 gives a zlib stream, -8..-15 raw deflate, and 24..31
    /// a gzip stream.
    pub fn with_params(
        level: i32,
        method: i32,
        window_bits: i32,
        mem_level: i32,
        strategy: i32,
    ) -> Result<Deflate, Status> {
        DeflateState::init2(level, method, window_bits, mem_level, strategy)
            .map(|s| Deflate { s })
            .map_err(Status::from_code)
    }

    /// `deflate(strm, flush)` with `avail_in = input.len()` and
    /// `avail_out = output.len()` (each capped at `u32::MAX`, like `uInt`).
    pub fn deflate(&mut self, input: &[u8], output: &mut [u8], flush: Flush) -> StreamResult {
        self.deflate_raw(input, output, flush as i32)
    }

    /// Like [`Deflate::deflate`] but with a raw C flush value (invalid values
    /// give `Z_STREAM_ERROR`, as in C).
    pub fn deflate_raw(&mut self, input: &[u8], output: &mut [u8], flush: i32) -> StreamResult {
        let mut io = Io::new(input, output);
        let (ai, ao) = (io.avail_in, io.avail_out);
        let r = self.s.deflate(&mut io, flush);
        StreamResult {
            status: Status::from_code(r),
            consumed: (ai - io.avail_in) as usize,
            produced: (ao - io.avail_out) as usize,
        }
    }

    /// `deflateParams()`. As in C this may call `deflate(strm, Z_BLOCK)`
    /// with the given buffers, so it can consume input and produce output.
    pub fn params(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        level: i32,
        strategy: i32,
    ) -> StreamResult {
        let mut io = Io::new(input, output);
        let (ai, ao) = (io.avail_in, io.avail_out);
        let r = self.s.params(&mut io, level, strategy);
        StreamResult {
            status: Status::from_code(r),
            consumed: (ai - io.avail_in) as usize,
            produced: (ao - io.avail_out) as usize,
        }
    }

    /// `deflateReset()`
    pub fn reset(&mut self) -> Status {
        self.s.reset();
        Status::Ok
    }

    /// `deflateResetKeep()`
    pub fn reset_keep(&mut self) -> Status {
        self.s.reset_keep();
        Status::Ok
    }

    /// `deflateSetDictionary()`
    pub fn set_dictionary(&mut self, dictionary: &[u8]) -> Status {
        Status::from_code(self.s.set_dictionary(dictionary))
    }

    /// `deflateGetDictionary()`
    pub fn get_dictionary(&self) -> Vec<u8> {
        self.s.get_dictionary()
    }

    /// `deflateSetHeader()` (gzip streams only).
    pub fn set_header(&mut self, head: GzHeader) -> Status {
        if self.s.wrap != 2 {
            return Status::StreamError;
        }
        self.s.gzhead = Some(head);
        Status::Ok
    }

    /// `deflateTune()`
    pub fn tune(&mut self, good_length: i32, max_lazy: i32, nice_length: i32, max_chain: i32) -> Status {
        self.s.tune(good_length, max_lazy, nice_length, max_chain);
        Status::Ok
    }

    /// `deflateBound_z()` for this stream's parameters.
    pub fn bound(&self, source_len: u64) -> u64 {
        self.s.bound(source_len)
    }

    /// `deflatePending()`: (pending bytes, pending bits).
    pub fn pending(&self) -> (u64, i32) {
        (self.s.pending as u64, self.s.bi_valid)
    }

    /// `deflateUsed()`: number of bits used in the last byte when the
    /// stream ended on a byte boundary.
    pub fn used(&self) -> i32 {
        self.s.bi_used
    }

    /// `deflatePrime()`
    pub fn prime(&mut self, bits: i32, value: i32) -> Status {
        Status::from_code(self.s.prime(bits, value))
    }

    /// `deflateEnd()`: `Z_DATA_ERROR` if the stream was freed prematurely
    /// (some input or output was discarded), else `Z_OK`.
    pub fn end(self) -> Status {
        Status::from_code(self.s.end_status())
    }

    pub fn total_in(&self) -> u64 {
        self.s.total_in
    }
    pub fn total_out(&self) -> u64 {
        self.s.total_out
    }
    /// `strm->adler`: Adler-32 (zlib), CRC-32 (gzip) of the input so far.
    pub fn adler(&self) -> u32 {
        self.s.adler
    }
    /// `strm->data_type`: `Z_BINARY`, `Z_TEXT` or `Z_UNKNOWN`.
    pub fn data_type(&self) -> i32 {
        self.s.data_type
    }
    /// Set `strm->data_type` (deflate only guesses it while it is
    /// `Z_UNKNOWN`).
    pub fn set_data_type(&mut self, t: i32) {
        self.s.data_type = t;
    }
    /// `strm->msg`
    pub fn msg(&self) -> Option<&'static str> {
        self.s.msg
    }
}

/// `deflateBound(Z_NULL, sourceLen)`: bound without stream parameters.
pub fn deflate_bound_no_stream(source_len: u64) -> u64 {
    deflate::bound_no_state(source_len)
}

/// An inflate (decompression) stream: `inflateInit2()` .. `inflateEnd()`.
#[derive(Clone)]
pub struct Inflate {
    s: Box<InflateState>,
}

impl std::fmt::Debug for Inflate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inflate")
            .field("total_in", &self.s.total_in)
            .field("total_out", &self.s.total_out)
            .field("mode", &self.s.mode)
            .finish()
    }
}

impl Inflate {
    /// `inflateInit()`: zlib stream with up to 32K window.
    pub fn new() -> Result<Inflate, Status> {
        Self::with_window_bits(15)
    }

    /// `inflateInit2(windowBits)`: 8..15 zlib, 0 zlib with window size from
    /// the header, -8..-15 raw, +16 gzip only, +32 automatic zlib/gzip.
    pub fn with_window_bits(window_bits: i32) -> Result<Inflate, Status> {
        InflateState::init2(window_bits)
            .map(|s| Inflate { s })
            .map_err(Status::from_code)
    }

    /// `inflate(strm, flush)`.
    pub fn inflate(&mut self, input: &[u8], output: &mut [u8], flush: Flush) -> StreamResult {
        self.inflate_raw(input, output, flush as i32)
    }

    /// `inflate()` with a raw C flush value.
    pub fn inflate_raw(&mut self, input: &[u8], output: &mut [u8], flush: i32) -> StreamResult {
        let mut io = Io::new(input, output);
        let (ai, ao) = (io.avail_in, io.avail_out);
        let r = self.s.inflate(&mut io, flush);
        StreamResult {
            status: Status::from_code(r),
            consumed: (ai - io.avail_in) as usize,
            produced: (ao - io.avail_out) as usize,
        }
    }

    /// `inflateReset()`
    pub fn reset(&mut self) -> Status {
        Status::from_code(self.s.reset())
    }
    /// `inflateResetKeep()`
    pub fn reset_keep(&mut self) -> Status {
        Status::from_code(self.s.reset_keep())
    }
    /// `inflateReset2()`
    pub fn reset2(&mut self, window_bits: i32) -> Status {
        Status::from_code(self.s.reset2(window_bits))
    }
    /// `inflateSetDictionary()`
    pub fn set_dictionary(&mut self, dictionary: &[u8]) -> Status {
        Status::from_code(self.s.set_dictionary(dictionary))
    }
    /// `inflateGetDictionary()`
    pub fn get_dictionary(&self) -> Vec<u8> {
        self.s.get_dictionary()
    }
    /// `inflateSync()`: returns the status and the number of input bytes
    /// skipped.
    pub fn sync(&mut self, input: &[u8]) -> StreamResult {
        let mut scratch = [0u8; 0];
        let mut io = Io::new(input, &mut scratch);
        let ai = io.avail_in;
        let r = self.s.sync(&mut io);
        StreamResult {
            status: Status::from_code(r),
            consumed: (ai - io.avail_in) as usize,
            produced: 0,
        }
    }
    /// `inflateSyncPoint()`
    pub fn sync_point(&self) -> bool {
        self.s.sync_point() != 0
    }
    /// `inflatePrime()`
    pub fn prime(&mut self, bits: i32, value: i32) -> Status {
        Status::from_code(self.s.prime(bits, value))
    }
    /// `inflateMark()`
    pub fn mark(&self) -> i64 {
        self.s.mark()
    }
    /// `inflateCodesUsed()`
    pub fn codes_used(&self) -> u64 {
        self.s.next as u64
    }
    /// `inflateValidate()`
    pub fn validate(&mut self, check: bool) -> Status {
        self.s.validate(check);
        Status::Ok
    }
    /// `inflateUndermine()`: not compiled with
    /// INFLATE_ALLOW_INVALID_DISTANCE_TOOFAR_ARRR, so always `Z_DATA_ERROR`.
    pub fn undermine(&mut self, _subvert: bool) -> Status {
        self.s.sane = true;
        Status::DataError
    }
    /// `inflateGetHeader()`: request gzip header information. The header
    /// is filled in as inflate() processes it; read it with
    /// [`Inflate::header`].
    pub fn get_header(&mut self, mut head: GzHeader) -> Status {
        if (self.s.wrap & 2) == 0 {
            return Status::StreamError;
        }
        head.done = 0;
        self.s.head = Some(head);
        Status::Ok
    }
    /// The gzip header registered with [`Inflate::get_header`].
    pub fn header(&self) -> Option<&GzHeader> {
        self.s.head.as_ref()
    }
    /// `inflateEnd()`
    pub fn end(self) -> Status {
        Status::Ok
    }

    pub fn total_in(&self) -> u64 {
        self.s.total_in
    }
    pub fn total_out(&self) -> u64 {
        self.s.total_out
    }
    /// `strm->adler`
    pub fn adler(&self) -> u32 {
        self.s.adler
    }
    /// `strm->data_type` (bit-level state flags as in C).
    pub fn data_type(&self) -> i32 {
        self.s.data_type
    }
    /// `strm->msg`
    pub fn msg(&self) -> Option<&'static str> {
        self.s.msg
    }
}
