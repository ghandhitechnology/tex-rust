//! Port of deflate.c (zlib 1.3.2).
//!
//! The state machine, window management, hash chains, lazy matching and the
//! stored-block logic follow the C code statement by statement, so that the
//! same sequence of calls with the same input and output buffer sizes produces
//! the same bytes, return codes and counters as C zlib 1.3.2.

use crate::adler32::adler32;
use crate::crc32::{crc32, crc32_z};
use crate::trees::{CtData, TreeWork, BL_CODES};
use crate::{
    GzHeader, Z_BLOCK, Z_BUF_ERROR, Z_DATA_ERROR, Z_DEFAULT_COMPRESSION, Z_DEFLATED, Z_FILTERED,
    Z_FINISH, Z_FIXED, Z_FULL_FLUSH, Z_HUFFMAN_ONLY, Z_NO_FLUSH, Z_OK, Z_PARTIAL_FLUSH, Z_RLE,
    Z_STREAM_END, Z_STREAM_ERROR, Z_UNKNOWN,
};

pub(crate) const LENGTH_CODES: usize = 29;
pub(crate) const LITERALS: usize = 256;
pub(crate) const L_CODES: usize = LITERALS + 1 + LENGTH_CODES;
pub(crate) const D_CODES: usize = 30;
pub(crate) const HEAP_SIZE: usize = 2 * L_CODES + 1;
pub(crate) const MAX_BITS: usize = 15;

pub(crate) const INIT_STATE: i32 = 42;
pub(crate) const GZIP_STATE: i32 = 57;
pub(crate) const EXTRA_STATE: i32 = 69;
pub(crate) const NAME_STATE: i32 = 73;
pub(crate) const COMMENT_STATE: i32 = 91;
pub(crate) const HCRC_STATE: i32 = 103;
pub(crate) const BUSY_STATE: i32 = 113;
pub(crate) const FINISH_STATE: i32 = 666;

pub(crate) const MIN_MATCH: u32 = 3;
pub(crate) const MAX_MATCH: u32 = 258;
pub(crate) const MIN_LOOKAHEAD: u32 = MAX_MATCH + MIN_MATCH + 1;
const WIN_INIT: usize = MAX_MATCH as usize;
const NIL: u32 = 0;
const TOO_FAR: u32 = 4096;
const MAX_STORED: u32 = 65535;
const PRESET_DICT: u32 = 0x20;

/// `MAX_MEM_LEVEL` from zconf.h.
pub const MAX_MEM_LEVEL: i32 = 9;
/// `MAX_WBITS` from zconf.h.
pub const MAX_WBITS: i32 = 15;
/// `DEF_MEM_LEVEL` from zutil.h.
pub const DEF_MEM_LEVEL: i32 = 8;

/// `OS_CODE` written into gzip headers. zlib uses 10 (TOPS-20/NTFS) when
/// compiled for Windows (`_WIN32` makes zconf.h define `WIN32`), which is
/// what TeX Live's Windows binaries write. Other targets use their zutil.h
/// value (Unix 3, macOS 19).
#[cfg(windows)]
pub const OS_CODE: u8 = 10;
#[cfg(target_os = "macos")]
pub const OS_CODE: u8 = 19;
#[cfg(not(any(windows, target_os = "macos")))]
pub const OS_CODE: u8 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BlockState {
    NeedMore,
    BlockDone,
    FinishStarted,
    FinishDone,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Func {
    Stored,
    Fast,
    Slow,
}

struct Config {
    good_length: u16,
    max_lazy: u16,
    nice_length: u16,
    max_chain: u16,
    func: Func,
}

const fn cfg(good: u16, lazy: u16, nice: u16, chain: u16, func: Func) -> Config {
    Config {
        good_length: good,
        max_lazy: lazy,
        nice_length: nice,
        max_chain: chain,
        func,
    }
}

static CONFIGURATION_TABLE: [Config; 10] = [
    /*      good lazy nice chain */
    /* 0 */ cfg(0, 0, 0, 0, Func::Stored), // store only
    /* 1 */ cfg(4, 4, 8, 4, Func::Fast), // max speed, no lazy matches
    /* 2 */ cfg(4, 5, 16, 8, Func::Fast),
    /* 3 */ cfg(4, 6, 32, 32, Func::Fast),
    /* 4 */ cfg(4, 4, 16, 16, Func::Slow), // lazy matches
    /* 5 */ cfg(8, 16, 32, 32, Func::Slow),
    /* 6 */ cfg(8, 16, 128, 128, Func::Slow),
    /* 7 */ cfg(8, 32, 128, 256, Func::Slow),
    /* 8 */ cfg(32, 128, 258, 1024, Func::Slow),
    /* 9 */ cfg(32, 258, 258, 4096, Func::Slow), // max compression
];

/// rank Z_BLOCK between Z_NO_FLUSH and Z_PARTIAL_FLUSH
#[inline]
fn rank(f: i32) -> i32 {
    (f * 2) - if f > 4 { 9 } else { 0 }
}

/// The caller's buffers for one call, i.e. `next_in`/`avail_in` and
/// `next_out`/`avail_out` of `z_stream`, expressed as indices.
pub(crate) struct Io<'a> {
    pub input: &'a [u8],
    pub next_in: usize,
    pub avail_in: u32,
    pub output: &'a mut [u8],
    pub next_out: usize,
    pub avail_out: u32,
}

impl<'a> Io<'a> {
    pub fn new(input: &'a [u8], output: &'a mut [u8]) -> Self {
        let avail_in = input.len().min(u32::MAX as usize) as u32;
        let avail_out = output.len().min(u32::MAX as usize) as u32;
        Io {
            input,
            next_in: 0,
            avail_in,
            output,
            next_out: 0,
            avail_out,
        }
    }
}

/// `deflate_state` plus the `z_stream` fields deflate maintains
/// (`total_in`, `total_out`, `adler`, `data_type`, `msg`).
#[derive(Clone)]
pub(crate) struct DeflateState {
    // z_stream fields
    pub total_in: u64,
    pub total_out: u64,
    pub adler: u32,
    pub data_type: i32,
    pub msg: Option<&'static str>,

    pub status: i32,
    pub pending_buf: Vec<u8>,
    pub pending_buf_size: usize,
    pub pending_out: usize,
    pub pending: usize,
    pub wrap: i32,
    pub gzhead: Option<GzHeader>,
    pub gzindex: usize,
    #[allow(dead_code)] // kept for fidelity with deflate_state
    pub method: u8,
    pub last_flush: i32,

    pub w_size: u32,
    pub w_bits: u32,
    pub w_mask: u32,
    pub window: Vec<u8>,
    pub window_size: usize,
    pub prev: Vec<u16>,
    pub head: Vec<u16>,
    pub ins_h: u32,
    #[allow(dead_code)]
    pub hash_size: u32,
    pub hash_bits: u32,
    pub hash_mask: u32,
    pub hash_shift: u32,
    pub block_start: i64,
    pub match_length: u32,
    pub prev_match: u32,
    pub match_available: bool,
    pub strstart: u32,
    pub match_start: u32,
    pub lookahead: u32,
    pub prev_length: u32,
    pub max_chain_length: u32,
    pub max_lazy_match: u32,
    pub level: i32,
    pub strategy: i32,
    pub good_match: u32,
    pub nice_match: i32,

    pub dyn_ltree: [CtData; HEAP_SIZE],
    pub dyn_dtree: [CtData; 2 * D_CODES + 1],
    pub bl_tree: [CtData; 2 * BL_CODES + 1],
    pub l_max_code: i32,
    pub d_max_code: i32,
    pub bl_max_code: i32,
    pub tw: TreeWork,

    /// Offset of `sym_buf` inside `pending_buf` (the two are overlaid, as in C).
    pub sym_buf: usize,
    #[allow(dead_code)]
    pub lit_bufsize: u32,
    pub sym_next: u32,
    pub sym_end: u32,
    pub matches: u32,
    pub insert: u32,

    pub bi_buf: u16,
    pub bi_valid: i32,
    pub bi_used: i32,
    pub high_water: usize,
    pub slid: bool,
}

/// `FLUSH_BLOCK(s, last)`: flush the current block and return early if the
/// output buffer is full.
macro_rules! flush_block {
    ($s:expr, $io:expr, $last:expr) => {{
        $s.flush_block_only($io, $last);
        if $io.avail_out == 0 {
            return if $last {
                BlockState::FinishStarted
            } else {
                BlockState::NeedMore
            };
        }
    }};
}

impl DeflateState {
    /// `deflateInit2_()`: returns the state or a zlib error code.
    pub(crate) fn init2(
        level: i32,
        method: i32,
        window_bits: i32,
        mem_level: i32,
        strategy: i32,
    ) -> Result<Box<DeflateState>, i32> {
        let mut level = level;
        let mut window_bits = window_bits;
        let mut wrap = 1;
        if level == Z_DEFAULT_COMPRESSION {
            level = 6;
        }
        if window_bits < 0 {
            // suppress zlib wrapper
            wrap = 0;
            if window_bits < -15 {
                return Err(Z_STREAM_ERROR);
            }
            window_bits = -window_bits;
        } else if window_bits > 15 {
            wrap = 2; // write gzip wrapper instead
            window_bits -= 16;
        }
        if mem_level < 1
            || mem_level > MAX_MEM_LEVEL
            || method != Z_DEFLATED
            || window_bits < 8
            || window_bits > 15
            || level < 0
            || level > 9
            || strategy < 0
            || strategy > Z_FIXED
            || (window_bits == 8 && wrap != 1)
        {
            return Err(Z_STREAM_ERROR);
        }
        if window_bits == 8 {
            window_bits = 9; // until 256-byte window bug fixed
        }
        let w_bits = window_bits as u32;
        let w_size = 1u32 << w_bits;
        let hash_bits = mem_level as u32 + 7;
        let hash_size = 1u32 << hash_bits;
        let lit_bufsize = 1u32 << (mem_level + 6); // 16K elements by default
        let pending_buf_size = lit_bufsize as usize * 4;

        let mut s = Box::new(DeflateState {
            total_in: 0,
            total_out: 0,
            adler: 0,
            data_type: Z_UNKNOWN,
            msg: None,
            status: INIT_STATE,
            // LIT_BUFS == 4 (no LIT_MEM)
            pending_buf: vec![0u8; pending_buf_size],
            pending_buf_size,
            pending_out: 0,
            pending: 0,
            wrap,
            gzhead: None,
            gzindex: 0,
            method: method as u8,
            last_flush: 0,
            w_size,
            w_bits,
            w_mask: w_size - 1,
            window: vec![0u8; 2 * w_size as usize],
            window_size: 0,
            prev: vec![0u16; w_size as usize],
            head: vec![0u16; hash_size as usize],
            ins_h: 0,
            hash_size,
            hash_bits,
            hash_mask: hash_size - 1,
            hash_shift: (hash_bits + MIN_MATCH - 1) / MIN_MATCH,
            block_start: 0,
            match_length: 0,
            prev_match: 0,
            match_available: false,
            strstart: 0,
            match_start: 0,
            lookahead: 0,
            prev_length: 0,
            max_chain_length: 0,
            max_lazy_match: 0,
            level,
            strategy,
            good_match: 0,
            nice_match: 0,
            dyn_ltree: [CtData::default(); HEAP_SIZE],
            dyn_dtree: [CtData::default(); 2 * D_CODES + 1],
            bl_tree: [CtData::default(); 2 * BL_CODES + 1],
            l_max_code: 0,
            d_max_code: 0,
            bl_max_code: 0,
            tw: TreeWork::new(),
            sym_buf: lit_bufsize as usize,
            lit_bufsize,
            sym_next: 0,
            sym_end: (lit_bufsize - 1) * 3,
            matches: 0,
            insert: 0,
            bi_buf: 0,
            bi_valid: 0,
            bi_used: 0,
            high_water: 0,
            slid: false,
        });
        // We avoid equality with lit_bufsize*3 because of wraparound at 64K on
        // 16 bit machines and because stored blocks are restricted to 64K-1
        // bytes.
        s.reset();
        Ok(s)
    }

    #[inline(always)]
    fn max_dist(&self) -> u32 {
        self.w_size - MIN_LOOKAHEAD
    }

    /// `CLEAR_HASH(s)`
    fn clear_hash(&mut self) {
        for h in self.head.iter_mut() {
            *h = 0;
        }
        self.slid = false;
    }

    /// Slide the hash table when sliding the window down.
    fn slide_hash(&mut self) {
        let wsize = self.w_size;
        for p in self.head.iter_mut() {
            let m = *p as u32;
            *p = if m >= wsize { (m - wsize) as u16 } else { NIL as u16 };
        }
        for p in self.prev.iter_mut() {
            let m = *p as u32;
            *p = if m >= wsize { (m - wsize) as u16 } else { NIL as u16 };
        }
        self.slid = true;
    }

    /// `UPDATE_HASH(s, h, c)`
    #[inline(always)]
    fn update_hash(&self, h: u32, c: u8) -> u32 {
        ((h << self.hash_shift) ^ c as u32) & self.hash_mask
    }

    /// `INSERT_STRING(s, str, match_head)`: returns match_head.
    #[inline(always)]
    fn insert_string(&mut self, str: u32) -> u32 {
        self.ins_h = self.update_hash(self.ins_h, self.window[(str + (MIN_MATCH - 1)) as usize]);
        let h = self.head[self.ins_h as usize];
        self.prev[(str & self.w_mask) as usize] = h;
        self.head[self.ins_h as usize] = str as u16;
        h as u32
    }

    /// `read_buf()` into the window at offset `dst`.
    fn read_buf_window(&mut self, io: &mut Io, dst: usize, size: u32) -> u32 {
        let mut len = io.avail_in;
        if len > size {
            len = size;
        }
        if len == 0 {
            return 0;
        }
        io.avail_in -= len;
        let n = len as usize;
        self.window[dst..dst + n].copy_from_slice(&io.input[io.next_in..io.next_in + n]);
        if self.wrap == 1 {
            self.adler = adler32(self.adler, &self.window[dst..dst + n]);
        } else if self.wrap == 2 {
            self.adler = crc32(self.adler, &self.window[dst..dst + n]);
        }
        io.next_in += n;
        self.total_in += len as u64;
        len
    }

    /// `read_buf(s->strm, s->strm->next_out, len)`: copy input directly to
    /// the output buffer (deflate_stored). Does not advance next_out.
    fn read_buf_output(&mut self, io: &mut Io, size: u32) -> u32 {
        let mut len = io.avail_in;
        if len > size {
            len = size;
        }
        if len == 0 {
            return 0;
        }
        io.avail_in -= len;
        let n = len as usize;
        let src = &io.input[io.next_in..io.next_in + n];
        io.output[io.next_out..io.next_out + n].copy_from_slice(src);
        if self.wrap == 1 {
            self.adler = adler32(self.adler, src);
        } else if self.wrap == 2 {
            self.adler = crc32(self.adler, src);
        }
        io.next_in += n;
        self.total_in += len as u64;
        len
    }

    /// Fill the window when the lookahead becomes insufficient.
    fn fill_window(&mut self, io: &mut Io) {
        let wsize = self.w_size;
        loop {
            let mut more =
                (self.window_size - self.lookahead as usize - self.strstart as usize) as u32;

            // If the window is almost full and there is insufficient
            // lookahead, move the upper half to the lower one to make room in
            // the upper half.
            if self.strstart >= wsize + self.max_dist() {
                let n = wsize.wrapping_sub(more) as usize;
                let ws = wsize as usize;
                self.window.copy_within(ws..ws + n, 0);
                self.match_start = self.match_start.wrapping_sub(wsize);
                self.strstart -= wsize; // we now have strstart >= MAX_DIST
                self.block_start -= wsize as i64;
                if self.insert > self.strstart {
                    self.insert = self.strstart;
                }
                self.slide_hash();
                more = more.wrapping_add(wsize);
            }
            if io.avail_in == 0 {
                break;
            }

            let dst = (self.strstart + self.lookahead) as usize;
            let n = self.read_buf_window(io, dst, more);
            self.lookahead += n;

            // Initialize the hash value now that we have some input:
            if self.lookahead + self.insert >= MIN_MATCH {
                let mut str = self.strstart - self.insert;
                self.ins_h = self.window[str as usize] as u32;
                self.ins_h = self.update_hash(self.ins_h, self.window[str as usize + 1]);
                while self.insert != 0 {
                    self.ins_h =
                        self.update_hash(self.ins_h, self.window[(str + MIN_MATCH - 1) as usize]);
                    self.prev[(str & self.w_mask) as usize] = self.head[self.ins_h as usize];
                    self.head[self.ins_h as usize] = str as u16;
                    str += 1;
                    self.insert -= 1;
                    if self.lookahead + self.insert < MIN_MATCH {
                        break;
                    }
                }
            }
            // If the whole input has less than MIN_MATCH bytes, ins_h is
            // garbage, but this is not important since only literal bytes will
            // be emitted.

            if !(self.lookahead < MIN_LOOKAHEAD && io.avail_in != 0) {
                break;
            }
        }

        // If the WIN_INIT bytes after the end of the current data have never
        // been written, then zero those bytes in order to avoid memory check
        // reports of the use of uninitialized bytes by the longest match
        // routines. (The Rust window is zero-initialized; we still track and
        // zero exactly as C does so stale data is handled identically.)
        if self.high_water < self.window_size {
            let curr = self.strstart as usize + self.lookahead as usize;
            if self.high_water < curr {
                // Previous high water mark below current data -- zero WIN_INIT
                // bytes or up to end of window, whichever is less.
                let mut init = self.window_size - curr;
                if init > WIN_INIT {
                    init = WIN_INIT;
                }
                self.window[curr..curr + init].fill(0);
                self.high_water = curr + init;
            } else if self.high_water < curr + WIN_INIT {
                // High water mark at or above current data, but below current
                // data plus WIN_INIT -- zero out to current data plus WIN_INIT,
                // or up to end of window, whichever is less.
                let mut init = curr + WIN_INIT - self.high_water;
                if init > self.window_size - self.high_water {
                    init = self.window_size - self.high_water;
                }
                let hw = self.high_water;
                self.window[hw..hw + init].fill(0);
                self.high_water += init;
            }
        }
    }

    /// `deflateResetKeep()`
    pub(crate) fn reset_keep(&mut self) {
        self.total_in = 0;
        self.total_out = 0;
        self.msg = None;
        self.data_type = Z_UNKNOWN;
        self.pending = 0;
        self.pending_out = 0;
        if self.wrap < 0 {
            self.wrap = -self.wrap; // was made negative by deflate(..., Z_FINISH);
        }
        self.status = if self.wrap == 2 { GZIP_STATE } else { INIT_STATE };
        self.adler = if self.wrap == 2 { crc32_z(0, None) } else { 1 };
        self.last_flush = -2;
        self.tr_init();
    }

    /// Initialize the "longest match" routines for a new zlib stream.
    fn lm_init(&mut self) {
        self.window_size = 2 * self.w_size as usize;
        self.clear_hash();
        // Set the default configuration parameters:
        let c = &CONFIGURATION_TABLE[self.level as usize];
        self.max_lazy_match = c.max_lazy as u32;
        self.good_match = c.good_length as u32;
        self.nice_match = c.nice_length as i32;
        self.max_chain_length = c.max_chain as u32;

        self.strstart = 0;
        self.block_start = 0;
        self.lookahead = 0;
        self.insert = 0;
        self.match_length = MIN_MATCH - 1;
        self.prev_length = MIN_MATCH - 1;
        self.match_available = false;
        self.ins_h = 0;
    }

    /// `deflateReset()`
    pub(crate) fn reset(&mut self) {
        self.reset_keep();
        self.lm_init();
    }

    /// `deflateSetDictionary()`
    pub(crate) fn set_dictionary(&mut self, dictionary: &[u8]) -> i32 {
        let wrap = self.wrap;
        if wrap == 2 || (wrap == 1 && self.status != INIT_STATE) || self.lookahead != 0 {
            return Z_STREAM_ERROR;
        }
        let mut dict = dictionary;
        // when using zlib wrappers, compute Adler-32 for provided dictionary
        if wrap == 1 {
            self.adler = adler32(self.adler, dict);
        }
        self.wrap = 0; // avoid computing Adler-32 in read_buf

        // if dictionary would fill window, just replace the history
        if dict.len() as u64 >= self.w_size as u64 {
            if wrap == 0 {
                // already empty otherwise
                self.clear_hash();
                self.strstart = 0;
                self.block_start = 0;
                self.insert = 0;
            }
            dict = &dict[dict.len() - self.w_size as usize..]; // use the tail
        }

        // insert dictionary into window and hash
        let mut scratch = [0u8; 0];
        let mut io = Io::new(dict, &mut scratch);
        self.fill_window(&mut io);
        while self.lookahead >= MIN_MATCH {
            let mut str = self.strstart;
            let mut n = self.lookahead - (MIN_MATCH - 1);
            loop {
                self.ins_h =
                    self.update_hash(self.ins_h, self.window[(str + MIN_MATCH - 1) as usize]);
                self.prev[(str & self.w_mask) as usize] = self.head[self.ins_h as usize];
                self.head[self.ins_h as usize] = str as u16;
                str += 1;
                n -= 1;
                if n == 0 {
                    break;
                }
            }
            self.strstart = str;
            self.lookahead = MIN_MATCH - 1;
            self.fill_window(&mut io);
        }
        self.strstart += self.lookahead;
        self.block_start = self.strstart as i64;
        self.insert = self.lookahead;
        self.lookahead = 0;
        self.match_length = MIN_MATCH - 1;
        self.prev_length = MIN_MATCH - 1;
        self.match_available = false;
        self.wrap = wrap;
        Z_OK
    }

    /// `deflateGetDictionary()`
    pub(crate) fn get_dictionary(&self) -> Vec<u8> {
        let mut len = self.strstart + self.lookahead;
        if len > self.w_size {
            len = self.w_size;
        }
        let end = (self.strstart + self.lookahead) as usize;
        self.window[end - len as usize..end].to_vec()
    }

    /// `deflatePrime()`
    pub(crate) fn prime(&mut self, bits: i32, value: i32) -> i32 {
        // sym_buf < pending_out + ((Buf_size + 7) >> 3)
        if bits < 0 || bits > 16 || self.sym_buf < self.pending_out + 2 {
            return Z_BUF_ERROR;
        }
        let mut bits = bits;
        let mut value = value;
        loop {
            let mut put = 16 - self.bi_valid;
            if put > bits {
                put = bits;
            }
            let mask = ((1i64 << put) - 1) as i32;
            self.bi_buf |= (((value & mask) as u32) << self.bi_valid) as u16;
            self.bi_valid += put;
            self.tr_flush_bits();
            value >>= put;
            bits -= put;
            if bits == 0 {
                break;
            }
        }
        Z_OK
    }

    /// `deflateTune()`
    pub(crate) fn tune(&mut self, good_length: i32, max_lazy: i32, nice_length: i32, max_chain: i32) {
        self.good_match = good_length as u32;
        self.max_lazy_match = max_lazy as u32;
        self.nice_match = nice_length;
        self.max_chain_length = max_chain as u32;
    }

    /// `deflateBound_z()` for an initialized stream.
    pub(crate) fn bound(&self, source_len: u64) -> u64 {
        let (fixedlen, storelen) = bound_lens(source_len);
        // compute wrapper length
        let wraplen: u64 = match if self.wrap < 0 { -self.wrap } else { self.wrap } {
            0 => 0,                                    // raw deflate
            1 => 6 + if self.strstart != 0 { 4 } else { 0 }, // zlib wrapper
            2 => {
                // gzip wrapper
                let mut wl = 18u64;
                if let Some(h) = &self.gzhead {
                    // user-supplied gzip header
                    if h.extra.is_some() {
                        wl += 2 + h.extra_len as u64;
                    }
                    if let Some(n) = &h.name {
                        wl += c_strlen(n) as u64 + 1;
                    }
                    if let Some(c) = &h.comment {
                        wl += c_strlen(c) as u64 + 1;
                    }
                    if h.hcrc != 0 {
                        wl += 2;
                    }
                }
                wl
            }
            _ => 18,
        };

        // if not default parameters, return one of the conservative bounds
        if self.w_bits != 15 || self.hash_bits != 8 + 7 {
            let bound = if self.w_bits <= self.hash_bits && self.level != 0 {
                fixedlen
            } else {
                storelen
            };
            return bound.checked_add(wraplen).unwrap_or(u64::MAX);
        }

        // default settings: return tight bound for that case -- ~0.03% overhead
        // plus a small constant
        let bound = source_len
            .wrapping_add(source_len >> 12)
            .wrapping_add(source_len >> 14)
            .wrapping_add(source_len >> 25)
            .wrapping_add(13 - 6)
            .wrapping_add(wraplen);
        if bound < source_len {
            u64::MAX
        } else {
            bound
        }
    }

    /// Put a short in the pending buffer, MSB first.
    fn put_short_msb(&mut self, b: u32) {
        self.put_byte((b >> 8) as u8);
        self.put_byte((b & 0xff) as u8);
    }

    /// Flush as much pending output as possible.
    pub(crate) fn flush_pending(&mut self, io: &mut Io) {
        self.tr_flush_bits();
        let len = if self.pending > io.avail_out as usize {
            io.avail_out as usize
        } else {
            self.pending
        };
        if len == 0 {
            return;
        }
        io.output[io.next_out..io.next_out + len]
            .copy_from_slice(&self.pending_buf[self.pending_out..self.pending_out + len]);
        io.next_out += len;
        self.pending_out += len;
        self.total_out += len as u64;
        io.avail_out -= len as u32;
        self.pending -= len;
        if self.pending == 0 {
            self.pending_out = 0;
        }
    }

    /// `HCRC_UPDATE(beg)`
    fn hcrc_update(&mut self, beg: usize) {
        if self.gzhead.as_ref().map_or(false, |h| h.hcrc != 0) && self.pending > beg {
            self.adler = crc32(self.adler, &self.pending_buf[beg..self.pending]);
        }
    }

    /// `deflate()`
    pub(crate) fn deflate(&mut self, io: &mut Io, flush: i32) -> i32 {
        if flush > Z_BLOCK || flush < 0 {
            return Z_STREAM_ERROR;
        }
        if self.status == FINISH_STATE && flush != Z_FINISH {
            self.msg = Some(crate::err_msg(Z_STREAM_ERROR));
            return Z_STREAM_ERROR;
        }
        if io.avail_out == 0 {
            self.msg = Some(crate::err_msg(Z_BUF_ERROR));
            return Z_BUF_ERROR;
        }

        let old_flush = self.last_flush;
        self.last_flush = flush;

        // Flush as much pending output as possible
        if self.pending != 0 {
            self.flush_pending(io);
            if io.avail_out == 0 {
                // Since avail_out is 0, deflate will be called again with more
                // output space, but possibly with both pending and avail_in
                // equal to zero. There won't be anything to do, but this is not
                // an error situation so make sure we return OK instead of
                // BUF_ERROR at next call of deflate:
                self.last_flush = -1;
                return Z_OK;
            }
        // Make sure there is something to do and avoid duplicate consecutive
        // flushes. For repeated and useless calls with Z_FINISH, we keep
        // returning Z_STREAM_END instead of Z_BUF_ERROR.
        } else if io.avail_in == 0 && rank(flush) <= rank(old_flush) && flush != Z_FINISH {
            self.msg = Some(crate::err_msg(Z_BUF_ERROR));
            return Z_BUF_ERROR;
        }

        // User must not provide more input after the first FINISH:
        if self.status == FINISH_STATE && io.avail_in != 0 {
            self.msg = Some(crate::err_msg(Z_BUF_ERROR));
            return Z_BUF_ERROR;
        }

        // Write the header
        if self.status == INIT_STATE && self.wrap == 0 {
            self.status = BUSY_STATE;
        }
        if self.status == INIT_STATE {
            // zlib header
            let mut header: u32 = (Z_DEFLATED as u32 + ((self.w_bits - 8) << 4)) << 8;
            let level_flags: u32 = if self.strategy >= Z_HUFFMAN_ONLY || self.level < 2 {
                0
            } else if self.level < 6 {
                1
            } else if self.level == 6 {
                2
            } else {
                3
            };
            header |= level_flags << 6;
            if self.strstart != 0 {
                header |= PRESET_DICT;
            }
            header += 31 - (header % 31);

            self.put_short_msb(header);

            // Save the adler32 of the preset dictionary:
            if self.strstart != 0 {
                let a = self.adler;
                self.put_short_msb(a >> 16);
                self.put_short_msb(a & 0xffff);
            }
            self.adler = 1;
            self.status = BUSY_STATE;

            // Compression must start with an empty pending buffer
            self.flush_pending(io);
            if self.pending != 0 {
                self.last_flush = -1;
                return Z_OK;
            }
        }
        if self.status == GZIP_STATE {
            // gzip header
            self.adler = crc32_z(0, None);
            self.put_byte(31);
            self.put_byte(139);
            self.put_byte(8);
            let xfl = if self.level == 9 {
                2
            } else if self.strategy >= Z_HUFFMAN_ONLY || self.level < 2 {
                4
            } else {
                0
            };
            match self.gzhead.clone() {
                None => {
                    self.put_byte(0);
                    self.put_byte(0);
                    self.put_byte(0);
                    self.put_byte(0);
                    self.put_byte(0);
                    self.put_byte(xfl);
                    self.put_byte(OS_CODE);
                    self.status = BUSY_STATE;

                    // Compression must start with an empty pending buffer
                    self.flush_pending(io);
                    if self.pending != 0 {
                        self.last_flush = -1;
                        return Z_OK;
                    }
                }
                Some(h) => {
                    self.put_byte(
                        (if h.text != 0 { 1 } else { 0 })
                            + (if h.hcrc != 0 { 2 } else { 0 })
                            + (if h.extra.is_none() { 0 } else { 4 })
                            + (if h.name.is_none() { 0 } else { 8 })
                            + (if h.comment.is_none() { 0 } else { 16 }),
                    );
                    self.put_byte((h.time & 0xff) as u8);
                    self.put_byte(((h.time >> 8) & 0xff) as u8);
                    self.put_byte(((h.time >> 16) & 0xff) as u8);
                    self.put_byte(((h.time >> 24) & 0xff) as u8);
                    self.put_byte(xfl);
                    self.put_byte((h.os & 0xff) as u8);
                    if h.extra.is_some() {
                        self.put_byte((h.extra_len & 0xff) as u8);
                        self.put_byte(((h.extra_len >> 8) & 0xff) as u8);
                    }
                    if h.hcrc != 0 {
                        self.adler = crc32(self.adler, &self.pending_buf[..self.pending]);
                    }
                    self.gzindex = 0;
                    self.status = EXTRA_STATE;
                }
            }
        }
        if self.status == EXTRA_STATE {
            let extra = self
                .gzhead
                .as_ref()
                .and_then(|h| h.extra.clone().map(|e| (e, h.extra_len)));
            if let Some((extra, extra_len)) = extra {
                let mut beg = self.pending; // start of bytes to update crc
                let mut left = ((extra_len & 0xffff) as usize).wrapping_sub(self.gzindex);
                while self.pending + left > self.pending_buf_size {
                    let copy = self.pending_buf_size - self.pending;
                    let p = self.pending;
                    self.pending_buf[p..p + copy]
                        .copy_from_slice(&extra[self.gzindex..self.gzindex + copy]);
                    self.pending = self.pending_buf_size;
                    self.hcrc_update(beg);
                    self.gzindex += copy;
                    self.flush_pending(io);
                    if self.pending != 0 {
                        self.last_flush = -1;
                        return Z_OK;
                    }
                    beg = 0;
                    left -= copy;
                }
                let p = self.pending;
                self.pending_buf[p..p + left]
                    .copy_from_slice(&extra[self.gzindex..self.gzindex + left]);
                self.pending += left;
                self.hcrc_update(beg);
                self.gzindex = 0;
            }
            self.status = NAME_STATE;
        }
        if self.status == NAME_STATE {
            let name = self.gzhead.as_ref().and_then(|h| h.name.clone());
            if let Some(name) = name {
                let mut beg = self.pending; // start of bytes to update crc
                loop {
                    if self.pending == self.pending_buf_size {
                        self.hcrc_update(beg);
                        self.flush_pending(io);
                        if self.pending != 0 {
                            self.last_flush = -1;
                            return Z_OK;
                        }
                        beg = 0;
                    }
                    let val = name.get(self.gzindex).copied().unwrap_or(0);
                    self.gzindex += 1;
                    self.put_byte(val);
                    if val == 0 {
                        break;
                    }
                }
                self.hcrc_update(beg);
                self.gzindex = 0;
            }
            self.status = COMMENT_STATE;
        }
        if self.status == COMMENT_STATE {
            let comment = self.gzhead.as_ref().and_then(|h| h.comment.clone());
            if let Some(comment) = comment {
                let mut beg = self.pending; // start of bytes to update crc
                loop {
                    if self.pending == self.pending_buf_size {
                        self.hcrc_update(beg);
                        self.flush_pending(io);
                        if self.pending != 0 {
                            self.last_flush = -1;
                            return Z_OK;
                        }
                        beg = 0;
                    }
                    let val = comment.get(self.gzindex).copied().unwrap_or(0);
                    self.gzindex += 1;
                    self.put_byte(val);
                    if val == 0 {
                        break;
                    }
                }
                self.hcrc_update(beg);
            }
            self.status = HCRC_STATE;
        }
        if self.status == HCRC_STATE {
            if self.gzhead.as_ref().map_or(false, |h| h.hcrc != 0) {
                if self.pending + 2 > self.pending_buf_size {
                    self.flush_pending(io);
                    if self.pending != 0 {
                        self.last_flush = -1;
                        return Z_OK;
                    }
                }
                let a = self.adler;
                self.put_byte((a & 0xff) as u8);
                self.put_byte(((a >> 8) & 0xff) as u8);
                self.adler = crc32_z(0, None);
            }
            self.status = BUSY_STATE;

            // Compression must start with an empty pending buffer
            self.flush_pending(io);
            if self.pending != 0 {
                self.last_flush = -1;
                return Z_OK;
            }
        }

        // Start a new block or continue the current one.
        if io.avail_in != 0
            || self.lookahead != 0
            || (flush != Z_NO_FLUSH && self.status != FINISH_STATE)
        {
            let bstate = if self.level == 0 {
                self.deflate_stored(io, flush)
            } else if self.strategy == Z_HUFFMAN_ONLY {
                self.deflate_huff(io, flush)
            } else if self.strategy == Z_RLE {
                self.deflate_rle(io, flush)
            } else {
                match CONFIGURATION_TABLE[self.level as usize].func {
                    Func::Stored => self.deflate_stored(io, flush),
                    Func::Fast => self.deflate_fast(io, flush),
                    Func::Slow => self.deflate_slow(io, flush),
                }
            };

            if bstate == BlockState::FinishStarted || bstate == BlockState::FinishDone {
                self.status = FINISH_STATE;
            }
            if bstate == BlockState::NeedMore || bstate == BlockState::FinishStarted {
                if io.avail_out == 0 {
                    self.last_flush = -1; // avoid BUF_ERROR next call, see above
                }
                return Z_OK;
                // If flush != Z_NO_FLUSH && avail_out == 0, the next call of
                // deflate should use the same flush parameter to make sure that
                // the flush is complete. So we don't have to output an empty
                // block here, this will be done at next call. This also ensures
                // that for a very small output buffer, we emit at most one
                // empty block.
            }
            if bstate == BlockState::BlockDone {
                if flush == Z_PARTIAL_FLUSH {
                    self.tr_align();
                } else if flush != Z_BLOCK {
                    // FULL_FLUSH or SYNC_FLUSH
                    self.tr_stored_block(None, 0, false);
                    // For a full flush, this empty block will be recognized as
                    // a special marker by inflate_sync().
                    if flush == Z_FULL_FLUSH {
                        self.clear_hash(); // forget history
                        if self.lookahead == 0 {
                            self.strstart = 0;
                            self.block_start = 0;
                            self.insert = 0;
                        }
                    }
                }
                self.flush_pending(io);
                if io.avail_out == 0 {
                    self.last_flush = -1; // avoid BUF_ERROR at next call, see above
                    return Z_OK;
                }
            }
        }

        if flush != Z_FINISH {
            return Z_OK;
        }
        if self.wrap <= 0 {
            return Z_STREAM_END;
        }

        // Write the trailer
        if self.wrap == 2 {
            let a = self.adler;
            let t = self.total_in;
            self.put_byte((a & 0xff) as u8);
            self.put_byte(((a >> 8) & 0xff) as u8);
            self.put_byte(((a >> 16) & 0xff) as u8);
            self.put_byte(((a >> 24) & 0xff) as u8);
            self.put_byte((t & 0xff) as u8);
            self.put_byte(((t >> 8) & 0xff) as u8);
            self.put_byte(((t >> 16) & 0xff) as u8);
            self.put_byte(((t >> 24) & 0xff) as u8);
        } else {
            let a = self.adler;
            self.put_short_msb(a >> 16);
            self.put_short_msb(a & 0xffff);
        }
        self.flush_pending(io);
        // If avail_out is zero, the application will call deflate again to
        // flush the rest.
        if self.wrap > 0 {
            self.wrap = -self.wrap; // write the trailer only once!
        }
        if self.pending != 0 {
            Z_OK
        } else {
            Z_STREAM_END
        }
    }

    /// `deflateEnd()` status result.
    pub(crate) fn end_status(&self) -> i32 {
        if self.status == BUSY_STATE {
            Z_DATA_ERROR
        } else {
            Z_OK
        }
    }

    /// `deflateParams()`
    pub(crate) fn params(&mut self, io: &mut Io, level: i32, strategy: i32) -> i32 {
        let mut level = level;
        if level == Z_DEFAULT_COMPRESSION {
            level = 6;
        }
        if level < 0 || level > 9 || strategy < 0 || strategy > Z_FIXED {
            return Z_STREAM_ERROR;
        }
        let func = CONFIGURATION_TABLE[self.level as usize].func;

        if (strategy != self.strategy || func != CONFIGURATION_TABLE[level as usize].func)
            && self.last_flush != -2
        {
            // Flush the last buffer:
            let err = self.deflate(io, Z_BLOCK);
            if err == Z_STREAM_ERROR {
                return err;
            }
            if io.avail_in != 0
                || (self.strstart as i64 - self.block_start) + self.lookahead as i64 != 0
            {
                return Z_BUF_ERROR;
            }
        }
        if self.level != level {
            if self.level == 0 && self.matches != 0 {
                if self.matches == 1 {
                    self.slide_hash();
                } else {
                    self.clear_hash();
                }
                self.matches = 0;
            }
            self.level = level;
            let c = &CONFIGURATION_TABLE[level as usize];
            self.max_lazy_match = c.max_lazy as u32;
            self.good_match = c.good_length as u32;
            self.nice_match = c.nice_length as i32;
            self.max_chain_length = c.max_chain as u32;
        }
        self.strategy = strategy;
        Z_OK
    }

    /// Set match_start to the longest match starting at the given string and
    /// return its length. Matches shorter or equal to prev_length are
    /// discarded, in which case the result is equal to prev_length and
    /// match_start is garbage.
    #[inline]
    fn longest_match(&mut self, mut cur_match: u32) -> u32 {
        let mut chain_length = self.max_chain_length; // max hash chain length
        let strstart = self.strstart as usize;
        let mut best_len = self.prev_length as i32; // best match length so far
        let mut nice_match = self.nice_match; // stop if match long enough
        let max_dist = self.max_dist();
        let limit = if self.strstart > max_dist {
            self.strstart - max_dist
        } else {
            NIL
        };
        // Stop when cur_match becomes <= limit. To simplify the code, we
        // prevent matches with the string of window index 0.
        let wmask = self.w_mask;
        let window: &[u8] = &self.window;
        let prev: &[u16] = &self.prev;

        let at = |i: isize| -> u8 {
            // window[strstart + i] with C's pointer arithmetic (may be -1 for
            // a zero prev_length, which never happens in practice)
            if i < 0 && (strstart as isize + i) < 0 {
                0
            } else {
                window[(strstart as isize + i) as usize]
            }
        };
        let mut scan_end1 = at(best_len as isize - 1);
        let mut scan_end = at(best_len as isize);
        let scan0 = window[strstart];
        let scan1 = window[strstart + 1];

        // Do not waste too much time if we already have a good match:
        if self.prev_length >= self.good_match {
            chain_length >>= 2;
        }
        // Do not look for matches beyond the end of the input. This is
        // necessary to make deflate deterministic.
        if nice_match as u32 > self.lookahead {
            nice_match = self.lookahead as i32;
        }

        // All reads below stay within window[..window_size]: strstart <=
        // window_size - MIN_LOOKAHEAD and the scan stops at strstart + 258.
        assert!(strstart + MAX_MATCH as usize <= window.len());
        let mut match_start = self.match_start;
        loop {
            let m = cur_match as usize;
            // Skip to next match if the match length cannot increase or if
            // the match length is less than 2.
            let bl = best_len as isize;
            let skip = window[(m as isize + bl) as usize] != scan_end
                || window[(m as isize + bl - 1) as usize] != scan_end1
                || window[m] != scan0
                || window[m + 1] != scan1;
            if !skip {
                // It is not necessary to compare scan[2] and match[2] since
                // they are always equal when the other bytes match, given that
                // the hash keys are equal and that HASH_BITS >= 8. Compare from
                // offset 3; a mismatch at offset k gives length k, and equality
                // through offset 257 gives MAX_MATCH.
                let len = 3 + common_len(window, strstart + 3, m + 3, 255) as i32;
                if len > best_len {
                    match_start = cur_match;
                    best_len = len;
                    if len >= nice_match {
                        break;
                    }
                    scan_end1 = window[strstart + best_len as usize - 1];
                    scan_end = window[strstart + best_len as usize];
                }
            }
            cur_match = prev[(cur_match & wmask) as usize] as u32;
            if cur_match <= limit {
                break;
            }
            chain_length = chain_length.wrapping_sub(1);
            if chain_length == 0 {
                break;
            }
        }
        self.match_start = match_start;

        if best_len as u32 <= self.lookahead {
            best_len as u32
        } else {
            self.lookahead
        }
    }

    /// `FLUSH_BLOCK_ONLY(s, last)`
    fn flush_block_only(&mut self, io: &mut Io, last: bool) {
        let buf = if self.block_start >= 0 {
            Some(self.block_start as usize)
        } else {
            None
        };
        let stored_len = (self.strstart as i64 - self.block_start) as u64;
        self.tr_flush_block(buf, stored_len, last);
        self.block_start = self.strstart as i64;
        self.flush_pending(io);
    }

    /// Copy without compression as much as possible from the input stream,
    /// return the current block state.
    fn deflate_stored(&mut self, io: &mut Io, flush: i32) -> BlockState {
        // Smallest worthy block size when not flushing or finishing. By
        // default this is 32K. This can be as small as 507 bytes for
        // memLevel == 1. For large input and output buffers, the stored block
        // size will be larger.
        let mut min_block: u32 = (self.pending_buf_size - 5).min(self.w_size as usize) as u32;

        // Copy as many min_block or larger stored blocks directly to next_out
        // as possible. If flushing, copy the remaining available input to
        // next_out as stored blocks, if there is enough space.
        let mut last = false;
        let mut len: u32;
        let mut left: u32;
        let mut have: u32;
        let mut used: u32 = io.avail_in;
        loop {
            // Set len to the maximum size block that we can copy directly with
            // the available input data and output space. Set left to how much
            // of that would be copied from what's left in the window.
            len = MAX_STORED; // maximum deflate stored block length
            have = (self.bi_valid as u32 + 42) >> 3; // bytes in header
            if io.avail_out < have {
                // need room for header
                break;
            }
            // maximum stored block length that will fit in avail_out:
            have = io.avail_out - have;
            left = (self.strstart as i64 - self.block_start) as u32; // window bytes
            // (ulg is 32 bits on Windows)
            if len > left.wrapping_add(io.avail_in) {
                len = left.wrapping_add(io.avail_in); // limit len to the input
            }
            if len > have {
                len = have; // limit len to the output
            }

            // If the stored block would be less than min_block in length, or if
            // unable to copy all of the available input when flushing, then try
            // copying to the window and the pending buffer instead. Also don't
            // write an empty block when flushing -- deflate() does that.
            if len < min_block
                && ((len == 0 && flush != Z_FINISH)
                    || flush == Z_NO_FLUSH
                    || len != left.wrapping_add(io.avail_in))
            {
                break;
            }

            // Make a dummy stored block in pending to get the header bytes,
            // including any pending bits. This also updates the debugging
            // counts.
            last = flush == Z_FINISH && len == left.wrapping_add(io.avail_in);
            self.tr_stored_block(None, 0, last);

            // Replace the lengths in the dummy stored block with len.
            let p = self.pending;
            self.pending_buf[p - 4] = len as u8;
            self.pending_buf[p - 3] = (len >> 8) as u8;
            self.pending_buf[p - 2] = !len as u8;
            self.pending_buf[p - 1] = (!len >> 8) as u8;

            // Write the stored block header bytes.
            self.flush_pending(io);

            // Copy uncompressed bytes from the window to next_out.
            if left != 0 {
                if left > len {
                    left = len;
                }
                let bs = self.block_start as usize;
                let n = left as usize;
                io.output[io.next_out..io.next_out + n].copy_from_slice(&self.window[bs..bs + n]);
                io.next_out += n;
                io.avail_out -= left;
                self.total_out += left as u64;
                self.block_start += left as i64;
                len -= left;
            }

            // Copy uncompressed bytes directly from next_in to next_out,
            // updating the check value.
            if len != 0 {
                self.read_buf_output(io, len);
                io.next_out += len as usize;
                io.avail_out -= len;
                self.total_out += len as u64;
            }
            if last {
                break;
            }
        }

        // Update the sliding window with the last s->w_size bytes of the
        // copied data, or append all of the copied data to the existing window
        // if less than s->w_size bytes were copied. Also update the number of
        // bytes to insert in the hash tables, in the event that deflateParams()
        // switches to a non-zero compression level.
        used -= io.avail_in; // number of input bytes directly copied
        if used != 0 {
            // If any input was used, then no unused input remains in the
            // window, therefore s->block_start == s->strstart.
            if used >= self.w_size {
                // supplant the previous history
                self.matches = 2; // clear hash
                let ws = self.w_size as usize;
                self.window[..ws].copy_from_slice(&io.input[io.next_in - ws..io.next_in]);
                self.strstart = self.w_size;
                self.insert = self.strstart;
            } else {
                if (self.window_size - self.strstart as usize) <= used as usize {
                    // Slide the window down.
                    self.strstart -= self.w_size;
                    let ws = self.w_size as usize;
                    let ss = self.strstart as usize;
                    self.window.copy_within(ws..ws + ss, 0);
                    if self.matches < 2 {
                        self.matches += 1; // add a pending slide_hash()
                    }
                    if self.insert > self.strstart {
                        self.insert = self.strstart;
                    }
                }
                let ss = self.strstart as usize;
                let u = used as usize;
                self.window[ss..ss + u].copy_from_slice(&io.input[io.next_in - u..io.next_in]);
                self.strstart += used;
                self.insert += used.min(self.w_size - self.insert);
            }
            self.block_start = self.strstart as i64;
        }
        if self.high_water < self.strstart as usize {
            self.high_water = self.strstart as usize;
        }

        // If the last block was written to next_out, then done.
        if last {
            self.bi_used = 8;
            return BlockState::FinishDone;
        }

        // If flushing and all input has been consumed, then done.
        if flush != Z_NO_FLUSH
            && flush != Z_FINISH
            && io.avail_in == 0
            && self.strstart as i64 == self.block_start
        {
            return BlockState::BlockDone;
        }

        // Fill the window with any remaining input.
        have = (self.window_size - self.strstart as usize) as u32;
        if io.avail_in > have && self.block_start >= self.w_size as i64 {
            // Slide the window down.
            self.block_start -= self.w_size as i64;
            self.strstart -= self.w_size;
            let ws = self.w_size as usize;
            let ss = self.strstart as usize;
            self.window.copy_within(ws..ws + ss, 0);
            if self.matches < 2 {
                self.matches += 1; // add a pending slide_hash()
            }
            have += self.w_size; // more space now
            if self.insert > self.strstart {
                self.insert = self.strstart;
            }
        }
        if have > io.avail_in {
            have = io.avail_in;
        }
        if have != 0 {
            let ss = self.strstart as usize;
            self.read_buf_window(io, ss, have);
            self.strstart += have;
            self.insert += have.min(self.w_size - self.insert);
        }
        if self.high_water < self.strstart as usize {
            self.high_water = self.strstart as usize;
        }

        // There was not enough avail_out to write a complete worthy or flushed
        // stored block to next_out. Write a stored block to pending instead, if
        // we have enough input for a worthy block, or if flushing and there is
        // enough room for the remaining input as a stored block in the pending
        // buffer.
        have = (self.bi_valid as u32 + 42) >> 3; // bytes in header
        // maximum stored block length that will fit in pending:
        have = (self.pending_buf_size - have as usize).min(MAX_STORED as usize) as u32;
        min_block = have.min(self.w_size);
        left = (self.strstart as i64 - self.block_start) as u32;
        if left >= min_block
            || ((left != 0 || flush == Z_FINISH)
                && flush != Z_NO_FLUSH
                && io.avail_in == 0
                && left <= have)
        {
            len = left.min(have);
            last = flush == Z_FINISH && io.avail_in == 0 && len == left;
            let bs = self.block_start as usize;
            self.tr_stored_block(Some(bs), len, last);
            self.block_start += len as i64;
            self.flush_pending(io);
        }

        // We've done all we can with the available input and output.
        if last {
            self.bi_used = 8;
        }
        if last {
            BlockState::FinishStarted
        } else {
            BlockState::NeedMore
        }
    }

    /// Compress as much as possible from the input stream, return the
    /// current block state. No lazy evaluation of matches (levels 1-3).
    fn deflate_fast(&mut self, io: &mut Io, flush: i32) -> BlockState {
        let mut hash_head: u32; // head of the hash chain
        let mut bflush: bool; // set if current block must be flushed

        loop {
            // Make sure that we always have enough lookahead, except at the end
            // of the input file. We need MAX_MATCH bytes for the next match,
            // plus MIN_MATCH bytes to insert the string following the next
            // match.
            if self.lookahead < MIN_LOOKAHEAD {
                self.fill_window(io);
                if self.lookahead < MIN_LOOKAHEAD && flush == Z_NO_FLUSH {
                    return BlockState::NeedMore;
                }
                if self.lookahead == 0 {
                    break; // flush the current block
                }
            }

            // Insert the string window[strstart .. strstart + 2] in the
            // dictionary, and set hash_head to the head of the hash chain:
            hash_head = NIL;
            if self.lookahead >= MIN_MATCH {
                hash_head = self.insert_string(self.strstart);
            }

            // Find the longest match, discarding those <= prev_length. At this
            // point we have always match_length < MIN_MATCH
            if hash_head != NIL && self.strstart.wrapping_sub(hash_head) <= self.max_dist() {
                // To simplify the code, we prevent matches with the string of
                // window index 0 (in particular we have to avoid a match of the
                // string with itself at the start of the input file).
                self.match_length = self.longest_match(hash_head);
                // longest_match() sets match_start
            }
            if self.match_length >= MIN_MATCH {
                bflush = self.tr_tally_dist(
                    self.strstart - self.match_start,
                    self.match_length - MIN_MATCH,
                );

                self.lookahead -= self.match_length;

                // Insert new strings in the hash table only if the match length
                // is not too large. This saves time but degrades compression.
                if self.match_length <= self.max_lazy_match && self.lookahead >= MIN_MATCH {
                    self.match_length -= 1; // string at strstart already in table
                    loop {
                        self.strstart += 1;
                        self.insert_string(self.strstart);
                        // strstart never exceeds WSIZE-MAX_MATCH, so there are
                        // always MIN_MATCH bytes ahead.
                        self.match_length -= 1;
                        if self.match_length == 0 {
                            break;
                        }
                    }
                    self.strstart += 1;
                } else {
                    self.strstart += self.match_length;
                    self.match_length = 0;
                    self.ins_h = self.window[self.strstart as usize] as u32;
                    self.ins_h = self.update_hash(self.ins_h, self.window[self.strstart as usize + 1]);
                    // If lookahead < MIN_MATCH, ins_h is garbage, but it does
                    // not matter since it will be recomputed at next deflate
                    // call.
                }
            } else {
                // No match, output a literal byte
                let c = self.window[self.strstart as usize];
                bflush = self.tr_tally_lit(c);
                self.lookahead -= 1;
                self.strstart += 1;
            }
            if bflush {
                flush_block!(self, io, false);
            }
        }
        self.insert = if self.strstart < MIN_MATCH - 1 {
            self.strstart
        } else {
            MIN_MATCH - 1
        };
        if flush == Z_FINISH {
            flush_block!(self, io, true);
            return BlockState::FinishDone;
        }
        if self.sym_next != 0 {
            flush_block!(self, io, false);
        }
        BlockState::BlockDone
    }

    /// Same as above, but achieves better compression. We use a lazy
    /// evaluation for matches: a match is finally adopted only if there is no
    /// better match at the next window position.
    fn deflate_slow(&mut self, io: &mut Io, flush: i32) -> BlockState {
        let mut hash_head: u32; // head of hash chain
        let mut bflush: bool; // set if current block must be flushed

        // Process the input block.
        loop {
            // Make sure that we always have enough lookahead, except at the end
            // of the input file. We need MAX_MATCH bytes for the next match,
            // plus MIN_MATCH bytes to insert the string following the next
            // match.
            if self.lookahead < MIN_LOOKAHEAD {
                self.fill_window(io);
                if self.lookahead < MIN_LOOKAHEAD && flush == Z_NO_FLUSH {
                    return BlockState::NeedMore;
                }
                if self.lookahead == 0 {
                    break; // flush the current block
                }
            }

            // Insert the string window[strstart .. strstart + 2] in the
            // dictionary, and set hash_head to the head of the hash chain:
            hash_head = NIL;
            if self.lookahead >= MIN_MATCH {
                hash_head = self.insert_string(self.strstart);
            }

            // Find the longest match, discarding those <= prev_length.
            self.prev_length = self.match_length;
            self.prev_match = self.match_start;
            self.match_length = MIN_MATCH - 1;

            if hash_head != NIL
                && self.prev_length < self.max_lazy_match
                && self.strstart.wrapping_sub(hash_head) <= self.max_dist()
            {
                // To simplify the code, we prevent matches with the string of
                // window index 0 (in particular we have to avoid a match of the
                // string with itself at the start of the input file).
                self.match_length = self.longest_match(hash_head);
                // longest_match() sets match_start

                if self.match_length <= 5
                    && (self.strategy == Z_FILTERED
                        || (self.match_length == MIN_MATCH
                            && self.strstart.wrapping_sub(self.match_start) > TOO_FAR))
                {
                    // If prev_match is also MIN_MATCH, match_start is garbage
                    // but we will ignore the current match anyway.
                    self.match_length = MIN_MATCH - 1;
                }
            }
            // If there was a match at the previous step and the current match
            // is not better, output the previous match:
            if self.prev_length >= MIN_MATCH && self.match_length <= self.prev_length {
                let max_insert = self.strstart + self.lookahead - MIN_MATCH;
                // Do not insert strings in hash table beyond this.

                bflush = self.tr_tally_dist(
                    self.strstart.wrapping_sub(1).wrapping_sub(self.prev_match),
                    self.prev_length - MIN_MATCH,
                );

                // Insert in hash table all strings up to the end of the match.
                // strstart - 1 and strstart are already inserted. If there is
                // not enough lookahead, the last two strings are not inserted
                // in the hash table.
                self.lookahead -= self.prev_length - 1;
                self.prev_length -= 2;
                loop {
                    self.strstart += 1;
                    if self.strstart <= max_insert {
                        self.insert_string(self.strstart);
                    }
                    self.prev_length -= 1;
                    if self.prev_length == 0 {
                        break;
                    }
                }
                self.match_available = false;
                self.match_length = MIN_MATCH - 1;
                self.strstart += 1;

                if bflush {
                    flush_block!(self, io, false);
                }
            } else if self.match_available {
                // If there was no match at the previous position, output a
                // single literal. If there was a match but the current match is
                // longer, truncate the previous match to a single literal.
                let c = self.window[self.strstart as usize - 1];
                bflush = self.tr_tally_lit(c);
                if bflush {
                    self.flush_block_only(io, false);
                }
                self.strstart += 1;
                self.lookahead -= 1;
                if io.avail_out == 0 {
                    return BlockState::NeedMore;
                }
            } else {
                // There is no previous match to compare with, wait for the next
                // step to decide.
                self.match_available = true;
                self.strstart += 1;
                self.lookahead -= 1;
            }
        }
        if self.match_available {
            let c = self.window[self.strstart as usize - 1];
            self.tr_tally_lit(c);
            self.match_available = false;
        }
        self.insert = if self.strstart < MIN_MATCH - 1 {
            self.strstart
        } else {
            MIN_MATCH - 1
        };
        if flush == Z_FINISH {
            flush_block!(self, io, true);
            return BlockState::FinishDone;
        }
        if self.sym_next != 0 {
            flush_block!(self, io, false);
        }
        BlockState::BlockDone
    }

    /// For Z_RLE, simply look for runs of bytes, generate matches only of
    /// distance one. Do not maintain a hash table.
    fn deflate_rle(&mut self, io: &mut Io, flush: i32) -> BlockState {
        let mut bflush: bool;

        loop {
            // Make sure that we always have enough lookahead, except at the end
            // of the input file. We need MAX_MATCH bytes for the longest run,
            // plus one for the unrolled loop.
            if self.lookahead <= MAX_MATCH {
                self.fill_window(io);
                if self.lookahead <= MAX_MATCH && flush == Z_NO_FLUSH {
                    return BlockState::NeedMore;
                }
                if self.lookahead == 0 {
                    break; // flush the current block
                }
            }

            // See how many times the previous byte repeats
            self.match_length = 0;
            if self.lookahead >= MIN_MATCH && self.strstart > 0 {
                let s = self.strstart as usize;
                let w = &self.window;
                let prev = w[s - 1];
                if prev == w[s] && prev == w[s + 1] && prev == w[s + 2] {
                    // offsets 3..=257 compared one at a time in C; equality
                    // through 257 gives MAX_MATCH.
                    let mut k = 3usize;
                    while k < MAX_MATCH as usize && w[s + k] == prev {
                        k += 1;
                    }
                    self.match_length = k as u32;
                    if self.match_length > self.lookahead {
                        self.match_length = self.lookahead;
                    }
                }
            }

            // Emit match if have run of MIN_MATCH or longer, else emit literal
            if self.match_length >= MIN_MATCH {
                bflush = self.tr_tally_dist(1, self.match_length - MIN_MATCH);
                self.lookahead -= self.match_length;
                self.strstart += self.match_length;
                self.match_length = 0;
            } else {
                // No match, output a literal byte
                let c = self.window[self.strstart as usize];
                bflush = self.tr_tally_lit(c);
                self.lookahead -= 1;
                self.strstart += 1;
            }
            if bflush {
                flush_block!(self, io, false);
            }
        }
        self.insert = 0;
        if flush == Z_FINISH {
            flush_block!(self, io, true);
            return BlockState::FinishDone;
        }
        if self.sym_next != 0 {
            flush_block!(self, io, false);
        }
        BlockState::BlockDone
    }

    /// For Z_HUFFMAN_ONLY, do not look for matches. Do not maintain a hash
    /// table.
    fn deflate_huff(&mut self, io: &mut Io, flush: i32) -> BlockState {
        let mut bflush: bool;

        loop {
            // Make sure that we have a literal to write.
            if self.lookahead == 0 {
                self.fill_window(io);
                if self.lookahead == 0 {
                    if flush == Z_NO_FLUSH {
                        return BlockState::NeedMore;
                    }
                    break; // flush the current block
                }
            }

            // Output a literal byte
            self.match_length = 0;
            let c = self.window[self.strstart as usize];
            bflush = self.tr_tally_lit(c);
            self.lookahead -= 1;
            self.strstart += 1;
            if bflush {
                flush_block!(self, io, false);
            }
        }
        self.insert = 0;
        if flush == Z_FINISH {
            flush_block!(self, io, true);
            return BlockState::FinishDone;
        }
        if self.sym_next != 0 {
            flush_block!(self, io, false);
        }
        BlockState::BlockDone
    }
}

/// Number of equal bytes at `a` and `b` in `w`, at most `max`.
#[inline(always)]
fn common_len(w: &[u8], a: usize, b: usize, max: usize) -> usize {
    assert!(a + max <= w.len() && b + max <= w.len());
    let mut i = 0usize;
    // SAFETY: the assertion above bounds every read below.
    unsafe {
        let p = w.as_ptr();
        while i + 8 <= max {
            let x = (p.add(a + i) as *const u64).read_unaligned();
            let y = (p.add(b + i) as *const u64).read_unaligned();
            let d = x ^ y;
            if d != 0 {
                return i + (u64::from_le(d).trailing_zeros() as usize >> 3);
            }
            i += 8;
        }
        while i < max && *p.add(a + i) == *p.add(b + i) {
            i += 1;
        }
    }
    i
}

/// Length of a C string stored in a byte vector (up to the first NUL).
fn c_strlen(v: &[u8]) -> usize {
    v.iter().position(|&b| b == 0).unwrap_or(v.len())
}

/// Shared part of `deflateBound_z()`: (fixedlen, storelen).
fn bound_lens(source_len: u64) -> (u64, u64) {
    // upper bound for fixed blocks with 9-bit literals and length 255
    // (memLevel == 2, which is the lowest that may not use stored blocks) --
    // ~13% overhead plus a small constant
    let mut fixedlen = source_len
        .wrapping_add(source_len >> 3)
        .wrapping_add(source_len >> 8)
        .wrapping_add(source_len >> 9)
        .wrapping_add(4);
    if fixedlen < source_len {
        fixedlen = u64::MAX;
    }
    // upper bound for stored blocks with length 127 (memLevel == 1) -- ~4%
    // overhead plus a small constant
    let mut storelen = source_len
        .wrapping_add(source_len >> 5)
        .wrapping_add(source_len >> 7)
        .wrapping_add(source_len >> 11)
        .wrapping_add(7);
    if storelen < source_len {
        storelen = u64::MAX;
    }
    (fixedlen, storelen)
}

/// `deflateBound_z()` when the stream state is not available.
pub(crate) fn bound_no_state(source_len: u64) -> u64 {
    let (fixedlen, storelen) = bound_lens(source_len);
    let bound = fixedlen.max(storelen);
    bound.checked_add(18).unwrap_or(u64::MAX)
}
