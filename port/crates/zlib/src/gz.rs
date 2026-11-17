//! Port of the gzip file layer (gzlib.c, gzwrite.c, gzread.c, gzclose.c) of
//! zlib 1.3.2, over any `std::io::Write` / `std::io::Read` instead of a file
//! descriptor.
//!
//! Buffering is reproduced exactly: `want` defaults to `GZBUFSIZE` (8192),
//! the write side uses an input buffer of `2*want` and an output buffer of
//! `want`, writes shorter than `size` are buffered and longer ones are
//! compressed directly from the caller's buffer, and `deflate()` /
//! `inflate()` are called with the same flush values and buffer sizes as the
//! C code, so the compressed bytes are identical to C zlib's for the same
//! sequence of calls (e.g. pdfTeX's `gzdopen(fd, "wb")`,
//! `gzsetparams(f, 1, Z_DEFAULT_STRATEGY)`, `gzwrite()`..., `gzclose()`).
//!
//! Differences that cannot affect data: errors carry `std::io::Error` text
//! instead of `strerror(errno)`; a non-blocking stall while compressing
//! directly from a caller's buffer does not keep a pointer to that buffer
//! (the unconsumed part must be passed again).

use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::deflate::{DeflateState, Io, DEF_MEM_LEVEL, MAX_WBITS};
use crate::inflate::InflateState;
use crate::{
    Z_BLOCK, Z_BUF_ERROR, Z_DATA_ERROR, Z_DEFAULT_COMPRESSION, Z_DEFAULT_STRATEGY, Z_DEFLATED,
    Z_ERRNO, Z_FILTERED, Z_FINISH, Z_FIXED, Z_HUFFMAN_ONLY, Z_MEM_ERROR, Z_NEED_DICT, Z_NO_FLUSH,
    Z_OK, Z_RLE, Z_STREAM_END, Z_STREAM_ERROR,
};

/// default i/o buffer size -- double this for output when reading
pub const GZBUFSIZE: u32 = 8192;

const LOOK: i32 = 0; // look for a gzip header
const COPY: i32 = 1; // copy input directly
const GZIP: i32 = 2; // decompress a gzip stream

/// `((unsigned)-1 >> 2) + 1`: largest single read()/write() request.
const IO_MAX: usize = ((u32::MAX >> 2) + 1) as usize;

/// Parsed `mode` string of `gzopen()` / `gzdopen()`.
#[derive(Clone, Copy, Debug)]
struct ModeSpec {
    read: bool,
    write: bool,
    level: i32,
    strategy: i32,
    direct: i32,
}

/// Interpret a mode string as `gz_open()` does. Returns `None` where C
/// returns NULL.
fn parse_mode(mode: &str) -> Option<ModeSpec> {
    // GZ_NONE / GZ_READ / GZ_WRITE / GZ_APPEND
    let mut m = 0; // 0 none, 1 read, 2 write, 3 append
    let mut level = Z_DEFAULT_COMPRESSION;
    let mut strategy = Z_DEFAULT_STRATEGY;
    let mut direct = 0;
    for c in mode.bytes() {
        if c.is_ascii_digit() {
            level = (c - b'0') as i32;
        } else {
            match c {
                b'r' => m = 1,
                b'w' => m = 2,
                b'a' => m = 3,
                b'+' => return None, // can't read and write at the same time
                b'b' => {}
                b'f' => strategy = Z_FILTERED,
                b'h' => strategy = Z_HUFFMAN_ONLY,
                b'R' => strategy = Z_RLE,
                b'F' => strategy = Z_FIXED,
                b'G' => direct = -1,
                b'T' => direct = 1,
                _ => {} // could consider as an error, but just ignore
            }
        }
    }
    if m == 0 {
        return None;
    }
    if m == 1 {
        if direct == 1 {
            return None; // can't force a transparent read
        }
        if direct == 0 {
            // default when reading is auto-detect of gzip vs. transparent
            direct = 1;
        }
    } else if direct == -1 {
        return None; // "G" has no meaning when writing
    }
    Some(ModeSpec {
        read: m == 1,
        write: m == 2 || m == 3,
        level,
        strategy,
        direct,
    })
}

/// Common error state (`gz_error()`).
#[derive(Debug, Default)]
struct ErrState {
    err: i32,
    msg: Option<String>,
    again: bool,
}

impl ErrState {
    /// `gz_error()`: returns true if `x.have` must be cleared.
    fn set(&mut self, path: &str, err: i32, msg: Option<&str>) -> bool {
        self.msg = None;
        let fatal = err != Z_OK && err != Z_BUF_ERROR && !self.again;
        self.err = err;
        if let Some(m) = msg {
            if err != Z_MEM_ERROR {
                self.msg = Some(format!("{}: {}", path, m));
            }
        }
        fatal
    }
    /// `gzerror()`
    fn message(&self) -> String {
        if self.err == Z_MEM_ERROR {
            "out of memory".to_string()
        } else {
            self.msg.clone().unwrap_or_default()
        }
    }
}

fn io_err_msg(e: &io::Error) -> String {
    e.to_string()
}

// ===========================================================================
// Writing
// ===========================================================================

/// A gzip file opened for writing (`gzopen(path, "w")` / `gzdopen(fd, "wb")`)
/// over any `Write`.
pub struct GzWriter<W: Write> {
    w: Option<W>,
    path: String,
    size: u32,
    want: u32,
    inbuf: Vec<u8>,
    outbuf: Vec<u8>,
    direct: i32,
    level: i32,
    strategy: i32,
    reset: bool,
    skip: i64,
    e: ErrState,
    pos: i64,
    x_next: usize,
    strm: Option<Box<DeflateState>>,
    next_in: usize,
    avail_in: u32,
    next_out: usize,
    avail_out: u32,
}

/// Where gz_comp() takes its input from: the internal buffer or the
/// caller's buffer (strm.next_in pointing into either in C).
#[derive(Clone, Copy)]
enum Src<'a> {
    Buf,
    User(&'a [u8]),
}

impl<W: Write> GzWriter<W> {
    /// `gzdopen(fd, mode)` with a writer instead of a file descriptor. The
    /// mode must request writing (`"w"`/`"a"`, plus optional level digit,
    /// strategy letter `f`/`h`/`R`/`F` or `T` for transparent output).
    /// Returns `None` where C returns NULL. `name` is used in error messages
    /// (C uses `"<fd:N>"`).
    pub fn with_mode(w: W, mode: &str, name: &str) -> Option<GzWriter<W>> {
        let spec = parse_mode(mode)?;
        if !spec.write {
            return None;
        }
        Some(GzWriter {
            w: Some(w),
            path: name.to_string(),
            size: 0, // no buffers allocated yet
            want: GZBUFSIZE,
            inbuf: Vec::new(),
            outbuf: Vec::new(),
            direct: spec.direct,
            level: spec.level,
            strategy: spec.strategy,
            reset: false,
            skip: 0,
            e: ErrState::default(),
            pos: 0,
            x_next: 0,
            strm: None,
            next_in: 0,
            avail_in: 0,
            next_out: 0,
            avail_out: 0,
        })
    }

    /// `gzdopen(fd, "wb")` equivalent with default parameters.
    pub fn new(w: W) -> GzWriter<W> {
        Self::with_mode(w, "wb", "<writer>").expect("valid mode")
    }

    /// `gz_error()`
    fn gz_error(&mut self, err: i32, msg: Option<&str>) {
        let _ = self.e.set(&self.path, err, msg);
    }

    /// Initialize state for writing a gzip file.
    fn gz_init(&mut self) -> i32 {
        // allocate input buffer (double size for gzprintf)
        self.inbuf = vec![0u8; (self.want as usize) << 1];

        // only need output buffer and deflate state if compressing
        if self.direct == 0 {
            // allocate output buffer
            self.outbuf = vec![0u8; self.want as usize];

            // allocate deflate memory, set up for gzip compression
            match DeflateState::init2(
                self.level,
                Z_DEFLATED,
                MAX_WBITS + 16,
                DEF_MEM_LEVEL,
                self.strategy,
            ) {
                Ok(s) => self.strm = Some(s),
                Err(_) => {
                    self.outbuf = Vec::new();
                    self.inbuf = Vec::new();
                    self.gz_error(Z_MEM_ERROR, Some("out of memory"));
                    return -1;
                }
            }
            self.next_in = 0; // strm->next_in = NULL
        }

        // mark state as initialized
        self.size = self.want;

        // initialize write buffer if compressing
        if self.direct == 0 {
            self.avail_out = self.size;
            self.next_out = 0;
            self.x_next = 0;
        }
        0
    }

    /// Write `buf` to the underlying writer, handling short writes.
    /// Returns bytes written or an error.
    fn raw_write(w: &mut W, buf: &[u8]) -> io::Result<usize> {
        loop {
            match w.write(buf) {
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// Compress whatever is at avail_in and next_in and write to the output
    /// file. Return -1 if there is an error writing to the output file or if
    /// gz_init() fails to allocate memory, otherwise 0.
    fn gz_comp(&mut self, src: Src, flush: i32) -> i32 {
        // allocate memory if this is the first time through
        if self.size == 0 && self.gz_init() == -1 {
            return -1;
        }

        // write directly if requested
        if self.direct != 0 {
            while self.avail_in != 0 {
                self.e.again = false;
                let put = (self.avail_in as usize).min(IO_MAX);
                let data: &[u8] = match src {
                    Src::Buf => &self.inbuf[self.next_in..self.next_in + put],
                    Src::User(b) => &b[self.next_in..self.next_in + put],
                };
                let w = self.w.as_mut().expect("open");
                match Self::raw_write(w, data) {
                    Ok(writ) => {
                        self.avail_in -= writ as u32;
                        self.next_in += writ;
                        if writ == 0 {
                            let m = "write returned zero".to_string();
                            self.gz_error(Z_ERRNO, Some(&m));
                            return -1;
                        }
                    }
                    Err(e) => {
                        if e.kind() == io::ErrorKind::WouldBlock {
                            self.e.again = true;
                        }
                        let m = io_err_msg(&e);
                        self.gz_error(Z_ERRNO, Some(&m));
                        return -1;
                    }
                }
            }
            return 0;
        }

        // check for a pending reset
        if self.reset {
            // don't start a new gzip member unless there is data to write and
            // we're not flushing
            if self.avail_in == 0 && flush == Z_NO_FLUSH {
                return 0;
            }
            self.strm.as_mut().unwrap().reset();
            self.reset = false;
        }

        // run deflate() on provided input until it produces no more output
        let mut ret = Z_OK;
        loop {
            // write out current buffer contents if full, or if flushing, but
            // if doing Z_FINISH then don't write until we get to Z_STREAM_END
            if self.avail_out == 0
                || (flush != Z_NO_FLUSH && (flush != Z_FINISH || ret == Z_STREAM_END))
            {
                while self.next_out > self.x_next {
                    self.e.again = false;
                    let put = (self.next_out - self.x_next).min(IO_MAX);
                    let w = self.w.as_mut().expect("open");
                    match Self::raw_write(w, &self.outbuf[self.x_next..self.x_next + put]) {
                        Ok(0) => {
                            self.gz_error(Z_ERRNO, Some("write returned zero"));
                            return -1;
                        }
                        Ok(writ) => self.x_next += writ,
                        Err(e) => {
                            if e.kind() == io::ErrorKind::WouldBlock {
                                self.e.again = true;
                            }
                            let m = io_err_msg(&e);
                            self.gz_error(Z_ERRNO, Some(&m));
                            return -1;
                        }
                    }
                }
                if self.avail_out == 0 {
                    self.avail_out = self.size;
                    self.next_out = 0;
                    self.x_next = 0;
                }
            }

            // compress
            let have = self.avail_out;
            {
                let input: &[u8] = match src {
                    Src::Buf => &self.inbuf[self.next_in..self.next_in + self.avail_in as usize],
                    Src::User(b) => &b[self.next_in..self.next_in + self.avail_in as usize],
                };
                let output =
                    &mut self.outbuf[self.next_out..self.next_out + self.avail_out as usize];
                let mut io = Io::new(input, output);
                ret = self.strm.as_mut().unwrap().deflate(&mut io, flush);
                self.next_in += io.next_in;
                self.avail_in = io.avail_in;
                self.next_out += io.next_out;
                self.avail_out = io.avail_out;
            }
            if ret == Z_STREAM_ERROR {
                self.gz_error(Z_STREAM_ERROR, Some("internal error: deflate stream corrupt"));
                return -1;
            }
            if have - self.avail_out == 0 {
                break;
            }
        }

        // if that completed a deflate stream, allow another to start
        if flush == Z_FINISH {
            self.reset = true;
        }

        // all done, no errors
        0
    }

    /// Compress state->skip (> 0) zeros to output.
    fn gz_zero(&mut self) -> i32 {
        // consume whatever's left in the input buffer
        if self.avail_in != 0 && self.gz_comp(Src::Buf, Z_NO_FLUSH) == -1 {
            return -1;
        }

        // compress state->skip zeros
        let mut first = true;
        loop {
            let n0: u32 = if (self.size as i64) > self.skip {
                self.skip as u32
            } else {
                self.size
            };
            if first {
                self.inbuf[..n0 as usize].fill(0);
                first = false;
            }
            self.avail_in = n0;
            self.next_in = 0;
            let ret = self.gz_comp(Src::Buf, Z_NO_FLUSH);
            let n = n0 - self.avail_in;
            self.pos += n as i64;
            self.skip -= n as i64;
            if ret == -1 {
                return -1;
            }
            if self.skip == 0 {
                break;
            }
        }
        0
    }

    /// `gz_write()`: write len bytes from buf to file. Return the number of
    /// bytes written.
    fn gz_write(&mut self, buf: &[u8]) -> usize {
        let put = buf.len();
        let mut len = buf.len();
        let mut off = 0usize;

        // if len is zero, avoid unnecessary operations
        if len == 0 {
            return 0;
        }

        // allocate memory if this is the first time through
        if self.size == 0 && self.gz_init() == -1 {
            return 0;
        }

        // check for seek request
        if self.skip != 0 && self.gz_zero() == -1 {
            return 0;
        }

        // for small len, copy to input buffer, otherwise compress directly
        if len < self.size as usize {
            // copy to input buffer, compress when full
            loop {
                if self.avail_in == 0 {
                    self.next_in = 0;
                }
                let have = self.next_in + self.avail_in as usize;
                let mut copy = self.size as usize - have;
                if copy > len {
                    copy = len;
                }
                self.inbuf[have..have + copy].copy_from_slice(&buf[off..off + copy]);
                self.avail_in += copy as u32;
                self.pos += copy as i64;
                off += copy;
                len -= copy;
                if len == 0 {
                    break;
                }
                if self.gz_comp(Src::Buf, Z_NO_FLUSH) == -1 {
                    return if self.e.again { put - len } else { 0 };
                }
            }
        } else {
            // consume whatever's left in the input buffer
            if self.avail_in != 0 && self.gz_comp(Src::Buf, Z_NO_FLUSH) == -1 {
                return 0;
            }

            // directly compress user buffer to file
            loop {
                let n0 = len.min(u32::MAX as usize) as u32;
                self.next_in = 0;
                self.avail_in = n0;
                let ret = self.gz_comp(Src::User(&buf[off..off + n0 as usize]), Z_NO_FLUSH);
                let n = n0 - self.avail_in;
                // the caller's buffer is not retained between calls
                self.avail_in = 0;
                self.next_in = 0;
                self.pos += n as i64;
                off += n as usize;
                len -= n as usize;
                if ret == -1 {
                    return if self.e.again { put - len } else { 0 };
                }
                if len == 0 {
                    break;
                }
            }
        }

        // input was all buffered or compressed
        put
    }

    /// Writes are refused after a serious error.
    fn writable(&self) -> bool {
        self.w.is_some() && (self.e.err == Z_OK || self.e.again)
    }

    /// `gzwrite()`: returns the number of uncompressed bytes written, or 0
    /// in case of error.
    pub fn gzwrite(&mut self, buf: &[u8]) -> i32 {
        // check that we're writing and that there's no (serious) error
        if !self.writable() {
            return 0;
        }
        self.gz_error(Z_OK, None);

        // since an int is returned, make sure len fits in one, otherwise
        // return with an error (this avoids a flaw in the interface)
        if buf.len() > i32::MAX as usize {
            self.gz_error(Z_DATA_ERROR, Some("requested length does not fit in int"));
            return 0;
        }

        // write len bytes from buf (the return value will fit in an int)
        self.gz_write(buf) as i32
    }

    /// `gzfwrite()`: returns the number of full items written.
    pub fn gzfwrite(&mut self, buf: &[u8], size: usize, nitems: usize) -> usize {
        if !self.writable() {
            return 0;
        }
        self.gz_error(Z_OK, None);
        let len = match size.checked_mul(nitems) {
            Some(l) => l,
            None => {
                self.gz_error(Z_STREAM_ERROR, Some("request does not fit in a size_t"));
                return 0;
            }
        };
        if len == 0 {
            return 0;
        }
        self.gz_write(&buf[..len]) / size
    }

    /// `gzputc()`
    pub fn gzputc(&mut self, c: i32) -> i32 {
        if !self.writable() {
            return -1;
        }
        self.gz_error(Z_OK, None);

        // check for seek request
        if self.skip != 0 && self.gz_zero() == -1 {
            return -1;
        }

        // try writing to input buffer for speed (state->size == 0 if buffer
        // not initialized)
        if self.size != 0 {
            if self.avail_in == 0 {
                self.next_in = 0;
            }
            let have = self.next_in + self.avail_in as usize;
            if have < self.size as usize {
                self.inbuf[have] = c as u8;
                self.avail_in += 1;
                self.pos += 1;
                return c & 0xff;
            }
        }

        // no room in buffer or not initialized, use gz_write()
        let b = [c as u8];
        if self.gz_write(&b) != 1 {
            return -1;
        }
        c & 0xff
    }

    /// `gzputs()`: write a string (up to its first NUL, as C would).
    pub fn gzputs(&mut self, s: &[u8]) -> i32 {
        if !self.writable() {
            return -1;
        }
        self.gz_error(Z_OK, None);
        let len = s.iter().position(|&b| b == 0).unwrap_or(s.len());
        if len > i32::MAX as usize {
            self.gz_error(Z_STREAM_ERROR, Some("string length does not fit in int"));
            return -1;
        }
        let put = self.gz_write(&s[..len]);
        if len != 0 && put == 0 {
            -1
        } else {
            put as i32
        }
    }

    /// If the second half of the input buffer is occupied, write out the
    /// contents.
    fn gz_vacate(&mut self) -> bool {
        if self.next_in + self.avail_in as usize <= self.size as usize {
            return false;
        }
        let _ = self.gz_comp(Src::Buf, Z_NO_FLUSH);
        if self.avail_in == 0 {
            self.next_in = 0;
            return false;
        }
        let (a, n) = (self.next_in, self.avail_in as usize);
        self.inbuf.copy_within(a..a + n, 0);
        self.next_in = 0;
        self.avail_in as usize > self.size as usize
    }

    /// `gzprintf()` / `gzvprintf()` with an already formatted string: the
    /// text must fit in the buffer (`len < size`) or nothing is written and
    /// 0 is returned, as with C's vsnprintf-based implementation.
    pub fn gzprintf(&mut self, text: &str) -> i32 {
        if !self.writable() {
            return Z_STREAM_ERROR;
        }
        self.gz_error(Z_OK, None);

        // make sure we have some buffer space
        if self.size == 0 && self.gz_init() == -1 {
            return self.e.err;
        }

        // check for seek request
        if self.skip != 0 && self.gz_zero() == -1 {
            return self.e.err;
        }

        let ret = self.gz_vacate();
        if self.e.err != Z_OK {
            if ret && self.e.again {
                self.gz_error(Z_BUF_ERROR, Some("stalled write on gzprintf"));
            }
            if !self.e.again {
                return self.e.err;
            }
        }
        if self.avail_in == 0 {
            self.next_in = 0;
        }
        let bytes = text.as_bytes();
        let len = bytes.len();

        // check that printf() results fit in buffer
        if len == 0 || len >= self.size as usize {
            return 0;
        }
        let at = self.next_in + self.avail_in as usize;
        self.inbuf[at..at + len].copy_from_slice(bytes);

        // update buffer and position
        self.avail_in += len as u32;
        self.pos += len as i64;

        // write out buffer if more than half is occupied
        let _ = self.gz_vacate();
        if self.e.err != Z_OK && !self.e.again {
            return self.e.err;
        }
        len as i32
    }

    /// `gzflush()`
    pub fn gzflush(&mut self, flush: i32) -> i32 {
        if !self.writable() {
            return Z_STREAM_ERROR;
        }
        self.gz_error(Z_OK, None);

        // check flush parameter
        if !(0..=Z_FINISH).contains(&flush) {
            return Z_STREAM_ERROR;
        }

        // check for seek request
        if self.skip != 0 && self.gz_zero() == -1 {
            return self.e.err;
        }

        // compress remaining data with requested flush
        let _ = self.gz_comp(Src::Buf, flush);
        self.e.err
    }

    /// `gzsetparams()`
    pub fn gzsetparams(&mut self, level: i32, strategy: i32) -> i32 {
        // check that we're compressing and that there's no (serious) error
        if !self.writable() || self.direct != 0 {
            return Z_STREAM_ERROR;
        }
        self.gz_error(Z_OK, None);

        // if no change is requested, then do nothing
        if level == self.level && strategy == self.strategy {
            return Z_OK;
        }

        // check for seek request
        if self.skip != 0 && self.gz_zero() == -1 {
            return self.e.err;
        }

        // change compression parameters for subsequent input
        if self.size != 0 {
            // flush previous input with previous parameters before changing
            if self.avail_in != 0 && self.gz_comp(Src::Buf, Z_BLOCK) == -1 {
                return self.e.err;
            }
            let input = &self.inbuf[self.next_in..self.next_in + self.avail_in as usize];
            let output = &mut self.outbuf[self.next_out..self.next_out + self.avail_out as usize];
            let mut io = Io::new(input, output);
            let _ = self.strm.as_mut().unwrap().params(&mut io, level, strategy);
            self.next_in += io.next_in;
            self.avail_in = io.avail_in;
            self.next_out += io.next_out;
            self.avail_out = io.avail_out;
        }
        self.level = level;
        self.strategy = strategy;
        Z_OK
    }

    /// `gzseek()` while writing: only forward seeks, which write zeros.
    /// `whence` is 0 (SEEK_SET) or 1 (SEEK_CUR).
    pub fn gzseek(&mut self, offset: i64, whence: i32) -> i64 {
        if self.e.err != Z_OK && self.e.err != Z_BUF_ERROR {
            return -1;
        }
        if whence != 0 && whence != 1 {
            return -1;
        }
        let mut offset = offset;
        if whence == 0 {
            offset -= self.pos;
        } else {
            offset += self.skip;
            self.skip = 0;
        }
        if offset < 0 {
            return -1; // writing -- can't go backwards
        }
        self.skip = offset;
        self.pos + offset
    }

    /// `gztell()`
    pub fn gztell(&self) -> i64 {
        self.pos + self.skip
    }

    /// `gzbuffer()`: must be called before the first write.
    pub fn gzbuffer(&mut self, size: u32) -> i32 {
        if self.size != 0 {
            return -1;
        }
        if size.wrapping_shl(1) < size {
            return -1; // need to be able to double it
        }
        self.want = size.max(8);
        0
    }

    /// `gzerror()`: (message, error code).
    pub fn gzerror(&self) -> (String, i32) {
        (self.e.message(), self.e.err)
    }

    /// `gzclearerr()`
    pub fn gzclearerr(&mut self) {
        self.gz_error(Z_OK, None);
    }

    /// The underlying writer.
    pub fn get_ref(&self) -> &W {
        self.w.as_ref().expect("open")
    }

    /// `gzclose_w()`: finish the gzip stream, flush the writer and return
    /// the zlib status together with the writer.
    pub fn finish(mut self) -> (i32, W) {
        let mut ret = Z_OK;

        // check for seek request
        if self.skip != 0 && self.gz_zero() == -1 {
            ret = self.e.err;
        }

        // flush, free memory, and close file
        if self.gz_comp(Src::Buf, Z_FINISH) == -1 {
            ret = self.e.err;
        }
        self.strm = None;
        self.gz_error(Z_OK, None);
        let mut w = self.w.take().expect("open");
        if w.flush().is_err() {
            ret = Z_ERRNO;
        }
        (ret, w)
    }

    /// `gzclose()` / `gzclose_w()`.
    pub fn gzclose(self) -> i32 {
        self.finish().0
    }
}

impl GzWriter<std::fs::File> {
    /// `gzopen(path, mode)` for writing.
    pub fn create<P: AsRef<std::path::Path>>(path: P, mode: &str) -> io::Result<Self> {
        parse_mode(mode)
            .filter(|s| s.write)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad gz mode"))?;
        let f = if mode.contains('a') {
            std::fs::OpenOptions::new().append(true).create(true).open(path.as_ref())?
        } else {
            std::fs::File::create(path.as_ref())?
        };
        let name = path.as_ref().to_string_lossy().into_owned();
        Ok(GzWriter::with_mode(f, mode, &name).expect("mode checked"))
    }
}

// ===========================================================================
// Reading
// ===========================================================================

/// A gzip file opened for reading (`gzopen(path, "r")` / `gzdopen(fd,
/// "rb")`) over any `Read`.
pub struct GzReader<R: Read> {
    r: Option<R>,
    path: String,
    size: u32,
    want: u32,
    inbuf: Vec<u8>,
    outbuf: Vec<u8>,
    direct: i32,
    junk: i32,
    how: i32,
    eof: bool,
    past: bool,
    skip: i64,
    e: ErrState,
    x_have: u32,
    x_next: usize,
    pos: i64,
    strm: Option<Box<InflateState>>,
    next_in: usize,
    avail_in: u32,
    next_out: usize,
    avail_out: u32,
}

impl<R: Read> GzReader<R> {
    /// `gzdopen(fd, mode)` with a reader. The mode must contain `r`
    /// (optionally `G` to require gzip). Returns `None` where C returns NULL.
    pub fn with_mode(r: R, mode: &str, name: &str) -> Option<GzReader<R>> {
        let spec = parse_mode(mode)?;
        if !spec.read {
            return None;
        }
        let mut g = GzReader {
            r: Some(r),
            path: name.to_string(),
            size: 0,
            want: GZBUFSIZE,
            inbuf: Vec::new(),
            outbuf: Vec::new(),
            direct: spec.direct,
            junk: -1,
            how: LOOK,
            eof: false,
            past: false,
            skip: 0,
            e: ErrState::default(),
            x_have: 0,
            x_next: 0,
            pos: 0,
            strm: None,
            next_in: 0,
            avail_in: 0,
            next_out: 0,
            avail_out: 0,
        };
        g.gz_reset();
        Some(g)
    }

    /// `gzdopen(fd, "rb")` equivalent.
    pub fn new(r: R) -> GzReader<R> {
        Self::with_mode(r, "rb", "<reader>").expect("valid mode")
    }

    /// Reset gzip file state.
    fn gz_reset(&mut self) {
        self.x_have = 0; // no output data available
        self.eof = false; // not at end of file
        self.past = false; // have not read past end yet
        self.how = LOOK; // look for gzip header
        self.junk = -1; // mark first member
        self.e.again = false; // no stalled i/o yet
        self.skip = 0; // no seek request pending
        self.gz_error(Z_OK, None); // clear error
        self.pos = 0; // no uncompressed data yet
        self.avail_in = 0; // no input data yet
    }

    fn gz_error(&mut self, err: i32, msg: Option<&str>) {
        if self.e.set(&self.path, err, msg) {
            // if fatal, set state->x.have to 0 so that the gzgetc() macro fails
            self.x_have = 0;
        }
    }

    /// Use read() to load a buffer -- return -1 on error, otherwise 0.
    /// `dst` selects the target buffer. Returns bytes read in `have`.
    fn gz_load(&mut self, dst: LoadDst, off: usize, len: usize, have: &mut usize) -> i32 {
        self.e.again = false;
        *have = 0;
        let mut ret: io::Result<usize>;
        loop {
            let get = (len - *have).min(IO_MAX);
            let r = self.r.as_mut().expect("open");
            let buf: &mut [u8] = match dst {
                LoadDst::In => &mut self.inbuf[off + *have..off + *have + get],
                LoadDst::Out => &mut self.outbuf[off + *have..off + *have + get],
            };
            ret = loop {
                match r.read(buf) {
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    other => break other,
                }
            };
            match &ret {
                Ok(0) | Err(_) => break,
                Ok(n) => *have += *n,
            }
            if *have >= len {
                break;
            }
        }
        match ret {
            Err(e) => {
                if e.kind() == io::ErrorKind::WouldBlock {
                    self.e.again = true;
                    if *have != 0 {
                        return 0;
                    }
                }
                let m = io_err_msg(&e);
                self.gz_error(Z_ERRNO, Some(&m));
                -1
            }
            Ok(0) => {
                self.eof = true;
                0
            }
            Ok(_) => 0,
        }
    }

    /// Load up input buffer and set eof flag if last data loaded.
    fn gz_avail(&mut self) -> i32 {
        if self.e.err != Z_OK && self.e.err != Z_BUF_ERROR {
            return -1;
        }
        if !self.eof {
            if self.avail_in != 0 {
                // copy what's there to the start
                if self.next_in != 0 {
                    let (a, n) = (self.next_in, self.avail_in as usize);
                    self.inbuf.copy_within(a..a + n, 0);
                }
            }
            let mut got = 0usize;
            let ai = self.avail_in as usize;
            let len = self.size as usize - ai;
            if self.gz_load(LoadDst::In, ai, len, &mut got) == -1 {
                return -1;
            }
            self.avail_in += got as u32;
            self.next_in = 0;
        }
        0
    }

    /// Look for gzip header, set up for inflate or copy.
    fn gz_look(&mut self) -> i32 {
        // allocate read buffers and inflate memory
        if self.size == 0 {
            self.inbuf = vec![0u8; self.want as usize];
            self.outbuf = vec![0u8; (self.want as usize) << 1];
            self.size = self.want;

            // allocate inflate memory
            self.avail_in = 0;
            self.next_in = 0;
            match InflateState::init2(15 + 16) {
                Ok(s) => self.strm = Some(s),
                Err(_) => {
                    self.size = 0;
                    self.gz_error(Z_MEM_ERROR, Some("out of memory"));
                    return -1;
                }
            }
        }

        // if transparent reading is disabled, which would only be at the
        // start, or if we're looking for a gzip member after the first one,
        // which is not at the start, then proceed directly to look for a gzip
        // member next
        if self.direct == -1 || self.junk == 0 {
            self.strm.as_mut().unwrap().reset();
            self.how = GZIP;
            self.junk = (self.junk != -1) as i32;
            self.direct = 0;
            return 0;
        }

        // otherwise we're at the start with auto-detect -- we check to see if
        // the first four bytes could be gzip header in order to decide whether
        // or not this will be a transparent read

        // load any header bytes into the input buffer -- if the input is
        // empty, then it's not an error as this is a transparent read of zero
        // bytes
        if self.gz_avail() == -1 {
            return -1;
        }
        if self.avail_in == 0 || (self.e.again && self.avail_in < 4) {
            // if non-blocking input stalled before getting four bytes, then
            // return and wait until a later call has accumulated enough
            return 0;
        }

        // see if this is (likely) gzip input
        let p = self.next_in;
        if self.avail_in > 3
            && self.inbuf[p] == 31
            && self.inbuf[p + 1] == 139
            && self.inbuf[p + 2] == 8
            && self.inbuf[p + 3] < 32
        {
            self.strm.as_mut().unwrap().reset();
            self.how = GZIP;
            self.junk = 1;
            self.direct = 0;
            return 0;
        }

        // doing raw i/o: copy any leftover input to output -- this assumes
        // that the output buffer is larger than the input buffer, which also
        // assures space for gzungetc()
        self.x_next = 0;
        let n = self.avail_in as usize;
        let (a, out) = (self.next_in, &mut self.outbuf);
        out[..n].copy_from_slice(&self.inbuf[a..a + n]);
        self.x_have = self.avail_in;
        self.avail_in = 0;
        self.how = COPY;
        0
    }

    /// Decompress from input to the provided next_out and avail_out. If
    /// `user` is given, output goes there (gz_read's direct path), else to
    /// the internal output buffer at next_out.
    fn gz_decomp(&mut self, mut user: Option<&mut [u8]>) -> i32 {
        let mut ret = Z_OK;

        // fill output buffer up to end of deflate stream
        let had = self.avail_out;
        loop {
            // get more input for inflate()
            if self.avail_in == 0 && self.gz_avail() == -1 {
                ret = self.e.err;
                break;
            }
            if self.avail_in == 0 {
                if !self.e.again {
                    self.gz_error(Z_BUF_ERROR, Some("unexpected end of file"));
                }
                break;
            }

            // decompress and handle errors
            {
                let input = &self.inbuf[self.next_in..self.next_in + self.avail_in as usize];
                let output: &mut [u8] = match user.as_deref_mut() {
                    Some(b) => &mut b[self.next_out..self.next_out + self.avail_out as usize],
                    None => {
                        &mut self.outbuf[self.next_out..self.next_out + self.avail_out as usize]
                    }
                };
                let mut io = Io::new(input, output);
                ret = self.strm.as_mut().unwrap().inflate(&mut io, Z_NO_FLUSH);
                self.next_in += io.next_in;
                self.avail_in = io.avail_in;
                self.next_out += io.next_out;
                self.avail_out = io.avail_out;
            }
            if self.avail_out < had {
                // any decompressed data marks this as a real gzip stream
                self.junk = 0;
            }
            if ret == Z_STREAM_ERROR || ret == Z_NEED_DICT {
                self.gz_error(Z_STREAM_ERROR, Some("internal error: inflate stream corrupt"));
                break;
            }
            if ret == Z_MEM_ERROR {
                self.gz_error(Z_MEM_ERROR, Some("out of memory"));
                break;
            }
            if ret == Z_DATA_ERROR {
                // deflate stream invalid
                if self.junk == 1 {
                    // trailing garbage is ok
                    self.avail_in = 0;
                    self.eof = true;
                    self.how = LOOK;
                    ret = Z_OK;
                    break;
                }
                let m = self
                    .strm
                    .as_ref()
                    .unwrap()
                    .msg
                    .unwrap_or("compressed data error");
                self.gz_error(Z_DATA_ERROR, Some(m));
                break;
            }
            if !(self.avail_out != 0 && ret != Z_STREAM_END) {
                break;
            }
        }

        // update available output
        self.x_have = had - self.avail_out;
        self.x_next = self.next_out - self.x_have as usize;

        // if the gzip stream completed successfully, look for another
        if ret == Z_STREAM_END {
            self.junk = 0;
            self.how = LOOK;
            return 0;
        }

        // return decompression status
        if ret != Z_OK {
            -1
        } else {
            0
        }
    }

    /// Fetch data and put it in the output buffer. Assumes x.have is 0.
    fn gz_fetch(&mut self) -> i32 {
        loop {
            match self.how {
                LOOK => {
                    // -> LOOK, COPY (only if never GZIP), or GZIP
                    if self.gz_look() == -1 {
                        return -1;
                    }
                    if self.how == LOOK {
                        return 0;
                    }
                }
                COPY => {
                    // -> COPY
                    let mut have = 0usize;
                    let len = (self.size as usize) << 1;
                    if self.gz_load(LoadDst::Out, 0, len, &mut have) == -1 {
                        self.x_have = have as u32;
                        return -1;
                    }
                    self.x_have = have as u32;
                    self.x_next = 0;
                    return 0;
                }
                GZIP => {
                    // -> GZIP or LOOK (if end of gzip stream)
                    self.avail_out = self.size << 1;
                    self.next_out = 0;
                    if self.gz_decomp(None) == -1 {
                        return -1;
                    }
                }
                _ => {
                    self.gz_error(Z_STREAM_ERROR, Some("state corrupt"));
                    return -1;
                }
            }
            if !(self.x_have == 0 && (!self.eof || self.avail_in != 0)) {
                break;
            }
        }
        0
    }

    /// Skip state->skip (> 0) uncompressed bytes of output.
    fn gz_skip(&mut self) -> i32 {
        // skip over len bytes or reach end-of-file, whichever comes first
        loop {
            // skip over whatever is in output buffer
            if self.x_have != 0 {
                let n = if (self.x_have as i64) > self.skip {
                    self.skip as u32
                } else {
                    self.x_have
                };
                self.x_have -= n;
                self.x_next += n as usize;
                self.pos += n as i64;
                self.skip -= n as i64;
            }
            // output buffer empty -- return if we're at the end of the input
            else if self.eof && self.avail_in == 0 {
                break;
            }
            // need more data to skip -- load up output buffer
            else {
                // get more output, looking for header if required
                if self.gz_fetch() == -1 {
                    return -1;
                }
            }
            if self.skip == 0 {
                break;
            }
        }
        0
    }

    /// Read len bytes into buf from file, or less than len up to the end of
    /// the input. Return the number of bytes read.
    fn gz_read(&mut self, buf: &mut [u8]) -> usize {
        let mut len = buf.len();
        let mut off = 0usize;

        // if len is zero, avoid unnecessary operations
        if len == 0 {
            return 0;
        }

        // process a skip request
        if self.skip != 0 && self.gz_skip() == -1 {
            return 0;
        }

        // get len bytes to buf, or less than len if at the end
        let mut got = 0usize;
        let mut err = 0i32;
        loop {
            // set n to the maximum amount of len that fits in an unsigned int
            let mut n: u32 = len.min(u32::MAX as usize) as u32;

            // first just try copying data from the output buffer
            if self.x_have != 0 {
                if self.x_have < n {
                    n = self.x_have;
                }
                let nn = n as usize;
                buf[off..off + nn].copy_from_slice(&self.outbuf[self.x_next..self.x_next + nn]);
                self.x_next += nn;
                self.x_have -= n;
                if self.e.err != Z_OK {
                    // caught deferred error from gz_fetch()
                    err = -1;
                }
            }
            // output buffer empty -- return if we're at the end of the input
            else if self.eof && self.avail_in == 0 {
                break;
            }
            // need output data -- for small len or new stream load up our
            // output buffer, so that gzgetc() can be fast
            else if self.how == LOOK || n < (self.size << 1) {
                // get more output, looking for header if required
                if self.gz_fetch() == -1 && self.x_have == 0 {
                    // if state->x.have != 0, error will be caught after copy
                    err = -1;
                }
                // no progress yet -- go back to copy above
                if len != 0 && err == 0 {
                    continue;
                }
                break;
            }
            // large len -- read directly into user buffer
            else if self.how == COPY {
                let have: usize;
                let nn = n as usize;
                let r = {
                    // read into user buffer
                    let mut tmp_off = 0usize;
                    let rr = self.r.as_mut().expect("open");
                    self.e.again = false;
                    let mut res: io::Result<usize> = Ok(1);
                    while tmp_off < nn {
                        let get = (nn - tmp_off).min(IO_MAX);
                        res = loop {
                            match rr.read(&mut buf[off + tmp_off..off + tmp_off + get]) {
                                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                                other => break other,
                            }
                        };
                        match &res {
                            Ok(0) | Err(_) => break,
                            Ok(k) => tmp_off += *k,
                        }
                    }
                    have = tmp_off;
                    res
                };
                n = have as u32;
                err = match r {
                    Err(e) => {
                        if e.kind() == io::ErrorKind::WouldBlock {
                            self.e.again = true;
                        }
                        if self.e.again && have != 0 {
                            0
                        } else {
                            let m = io_err_msg(&e);
                            self.gz_error(Z_ERRNO, Some(&m));
                            -1
                        }
                    }
                    Ok(0) => {
                        self.eof = true;
                        0
                    }
                    Ok(_) => 0,
                };
            }
            // large len -- decompress directly into user buffer
            else {
                // state->how == GZIP
                self.avail_out = n;
                self.next_out = 0;
                let nn = n as usize;
                err = self.gz_decomp(Some(&mut buf[off..off + nn]));
                n = self.x_have;
                self.x_have = 0;
            }

            // update progress
            len -= n as usize;
            off += n as usize;
            got += n as usize;
            self.pos += n as i64;
            if !(len != 0 && err == 0) {
                break;
            }
        }

        // note read past eof
        if len != 0 && self.eof {
            self.past = true;
        }

        // return number of bytes read into user buffer
        got
    }

    fn readable(&self) -> bool {
        self.r.is_some() && (self.e.err == Z_OK || self.e.err == Z_BUF_ERROR || self.e.again)
    }

    /// `gzread()`: returns the number of uncompressed bytes read (0 at end
    /// of file), or -1 on error.
    pub fn gzread(&mut self, buf: &mut [u8]) -> i32 {
        // check that there was no (serious) error
        if !self.readable() {
            return -1;
        }
        self.gz_error(Z_OK, None);

        // since an int is returned, make sure len fits in one
        if buf.len() > i32::MAX as usize {
            self.gz_error(Z_STREAM_ERROR, Some("request does not fit in an int"));
            return -1;
        }

        // read len or fewer bytes to buf
        let len = self.gz_read(buf);

        // check for an error
        if len == 0 {
            if self.e.err != Z_OK && self.e.err != Z_BUF_ERROR {
                return -1;
            }
            if self.e.again {
                // non-blocking input stalled after some input was read, but
                // no uncompressed bytes were produced
                self.gz_error(Z_ERRNO, Some("Resource temporarily unavailable"));
                return -1;
            }
        }
        len as i32
    }

    /// `gzfread()`: returns the number of full items read.
    pub fn gzfread(&mut self, buf: &mut [u8], size: usize, nitems: usize) -> usize {
        if !self.readable() {
            return 0;
        }
        self.gz_error(Z_OK, None);
        let len = match size.checked_mul(nitems) {
            Some(l) => l,
            None => {
                self.gz_error(Z_STREAM_ERROR, Some("request does not fit in a size_t"));
                return 0;
            }
        };
        if len == 0 {
            return 0;
        }
        self.gz_read(&mut buf[..len]) / size
    }

    /// `gzgetc()`: next byte or -1.
    pub fn gzgetc(&mut self) -> i32 {
        if !self.readable() {
            return -1;
        }
        self.gz_error(Z_OK, None);

        // try output buffer (no need to check for skip request)
        if self.x_have != 0 {
            self.x_have -= 1;
            self.pos += 1;
            let c = self.outbuf[self.x_next];
            self.x_next += 1;
            return c as i32;
        }

        // nothing there -- try gz_read()
        let mut b = [0u8; 1];
        if self.gz_read(&mut b) < 1 {
            -1
        } else {
            b[0] as i32
        }
    }

    /// `gzungetc()`
    pub fn gzungetc(&mut self, c: i32) -> i32 {
        if self.r.is_none() {
            return -1;
        }
        // in case this was just opened, set up the input buffer
        if self.how == LOOK && self.x_have == 0 {
            let _ = self.gz_look();
        }

        // check that there was no (serious) error
        if !self.readable() {
            return -1;
        }
        self.gz_error(Z_OK, None);

        // process a skip request
        if self.skip != 0 && self.gz_skip() == -1 {
            return -1;
        }

        // can't push EOF
        if c < 0 {
            return -1;
        }

        // if output buffer empty, put byte at end (allows more pushing)
        if self.x_have == 0 {
            self.x_have = 1;
            self.x_next = ((self.size as usize) << 1) - 1;
            self.outbuf[self.x_next] = c as u8;
            self.pos -= 1;
            self.past = false;
            return c;
        }

        // if no room, give up (must have already done a gzungetc())
        if self.x_have == (self.size << 1) {
            self.gz_error(Z_DATA_ERROR, Some("out of room to push characters"));
            return -1;
        }

        // slide output data if needed and insert byte before existing data
        if self.x_next == 0 {
            let have = self.x_have as usize;
            let total = (self.size as usize) << 1;
            self.outbuf.copy_within(0..have, total - have);
            self.x_next = total - have;
        }
        self.x_have += 1;
        self.x_next -= 1;
        self.outbuf[self.x_next] = c as u8;
        self.pos -= 1;
        self.past = false;
        c
    }

    /// `gzgets()`: read a line (including the newline) of at most
    /// `buf.len() - 1` bytes into `buf` and NUL-terminate it. Returns the
    /// number of bytes stored before the NUL, or `None` (C NULL).
    pub fn gzgets(&mut self, buf: &mut [u8]) -> Option<usize> {
        if buf.is_empty() || !self.readable() {
            return None;
        }
        self.gz_error(Z_OK, None);

        // process a skip request
        if self.skip != 0 && self.gz_skip() == -1 {
            return None;
        }

        // copy output up to a new line, len-1 bytes, or there is no more
        // output, whichever comes first
        let mut left = buf.len() - 1;
        let mut off = 0usize;
        if left != 0 {
            loop {
                // assure that something is in the output buffer
                if self.x_have == 0 && self.gz_fetch() == -1 {
                    break; // error
                }
                if self.x_have == 0 {
                    // end of file
                    self.past = true; // read past end
                    break; // return what we have
                }

                // look for end-of-line in current output buffer
                let mut n = (self.x_have as usize).min(left);
                let seg = &self.outbuf[self.x_next..self.x_next + n];
                let eol = seg.iter().position(|&b| b == b'\n');
                if let Some(p) = eol {
                    n = p + 1;
                }

                // copy through end-of-line, or remainder if not found
                buf[off..off + n].copy_from_slice(&self.outbuf[self.x_next..self.x_next + n]);
                self.x_have -= n as u32;
                self.x_next += n;
                self.pos += n as i64;
                left -= n;
                off += n;
                if !(left != 0 && eol.is_none()) {
                    break;
                }
            }
        }

        // append a terminating zero to the string; if nothing was read,
        // return NULL
        if off == 0 {
            return None;
        }
        buf[off] = 0;
        Some(off)
    }

    /// `gzdirect()`: true if transparent (not gzip) reading.
    pub fn gzdirect(&mut self) -> bool {
        // if the state is not known, but we can find out, then do so
        if self.how == LOOK && self.x_have == 0 {
            let _ = self.gz_look();
        }
        self.direct == 1
    }

    /// `gzeof()`
    pub fn gzeof(&self) -> bool {
        self.past
    }

    /// `gzerror()`: (message, error code).
    pub fn gzerror(&self) -> (String, i32) {
        (self.e.message(), self.e.err)
    }

    /// `gzclearerr()`
    pub fn gzclearerr(&mut self) {
        self.eof = false;
        self.past = false;
        self.gz_error(Z_OK, None);
    }

    /// `gzbuffer()`: must be called before the first read.
    pub fn gzbuffer(&mut self, size: u32) -> i32 {
        if self.size != 0 {
            return -1;
        }
        if size.wrapping_shl(1) < size {
            return -1; // need to be able to double it
        }
        self.want = size.max(8);
        0
    }

    /// `gztell()`
    pub fn gztell(&self) -> i64 {
        self.pos + if self.past { 0 } else { self.skip }
    }

    /// Forward-only `gzseek()` (SEEK_SET / SEEK_CUR); backward seeks need
    /// [`GzReader::gzrewind`] and therefore a seekable reader.
    pub fn gzseek_forward(&mut self, offset: i64, whence: i32) -> i64 {
        if self.e.err != Z_OK && self.e.err != Z_BUF_ERROR {
            return -1;
        }
        if whence != 0 && whence != 1 {
            return -1;
        }
        let mut offset = offset;
        if whence == 0 {
            offset -= self.pos;
        } else {
            offset += if self.past { 0 } else { self.skip };
            self.skip = 0;
        }
        if offset < 0 {
            return -1;
        }
        self.seek_skip(offset)
    }

    fn seek_skip(&mut self, mut offset: i64) -> i64 {
        // if reading, skip what's in output buffer (one less gzgetc() check)
        let n = if (self.x_have as i64) > offset {
            offset as u32
        } else {
            self.x_have
        };
        self.x_have -= n;
        self.x_next += n as usize;
        self.pos += n as i64;
        offset -= n as i64;

        // request skip (if not zero)
        self.skip = offset;
        self.pos + offset
    }

    /// `gzclose_r()`: returns `Z_BUF_ERROR` if the last read ended in the
    /// middle of a gzip stream, else `Z_OK`.
    pub fn gzclose(mut self) -> i32 {
        let err = if self.e.err == Z_BUF_ERROR {
            Z_BUF_ERROR
        } else {
            Z_OK
        };
        self.strm = None;
        self.r = None;
        err
    }

    /// The underlying reader.
    pub fn get_ref(&self) -> &R {
        self.r.as_ref().expect("open")
    }
}

impl<R: Read + Seek> GzReader<R> {
    /// `gzrewind()`
    pub fn gzrewind(&mut self) -> i32 {
        if self.e.err != Z_OK && self.e.err != Z_BUF_ERROR {
            return -1;
        }
        let r = self.r.as_mut().expect("open");
        if r.seek(SeekFrom::Start(0)).is_err() {
            return -1;
        }
        self.gz_reset();
        0
    }

    /// `gzseek()` including backward seeks (rewind and skip).
    pub fn gzseek(&mut self, offset: i64, whence: i32) -> i64 {
        if self.e.err != Z_OK && self.e.err != Z_BUF_ERROR {
            return -1;
        }
        if whence != 0 && whence != 1 {
            return -1;
        }
        let mut offset = offset;
        if whence == 0 {
            offset -= self.pos;
        } else {
            offset += if self.past { 0 } else { self.skip };
            self.skip = 0;
        }

        // if within raw area while reading, just go there
        if self.how == COPY && self.pos + offset >= 0 {
            let r = self.r.as_mut().expect("open");
            if r
                .seek(SeekFrom::Current(offset - self.x_have as i64))
                .is_err()
            {
                return -1;
            }
            self.x_have = 0;
            self.eof = false;
            self.past = false;
            self.skip = 0;
            self.gz_error(Z_OK, None);
            self.avail_in = 0;
            self.pos += offset;
            return self.pos;
        }

        // calculate skip amount, rewinding if needed for back seek
        if offset < 0 {
            offset += self.pos;
            if offset < 0 {
                return -1; // before start of file!
            }
            if self.gzrewind() == -1 {
                return -1;
            }
        }
        self.seek_skip(offset)
    }
}

impl GzReader<std::fs::File> {
    /// `gzopen(path, "rb")`.
    pub fn open<P: AsRef<std::path::Path>>(path: P) -> io::Result<Self> {
        let f = std::fs::File::open(path.as_ref())?;
        let name = path.as_ref().to_string_lossy().into_owned();
        Ok(GzReader::with_mode(f, "rb", &name).expect("valid mode"))
    }
}

#[derive(Clone, Copy)]
enum LoadDst {
    In,
    Out,
}

impl<R: Read> Read for GzReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = buf.len().min(i32::MAX as usize);
        let n = self.gzread(&mut buf[..len]);
        if n < 0 {
            let (m, _) = self.gzerror();
            Err(io::Error::new(io::ErrorKind::InvalidData, m))
        } else {
            Ok(n as usize)
        }
    }
}

impl<W: Write> Write for GzWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let len = buf.len().min(i32::MAX as usize);
        let n = self.gzwrite(&buf[..len]);
        if n <= 0 {
            let (m, _) = self.gzerror();
            Err(io::Error::other(m))
        } else {
            Ok(n as usize)
        }
    }
    /// Note: does not call gzflush() (that would change the compressed
    /// bytes); it only flushes the underlying writer.
    fn flush(&mut self) -> io::Result<()> {
        match self.w.as_mut() {
            Some(w) => w.flush(),
            None => Ok(()),
        }
    }
}

#[allow(dead_code)]
fn _assert_codes() {
    let _ = (Z_STREAM_END, Z_DEFAULT_COMPRESSION);
}
