//! Port of inflate.c (zlib 1.3.2): the inflate state machine.

use crate::adler32::adler32;
use crate::crc32::crc32;
use crate::deflate::Io;
use crate::inffast::inflate_fast;
use crate::inftrees::{inflate_table, Code, CodeType, ENOUGH};
use crate::tables::{DISTFIX, LENFIX};
use crate::{
    GzHeader, Z_BLOCK, Z_BUF_ERROR, Z_DATA_ERROR, Z_DEFLATED, Z_FINISH, Z_MEM_ERROR, Z_NEED_DICT,
    Z_OK, Z_STREAM_END, Z_STREAM_ERROR, Z_TREES,
};

/// Possible inflate modes between inflate() calls (same numbering as C).
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Mode {
    Head = 16180, // i: waiting for magic header
    Flags,        // i: waiting for method and flags (gzip)
    Time,         // i: waiting for modification time (gzip)
    Os,           // i: waiting for extra flags and operating system (gzip)
    ExLen,        // i: waiting for extra length (gzip)
    Extra,        // i: waiting for extra bytes (gzip)
    Name,         // i: waiting for end of file name (gzip)
    Comment,      // i: waiting for end of comment (gzip)
    Hcrc,         // i: waiting for header crc (gzip)
    DictId,       // i: waiting for dictionary check value
    Dict,         // waiting for inflateSetDictionary() call
    Type,         // i: waiting for type bits, including last-flag bit
    TypeDo,       // i: same, but skip check to exit inflate on new block
    Stored,       // i: waiting for stored size (length and complement)
    Copy_,        // i/o: same as COPY below, but only first time in
    Copy,         // i/o: waiting for input or output to copy stored block
    Table,        // i: waiting for dynamic block table lengths
    LenLens,      // i: waiting for code length code lengths
    CodeLens,     // i: waiting for length/lit and distance code lengths
    Len_,         // i: same as LEN below, but only first time in
    Len,          // i: waiting for length/lit/eob code
    LenExt,       // i: waiting for length extra bits
    Dist,         // i: waiting for distance code
    DistExt,      // i: waiting for distance extra bits
    Match,        // o: waiting for output space to copy string
    Lit,          // o: waiting for output space to write literal
    Check,        // i: waiting for 32-bit check value
    Length,       // i: waiting for 32-bit length (gzip)
    Done,         // finished check, done -- remain here until reset
    Bad,          // got a data error -- remain here until reset
    #[allow(dead_code)] // allocation failures abort in Rust
    Mem,          // got an inflate() memory error -- remain here until reset
    Sync,         // looking for synchronization bytes to restart inflate()
}

/// Where a decoding table lives: the static fixed tables or `codes[]`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TableRef {
    Fixed,
    Codes(usize),
}

static ORDER: [u16; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// `struct inflate_state` plus the `z_stream` fields inflate maintains.
#[derive(Clone)]
pub(crate) struct InflateState {
    // z_stream fields
    pub total_in: u64,
    pub total_out: u64,
    pub adler: u32,
    pub data_type: i32,
    pub msg: Option<&'static str>,

    pub mode: Mode,
    pub last: bool,
    pub wrap: i32,
    pub havedict: bool,
    pub flags: i32,
    pub dmax: u32,
    pub check: u32,
    pub total: u64,
    pub head: Option<GzHeader>,
    pub wbits: u32,
    pub wsize: u32,
    pub whave: u32,
    pub wnext: u32,
    /// Sliding window; empty means not allocated (`window == Z_NULL`).
    pub window: Vec<u8>,
    pub hold: u64,
    pub bits: u32,
    pub length: u32,
    pub offset: u32,
    pub extra: u32,
    pub lencode: TableRef,
    pub distcode: TableRef,
    pub lenbits: u32,
    pub distbits: u32,
    pub ncode: u32,
    pub nlen: u32,
    pub ndist: u32,
    pub have: u32,
    pub next: usize,
    pub lens: [u16; 320],
    pub work: [u16; 288],
    pub codes: Box<[Code; ENOUGH]>,
    pub sane: bool,
    pub back: i32,
    pub was: u32,
}

#[inline(always)]
fn zswap32(q: u64) -> u32 {
    let q = q as u32;
    q.swap_bytes()
}

impl InflateState {
    fn new_zeroed() -> InflateState {
        InflateState {
            total_in: 0,
            total_out: 0,
            adler: 0,
            data_type: 0,
            msg: None,
            mode: Mode::Head,
            last: false,
            wrap: 0,
            havedict: false,
            flags: 0,
            dmax: 0,
            check: 0,
            total: 0,
            head: None,
            wbits: 0,
            wsize: 0,
            whave: 0,
            wnext: 0,
            window: Vec::new(),
            hold: 0,
            bits: 0,
            length: 0,
            offset: 0,
            extra: 0,
            lencode: TableRef::Codes(0),
            distcode: TableRef::Codes(0),
            lenbits: 0,
            distbits: 0,
            ncode: 0,
            nlen: 0,
            ndist: 0,
            have: 0,
            next: 0,
            lens: [0; 320],
            work: [0; 288],
            codes: Box::new([Code::default(); ENOUGH]),
            sane: false,
            back: 0,
            was: 0,
        }
    }

    /// The length/literal and distance decoding tables.
    #[inline(always)]
    pub(crate) fn tables(&self) -> (&[Code], &[Code]) {
        let l: &[Code] = match self.lencode {
            TableRef::Fixed => &LENFIX,
            TableRef::Codes(o) => &self.codes[o..],
        };
        let d: &[Code] = match self.distcode {
            TableRef::Fixed => &DISTFIX,
            TableRef::Codes(o) => &self.codes[o..],
        };
        (l, d)
    }

    /// `inflateInit2_()`
    pub(crate) fn init2(window_bits: i32) -> Result<Box<InflateState>, i32> {
        let mut st = Box::new(InflateState::new_zeroed());
        st.mode = Mode::Head; // to pass state test in inflateReset2()
        let ret = st.reset2(window_bits);
        if ret != Z_OK {
            return Err(ret);
        }
        Ok(st)
    }

    /// `inflateResetKeep()`
    pub(crate) fn reset_keep(&mut self) -> i32 {
        self.total_in = 0;
        self.total_out = 0;
        self.total = 0;
        self.msg = None;
        self.data_type = 0;
        if self.wrap != 0 {
            // to support ill-conceived Java test suite
            self.adler = (self.wrap & 1) as u32;
        }
        self.mode = Mode::Head;
        self.last = false;
        self.havedict = false;
        self.flags = -1;
        self.dmax = 32768;
        self.head = None;
        self.hold = 0;
        self.bits = 0;
        self.lencode = TableRef::Codes(0);
        self.distcode = TableRef::Codes(0);
        self.next = 0;
        self.sane = true;
        self.back = -1;
        Z_OK
    }

    /// `inflateReset()`
    pub(crate) fn reset(&mut self) -> i32 {
        self.wsize = 0;
        self.whave = 0;
        self.wnext = 0;
        self.reset_keep()
    }

    /// `inflateReset2()`
    pub(crate) fn reset2(&mut self, window_bits: i32) -> i32 {
        let wrap;
        let mut window_bits = window_bits;
        // extract wrap request from windowBits parameter
        if window_bits < 0 {
            if window_bits < -15 {
                return Z_STREAM_ERROR;
            }
            wrap = 0;
            window_bits = -window_bits;
        } else {
            wrap = (window_bits >> 4) + 5;
            if window_bits < 48 {
                window_bits &= 15;
            }
        }

        // set number of window bits, free window if different
        if window_bits != 0 && !(8..=15).contains(&window_bits) {
            return Z_STREAM_ERROR;
        }
        if !self.window.is_empty() && self.wbits != window_bits as u32 {
            self.window = Vec::new();
        }

        // update state and reset the rest of it
        self.wrap = wrap;
        self.wbits = window_bits as u32;
        self.reset()
    }

    /// `inflatePrime()`
    pub(crate) fn prime(&mut self, bits: i32, value: i32) -> i32 {
        if bits == 0 {
            return Z_OK;
        }
        if bits < 0 {
            self.hold = 0;
            self.bits = 0;
            return Z_OK;
        }
        if bits > 16 || self.bits + bits as u32 > 32 {
            return Z_STREAM_ERROR;
        }
        let value = (value as i64) & ((1i64 << bits) - 1);
        self.hold = self.hold.wrapping_add((value as u64) << self.bits) & 0xffff_ffff;
        self.bits += bits as u32;
        Z_OK
    }

    /// `updatewindow()`: copy the last `src.len()` bytes written into the
    /// circular window, creating it if needed.
    fn update_window(&mut self, src: &[u8]) {
        let copy = src.len() as u32;
        let end = src.len();
        // if it hasn't been done already, allocate space for the window
        if self.window.is_empty() {
            self.window = vec![0u8; 1usize << self.wbits];
        }

        // if window not in use yet, initialize
        if self.wsize == 0 {
            self.wsize = 1u32 << self.wbits;
            self.wnext = 0;
            self.whave = 0;
        }

        // copy state->wsize or less output bytes into the circular window
        if copy >= self.wsize {
            let ws = self.wsize as usize;
            self.window[..ws].copy_from_slice(&src[end - ws..end]);
            self.wnext = 0;
            self.whave = self.wsize;
        } else {
            let mut dist = self.wsize - self.wnext;
            if dist > copy {
                dist = copy;
            }
            let wn = self.wnext as usize;
            let c = copy as usize;
            self.window[wn..wn + dist as usize].copy_from_slice(&src[end - c..end - c + dist as usize]);
            let copy = copy - dist;
            if copy != 0 {
                let c2 = copy as usize;
                self.window[..c2].copy_from_slice(&src[end - c2..end]);
                self.wnext = copy;
                self.whave = self.wsize;
            } else {
                self.wnext += dist;
                if self.wnext == self.wsize {
                    self.wnext = 0;
                }
                if self.whave < self.wsize {
                    self.whave += dist;
                }
            }
        }
    }

    /// `UPDATE_CHECK(check, buf, len)`
    #[inline]
    fn update_check(&self, check: u32, buf: &[u8]) -> u32 {
        if self.flags != 0 {
            crc32(check, buf)
        } else {
            adler32(check, buf)
        }
    }

    /// `inflate()`
    pub(crate) fn inflate(&mut self, io: &mut Io, flush: i32) -> i32 {
        let mut here: Code;
        let mut last: Code;
        let mut len: u32;
        let mut copy: u32;
        let mut ret: i32;
        let mut hbuf = [0u8; 4];

        if self.mode == Mode::Type {
            self.mode = Mode::TypeDo; // skip check
        }
        // LOAD()
        let mut put = io.next_out;
        let mut left = io.avail_out;
        let mut next = io.next_in;
        let mut have = io.avail_in;
        let mut hold = self.hold;
        let mut bits = self.bits;
        let mut in_ = have;
        let mut out = left;
        ret = Z_OK;

        macro_rules! pullbyte {
            ($l:lifetime) => {{
                if have == 0 {
                    break $l;
                }
                have -= 1;
                hold += (io.input[next] as u64) << bits;
                next += 1;
                bits += 8;
            }};
        }
        macro_rules! needbits {
            ($l:lifetime, $n:expr) => {{
                let n = $n as u32;
                while bits < n {
                    pullbyte!($l);
                }
            }};
        }
        macro_rules! nbits {
            ($n:expr) => {
                (hold as u32 & ((1u32 << ($n)) - 1))
            };
        }
        macro_rules! dropbits {
            ($n:expr) => {{
                let n = $n as u32;
                hold >>= n;
                bits -= n;
            }};
        }
        macro_rules! initbits {
            () => {{
                hold = 0;
                bits = 0;
            }};
        }
        macro_rules! bytebits {
            () => {{
                hold >>= bits & 7;
                bits -= bits & 7;
            }};
        }
        macro_rules! crc2 {
            ($check:expr, $word:expr) => {{
                let w = $word;
                hbuf[0] = w as u8;
                hbuf[1] = (w >> 8) as u8;
                $check = crc32($check, &hbuf[..2]);
            }};
        }
        macro_rules! crc4 {
            ($check:expr, $word:expr) => {{
                let w = $word;
                hbuf[0] = w as u8;
                hbuf[1] = (w >> 8) as u8;
                hbuf[2] = (w >> 16) as u8;
                hbuf[3] = (w >> 24) as u8;
                $check = crc32($check, &hbuf[..4]);
            }};
        }

        'leave: loop {
            match self.mode {
                Mode::Head => {
                    if self.wrap == 0 {
                        self.mode = Mode::TypeDo;
                        continue;
                    }
                    needbits!('leave, 16);
                    if (self.wrap & 2) != 0 && hold == 0x8b1f {
                        // gzip header
                        if self.wbits == 0 {
                            self.wbits = 15;
                        }
                        self.check = 0;
                        crc2!(self.check, hold);
                        initbits!();
                        self.mode = Mode::Flags;
                        continue;
                    }
                    if let Some(h) = self.head.as_mut() {
                        h.done = -1;
                    }
                    if (self.wrap & 1) == 0 // check if zlib header allowed
                        || ((nbits!(8) as u64) << 8).wrapping_add(hold >> 8) % 31 != 0
                    {
                        self.msg = Some("incorrect header check");
                        self.mode = Mode::Bad;
                        continue;
                    }
                    if nbits!(4) != Z_DEFLATED as u32 {
                        self.msg = Some("unknown compression method");
                        self.mode = Mode::Bad;
                        continue;
                    }
                    dropbits!(4);
                    len = nbits!(4) + 8;
                    if self.wbits == 0 {
                        self.wbits = len;
                    }
                    if len > 15 || len > self.wbits {
                        self.msg = Some("invalid window size");
                        self.mode = Mode::Bad;
                        continue;
                    }
                    self.dmax = 1u32 << len;
                    self.flags = 0; // indicate zlib header
                    self.check = 1;
                    self.adler = 1;
                    self.mode = if hold & 0x200 != 0 {
                        Mode::DictId
                    } else {
                        Mode::Type
                    };
                    initbits!();
                }
                Mode::Flags => {
                    needbits!('leave, 16);
                    self.flags = hold as i32;
                    if (self.flags & 0xff) != Z_DEFLATED {
                        self.msg = Some("unknown compression method");
                        self.mode = Mode::Bad;
                        continue;
                    }
                    if self.flags & 0xe000 != 0 {
                        self.msg = Some("unknown header flags set");
                        self.mode = Mode::Bad;
                        continue;
                    }
                    if let Some(h) = self.head.as_mut() {
                        h.text = ((hold >> 8) & 1) as i32;
                    }
                    if (self.flags & 0x0200) != 0 && (self.wrap & 4) != 0 {
                        crc2!(self.check, hold);
                    }
                    initbits!();
                    self.mode = Mode::Time;
                }
                Mode::Time => {
                    needbits!('leave, 32);
                    if let Some(h) = self.head.as_mut() {
                        h.time = hold as u32;
                    }
                    if (self.flags & 0x0200) != 0 && (self.wrap & 4) != 0 {
                        crc4!(self.check, hold);
                    }
                    initbits!();
                    self.mode = Mode::Os;
                }
                Mode::Os => {
                    needbits!('leave, 16);
                    if let Some(h) = self.head.as_mut() {
                        h.xflags = (hold & 0xff) as i32;
                        h.os = (hold >> 8) as i32;
                    }
                    if (self.flags & 0x0200) != 0 && (self.wrap & 4) != 0 {
                        crc2!(self.check, hold);
                    }
                    initbits!();
                    self.mode = Mode::ExLen;
                }
                Mode::ExLen => {
                    if self.flags & 0x0400 != 0 {
                        needbits!('leave, 16);
                        self.length = hold as u32;
                        if let Some(h) = self.head.as_mut() {
                            h.extra_len = hold as u32;
                        }
                        if (self.flags & 0x0200) != 0 && (self.wrap & 4) != 0 {
                            crc2!(self.check, hold);
                        }
                        initbits!();
                    } else if let Some(h) = self.head.as_mut() {
                        h.extra = None;
                    }
                    self.mode = Mode::Extra;
                }
                Mode::Extra => {
                    if self.flags & 0x0400 != 0 {
                        copy = self.length;
                        if copy > have {
                            copy = have;
                        }
                        if copy != 0 {
                            let length = self.length;
                            if let Some(h) = self.head.as_mut() {
                                let extra_len = h.extra_len;
                                let extra_max = h.extra_max;
                                if let Some(extra) = h.extra.as_mut() {
                                    let l = extra_len.wrapping_sub(length);
                                    if l < extra_max {
                                        let n = if l + copy > extra_max {
                                            extra_max - l
                                        } else {
                                            copy
                                        } as usize;
                                        let l = l as usize;
                                        let n = n.min(extra.len().saturating_sub(l));
                                        extra[l..l + n].copy_from_slice(&io.input[next..next + n]);
                                    }
                                }
                            }
                            if (self.flags & 0x0200) != 0 && (self.wrap & 4) != 0 {
                                self.check =
                                    crc32(self.check, &io.input[next..next + copy as usize]);
                            }
                            have -= copy;
                            next += copy as usize;
                            self.length -= copy;
                        }
                        if self.length != 0 {
                            break 'leave;
                        }
                    }
                    self.length = 0;
                    self.mode = Mode::Name;
                }
                Mode::Name => {
                    if self.flags & 0x0800 != 0 {
                        if have == 0 {
                            break 'leave;
                        }
                        copy = 0;
                        loop {
                            len = io.input[next + copy as usize] as u32;
                            copy += 1;
                            if let Some(h) = self.head.as_mut() {
                                if let Some(name) = h.name.as_mut() {
                                    if self.length < h.name_max && (self.length as usize) < name.len()
                                    {
                                        name[self.length as usize] = len as u8;
                                        self.length += 1;
                                    }
                                }
                            }
                            if !(len != 0 && copy < have) {
                                break;
                            }
                        }
                        if (self.flags & 0x0200) != 0 && (self.wrap & 4) != 0 {
                            self.check = crc32(self.check, &io.input[next..next + copy as usize]);
                        }
                        have -= copy;
                        next += copy as usize;
                        if len != 0 {
                            break 'leave;
                        }
                    } else if let Some(h) = self.head.as_mut() {
                        h.name = None;
                    }
                    self.length = 0;
                    self.mode = Mode::Comment;
                }
                Mode::Comment => {
                    if self.flags & 0x1000 != 0 {
                        if have == 0 {
                            break 'leave;
                        }
                        copy = 0;
                        loop {
                            len = io.input[next + copy as usize] as u32;
                            copy += 1;
                            if let Some(h) = self.head.as_mut() {
                                if let Some(c) = h.comment.as_mut() {
                                    if self.length < h.comm_max && (self.length as usize) < c.len() {
                                        c[self.length as usize] = len as u8;
                                        self.length += 1;
                                    }
                                }
                            }
                            if !(len != 0 && copy < have) {
                                break;
                            }
                        }
                        if (self.flags & 0x0200) != 0 && (self.wrap & 4) != 0 {
                            self.check = crc32(self.check, &io.input[next..next + copy as usize]);
                        }
                        have -= copy;
                        next += copy as usize;
                        if len != 0 {
                            break 'leave;
                        }
                    } else if let Some(h) = self.head.as_mut() {
                        h.comment = None;
                    }
                    self.mode = Mode::Hcrc;
                }
                Mode::Hcrc => {
                    if self.flags & 0x0200 != 0 {
                        needbits!('leave, 16);
                        if (self.wrap & 4) != 0 && hold != (self.check & 0xffff) as u64 {
                            self.msg = Some("header crc mismatch");
                            self.mode = Mode::Bad;
                            continue;
                        }
                        initbits!();
                    }
                    if let Some(h) = self.head.as_mut() {
                        h.hcrc = (self.flags >> 9) & 1;
                        h.done = 1;
                    }
                    self.check = 0;
                    self.adler = 0;
                    self.mode = Mode::Type;
                }
                Mode::DictId => {
                    needbits!('leave, 32);
                    self.check = zswap32(hold);
                    self.adler = self.check;
                    initbits!();
                    self.mode = Mode::Dict;
                }
                Mode::Dict => {
                    if !self.havedict {
                        // RESTORE()
                        io.next_out = put;
                        io.avail_out = left;
                        io.next_in = next;
                        io.avail_in = have;
                        self.hold = hold;
                        self.bits = bits;
                        return Z_NEED_DICT;
                    }
                    self.check = 1;
                    self.adler = 1;
                    self.mode = Mode::Type;
                }
                Mode::Type | Mode::TypeDo => {
                    if self.mode == Mode::Type && (flush == Z_BLOCK || flush == Z_TREES) {
                        break 'leave;
                    }
                    // TYPEDO:
                    if self.last {
                        bytebits!();
                        self.mode = Mode::Check;
                        continue;
                    }
                    needbits!('leave, 3);
                    self.last = nbits!(1) != 0;
                    dropbits!(1);
                    match nbits!(2) {
                        0 => {
                            // stored block
                            self.mode = Mode::Stored;
                        }
                        1 => {
                            // fixed block
                            self.lencode = TableRef::Fixed;
                            self.lenbits = 9;
                            self.distcode = TableRef::Fixed;
                            self.distbits = 5;
                            self.mode = Mode::Len_; // decode codes
                            if flush == Z_TREES {
                                dropbits!(2);
                                break 'leave;
                            }
                        }
                        2 => {
                            // dynamic block
                            self.mode = Mode::Table;
                        }
                        _ => {
                            self.msg = Some("invalid block type");
                            self.mode = Mode::Bad;
                        }
                    }
                    dropbits!(2);
                }
                Mode::Stored => {
                    bytebits!(); // go to byte boundary
                    needbits!('leave, 32);
                    if (hold & 0xffff) != ((hold >> 16) ^ 0xffff) {
                        self.msg = Some("invalid stored block lengths");
                        self.mode = Mode::Bad;
                        continue;
                    }
                    self.length = (hold as u32) & 0xffff;
                    initbits!();
                    self.mode = Mode::Copy_;
                    if flush == Z_TREES {
                        break 'leave;
                    }
                }
                Mode::Copy_ => {
                    self.mode = Mode::Copy;
                }
                Mode::Copy => {
                    copy = self.length;
                    if copy != 0 {
                        if copy > have {
                            copy = have;
                        }
                        if copy > left {
                            copy = left;
                        }
                        if copy == 0 {
                            break 'leave;
                        }
                        let c = copy as usize;
                        io.output[put..put + c].copy_from_slice(&io.input[next..next + c]);
                        have -= copy;
                        next += c;
                        left -= copy;
                        put += c;
                        self.length -= copy;
                        continue;
                    }
                    self.mode = Mode::Type;
                }
                Mode::Table => {
                    needbits!('leave, 14);
                    self.nlen = nbits!(5) + 257;
                    dropbits!(5);
                    self.ndist = nbits!(5) + 1;
                    dropbits!(5);
                    self.ncode = nbits!(4) + 4;
                    dropbits!(4);
                    if self.nlen > 286 || self.ndist > 30 {
                        self.msg = Some("too many length or distance symbols");
                        self.mode = Mode::Bad;
                        continue;
                    }
                    self.have = 0;
                    self.mode = Mode::LenLens;
                }
                Mode::LenLens => {
                    while self.have < self.ncode {
                        needbits!('leave, 3);
                        self.lens[ORDER[self.have as usize] as usize] = nbits!(3) as u16;
                        self.have += 1;
                        dropbits!(3);
                    }
                    while self.have < 19 {
                        self.lens[ORDER[self.have as usize] as usize] = 0;
                        self.have += 1;
                    }
                    self.next = 0;
                    self.lencode = TableRef::Codes(0);
                    self.distcode = TableRef::Codes(0);
                    self.lenbits = 7;
                    let r = inflate_table(
                        CodeType::Codes,
                        &self.lens,
                        19,
                        &mut self.codes[..],
                        &mut self.next,
                        &mut self.lenbits,
                        &mut self.work,
                    );
                    if r != 0 {
                        self.msg = Some("invalid code lengths set");
                        self.mode = Mode::Bad;
                        continue;
                    }
                    self.have = 0;
                    self.mode = Mode::CodeLens;
                }
                Mode::CodeLens => {
                    while self.have < self.nlen + self.ndist {
                        let lc = match self.lencode {
                            TableRef::Fixed => 0,
                            TableRef::Codes(o) => o,
                        };
                        loop {
                            here = self.codes[lc + nbits!(self.lenbits) as usize];
                            if here.bits as u32 <= bits {
                                break;
                            }
                            pullbyte!('leave);
                        }
                        if here.val < 16 {
                            dropbits!(here.bits);
                            self.lens[self.have as usize] = here.val;
                            self.have += 1;
                        } else {
                            if here.val == 16 {
                                needbits!('leave, here.bits as u32 + 2);
                                dropbits!(here.bits);
                                if self.have == 0 {
                                    self.msg = Some("invalid bit length repeat");
                                    self.mode = Mode::Bad;
                                    break;
                                }
                                len = self.lens[self.have as usize - 1] as u32;
                                copy = 3 + nbits!(2);
                                dropbits!(2);
                            } else if here.val == 17 {
                                needbits!('leave, here.bits as u32 + 3);
                                dropbits!(here.bits);
                                len = 0;
                                copy = 3 + nbits!(3);
                                dropbits!(3);
                            } else {
                                needbits!('leave, here.bits as u32 + 7);
                                dropbits!(here.bits);
                                len = 0;
                                copy = 11 + nbits!(7);
                                dropbits!(7);
                            }
                            if self.have + copy > self.nlen + self.ndist {
                                self.msg = Some("invalid bit length repeat");
                                self.mode = Mode::Bad;
                                break;
                            }
                            while copy > 0 {
                                copy -= 1;
                                self.lens[self.have as usize] = len as u16;
                                self.have += 1;
                            }
                        }
                    }

                    // handle error breaks in while
                    if self.mode == Mode::Bad {
                        continue;
                    }

                    // check for end-of-block code (better have one)
                    if self.lens[256] == 0 {
                        self.msg = Some("invalid code -- missing end-of-block");
                        self.mode = Mode::Bad;
                        continue;
                    }

                    // build code tables -- note: do not change the lenbits or
                    // distbits values here (9 and 6) without reading the
                    // comments in inftrees.h concerning the ENOUGH constants,
                    // which depend on those values
                    self.next = 0;
                    self.lencode = TableRef::Codes(0);
                    self.lenbits = 9;
                    let nlen = self.nlen as usize;
                    let r = inflate_table(
                        CodeType::Lens,
                        &self.lens,
                        nlen,
                        &mut self.codes[..],
                        &mut self.next,
                        &mut self.lenbits,
                        &mut self.work,
                    );
                    if r != 0 {
                        self.msg = Some("invalid literal/lengths set");
                        self.mode = Mode::Bad;
                        continue;
                    }
                    self.distcode = TableRef::Codes(self.next);
                    self.distbits = 6;
                    let ndist = self.ndist as usize;
                    let r = inflate_table(
                        CodeType::Dists,
                        &self.lens[nlen..],
                        ndist,
                        &mut self.codes[..],
                        &mut self.next,
                        &mut self.distbits,
                        &mut self.work,
                    );
                    if r != 0 {
                        self.msg = Some("invalid distances set");
                        self.mode = Mode::Bad;
                        continue;
                    }
                    self.mode = Mode::Len_;
                    if flush == Z_TREES {
                        break 'leave;
                    }
                }
                Mode::Len_ => {
                    self.mode = Mode::Len;
                }
                Mode::Len => {
                    if have >= 6 && left >= 258 {
                        // RESTORE()
                        io.next_out = put;
                        io.avail_out = left;
                        io.next_in = next;
                        io.avail_in = have;
                        self.hold = hold;
                        self.bits = bits;
                        inflate_fast(self, io, out);
                        // LOAD()
                        put = io.next_out;
                        left = io.avail_out;
                        next = io.next_in;
                        have = io.avail_in;
                        hold = self.hold;
                        bits = self.bits;
                        if self.mode == Mode::Type {
                            self.back = -1;
                        }
                        continue;
                    }
                    self.back = 0;
                    {
                        let (lcode, _) = self.tables();
                        loop {
                            here = lcode[nbits!(self.lenbits) as usize];
                            if here.bits as u32 <= bits {
                                break;
                            }
                            pullbyte!('leave);
                        }
                        if here.op != 0 && (here.op & 0xf0) == 0 {
                            last = here;
                            loop {
                                here = lcode[last.val as usize
                                    + (nbits!(last.bits as u32 + last.op as u32) >> last.bits)
                                        as usize];
                                if (last.bits as u32 + here.bits as u32) <= bits {
                                    break;
                                }
                                pullbyte!('leave);
                            }
                            dropbits!(last.bits);
                            self.back += last.bits as i32;
                        }
                    }
                    dropbits!(here.bits);
                    self.back += here.bits as i32;
                    self.length = here.val as u32;
                    if here.op == 0 {
                        self.mode = Mode::Lit;
                        continue;
                    }
                    if here.op & 32 != 0 {
                        self.back = -1;
                        self.mode = Mode::Type;
                        continue;
                    }
                    if here.op & 64 != 0 {
                        self.msg = Some("invalid literal/length code");
                        self.mode = Mode::Bad;
                        continue;
                    }
                    self.extra = (here.op as u32) & 15;
                    self.mode = Mode::LenExt;
                }
                Mode::LenExt => {
                    if self.extra != 0 {
                        needbits!('leave, self.extra);
                        self.length += nbits!(self.extra);
                        dropbits!(self.extra);
                        self.back += self.extra as i32;
                    }
                    self.was = self.length;
                    self.mode = Mode::Dist;
                }
                Mode::Dist => {
                    {
                        let (_, dcode) = self.tables();
                        loop {
                            here = dcode[nbits!(self.distbits) as usize];
                            if here.bits as u32 <= bits {
                                break;
                            }
                            pullbyte!('leave);
                        }
                        if (here.op & 0xf0) == 0 {
                            last = here;
                            loop {
                                here = dcode[last.val as usize
                                    + (nbits!(last.bits as u32 + last.op as u32) >> last.bits)
                                        as usize];
                                if (last.bits as u32 + here.bits as u32) <= bits {
                                    break;
                                }
                                pullbyte!('leave);
                            }
                            dropbits!(last.bits);
                            self.back += last.bits as i32;
                        }
                    }
                    dropbits!(here.bits);
                    self.back += here.bits as i32;
                    if here.op & 64 != 0 {
                        self.msg = Some("invalid distance code");
                        self.mode = Mode::Bad;
                        continue;
                    }
                    self.offset = here.val as u32;
                    self.extra = (here.op as u32) & 15;
                    self.mode = Mode::DistExt;
                }
                Mode::DistExt => {
                    if self.extra != 0 {
                        needbits!('leave, self.extra);
                        self.offset += nbits!(self.extra);
                        dropbits!(self.extra);
                        self.back += self.extra as i32;
                    }
                    self.mode = Mode::Match;
                }
                Mode::Match => {
                    if left == 0 {
                        break 'leave;
                    }
                    copy = out - left;
                    if self.offset > copy {
                        // copy from window
                        copy = self.offset - copy;
                        if copy > self.whave && self.sane {
                            self.msg = Some("invalid distance too far back");
                            self.mode = Mode::Bad;
                            continue;
                        }
                        let from = if copy > self.wnext {
                            copy -= self.wnext;
                            (self.wsize - copy) as usize
                        } else {
                            (self.wnext - copy) as usize
                        };
                        if copy > self.length {
                            copy = self.length;
                        }
                        if copy > left {
                            copy = left;
                        }
                        left -= copy;
                        self.length -= copy;
                        let c = copy as usize;
                        io.output[put..put + c].copy_from_slice(&self.window[from..from + c]);
                        put += c;
                    } else {
                        // copy from output
                        let dist = self.offset as usize;
                        copy = self.length;
                        if copy > left {
                            copy = left;
                        }
                        left -= copy;
                        self.length -= copy;
                        crate::inffast::copy_match(io.output, put, dist, copy as usize);
                        put += copy as usize;
                    }
                    if self.length == 0 {
                        self.mode = Mode::Len;
                    }
                }
                Mode::Lit => {
                    if left == 0 {
                        break 'leave;
                    }
                    io.output[put] = self.length as u8;
                    put += 1;
                    left -= 1;
                    self.mode = Mode::Len;
                }
                Mode::Check => {
                    if self.wrap != 0 {
                        needbits!('leave, 32);
                        out -= left;
                        self.total_out += out as u64;
                        self.total += out as u64;
                        if (self.wrap & 4) != 0 && out != 0 {
                            let o = out as usize;
                            self.check = self.update_check(self.check, &io.output[put - o..put]);
                            self.adler = self.check;
                        }
                        out = left;
                        let v = if self.flags != 0 {
                            hold
                        } else {
                            zswap32(hold) as u64
                        };
                        if (self.wrap & 4) != 0 && v != self.check as u64 {
                            self.msg = Some("incorrect data check");
                            self.mode = Mode::Bad;
                            continue;
                        }
                        initbits!();
                    }
                    self.mode = Mode::Length;
                }
                Mode::Length => {
                    if self.wrap != 0 && self.flags != 0 {
                        needbits!('leave, 32);
                        if (self.wrap & 4) != 0 && hold != (self.total & 0xffff_ffff) {
                            self.msg = Some("incorrect length check");
                            self.mode = Mode::Bad;
                            continue;
                        }
                        initbits!();
                    }
                    self.mode = Mode::Done;
                }
                Mode::Done => {
                    ret = Z_STREAM_END;
                    break 'leave;
                }
                Mode::Bad => {
                    ret = Z_DATA_ERROR;
                    break 'leave;
                }
                Mode::Mem => return Z_MEM_ERROR,
                Mode::Sync => return Z_STREAM_ERROR,
            }
        }

        // inf_leave:
        // Return from inflate(), updating the total counts and the check
        // value. If there was no progress during the inflate() call, return a
        // buffer error. Call updatewindow() to create and/or update the window
        // state.
        // RESTORE()
        io.next_out = put;
        io.avail_out = left;
        io.next_in = next;
        io.avail_in = have;
        self.hold = hold;
        self.bits = bits;
        if self.wsize != 0
            || (out != io.avail_out
                && self.mode < Mode::Bad
                && (self.mode < Mode::Check || flush != Z_FINISH))
        {
            let n = out.wrapping_sub(io.avail_out) as usize;
            let end = io.next_out;
            self.update_window(&io.output[end - n..end]);
        }
        in_ -= io.avail_in;
        out -= io.avail_out;
        self.total_in += in_ as u64;
        self.total_out += out as u64;
        self.total += out as u64;
        if (self.wrap & 4) != 0 && out != 0 {
            let o = out as usize;
            self.check = self.update_check(self.check, &io.output[io.next_out - o..io.next_out]);
            self.adler = self.check;
        }
        self.data_type = self.bits as i32
            + if self.last { 64 } else { 0 }
            + if self.mode == Mode::Type { 128 } else { 0 }
            + if self.mode == Mode::Len_ || self.mode == Mode::Copy_ {
                256
            } else {
                0
            };
        if ((in_ == 0 && out == 0) || flush == Z_FINISH) && ret == Z_OK {
            ret = Z_BUF_ERROR;
        }
        ret
    }

    /// `inflateGetDictionary()`
    pub(crate) fn get_dictionary(&self) -> Vec<u8> {
        let mut d = Vec::with_capacity(self.whave as usize);
        if self.whave != 0 {
            let wn = self.wnext as usize;
            let wh = self.whave as usize;
            d.extend_from_slice(&self.window[wn..wh]);
            d.extend_from_slice(&self.window[..wn]);
        }
        d
    }

    /// `inflateSetDictionary()`
    pub(crate) fn set_dictionary(&mut self, dictionary: &[u8]) -> i32 {
        if self.wrap != 0 && self.mode != Mode::Dict {
            return Z_STREAM_ERROR;
        }
        // check for correct dictionary identifier
        if self.mode == Mode::Dict {
            let dictid = adler32(1, dictionary);
            if dictid != self.check {
                return Z_DATA_ERROR;
            }
        }
        // copy dictionary to window using updatewindow(), which will amend the
        // existing dictionary if appropriate
        self.update_window(dictionary);
        self.havedict = true;
        Z_OK
    }

    /// `inflateSync()`
    pub(crate) fn sync(&mut self, io: &mut Io) -> i32 {
        if io.avail_in == 0 && self.bits < 8 {
            return Z_BUF_ERROR;
        }
        // if first time, start search in bit buffer
        if self.mode != Mode::Sync {
            self.mode = Mode::Sync;
            self.hold >>= self.bits & 7;
            self.bits -= self.bits & 7;
            let mut buf = [0u8; 4];
            let mut len = 0usize;
            while self.bits >= 8 {
                buf[len] = self.hold as u8;
                len += 1;
                self.hold >>= 8;
                self.bits -= 8;
            }
            self.have = 0;
            let mut h = self.have;
            syncsearch(&mut h, &buf[..len]);
            self.have = h;
        }

        // search available input
        let mut h = self.have;
        let len = syncsearch(&mut h, &io.input[io.next_in..io.next_in + io.avail_in as usize]);
        self.have = h;
        io.avail_in -= len as u32;
        io.next_in += len;
        self.total_in += len as u64;

        // return no joy or set up to restart inflate() on a new block
        if self.have != 4 {
            return Z_DATA_ERROR;
        }
        if self.flags == -1 {
            self.wrap = 0; // if no header yet, treat as raw
        } else {
            self.wrap &= !4; // no point in computing a check value now
        }
        let flags = self.flags;
        let (in_, out) = (self.total_in, self.total_out);
        self.reset();
        self.total_in = in_;
        self.total_out = out;
        self.flags = flags;
        self.mode = Mode::Type;
        Z_OK
    }

    /// `inflateSyncPoint()`
    pub(crate) fn sync_point(&self) -> i32 {
        (self.mode == Mode::Stored && self.bits == 0) as i32
    }

    /// `inflateMark()`
    pub(crate) fn mark(&self) -> i64 {
        let back = ((self.back as i64) as u64).wrapping_shl(16) as i64;
        back + match self.mode {
            Mode::Copy => self.length as i64,
            Mode::Match => (self.was - self.length) as i64,
            _ => 0,
        }
    }

    /// `inflateValidate()`
    pub(crate) fn validate(&mut self, check: bool) {
        if check && self.wrap != 0 {
            self.wrap |= 4;
        } else {
            self.wrap &= !4;
        }
    }
}

/// Search buf for the pattern: 0, 0, 0xff, 0xff.
fn syncsearch(have: &mut u32, buf: &[u8]) -> usize {
    let mut got = *have;
    let mut next = 0usize;
    while next < buf.len() && got < 4 {
        if buf[next] as u32 == if got < 2 { 0 } else { 0xff } {
            got += 1;
        } else if buf[next] != 0 {
            got = 0;
        } else {
            got = 4 - got;
        }
        next += 1;
    }
    *have = got;
    next
}
