//! Ports of the xpdf decoding filters in `Stream.cc` that the PDF reader
//! needs: `ASCIIHexStream`, `ASCII85Stream`, `LZWStream`,
//! `RunLengthStream`, `FlateStream` and `StreamPredictor`.
//!
//! xpdf decodes lazily (pull-based `getChar()` chains).  Every decoder
//! here consumes its whole input and returns the complete output, which
//! yields exactly the same bytes: each xpdf filter reads its source
//! sequentially until EOF or its own end marker, and none of them
//! depends on how far downstream consumers read.
//!
//! Note that FlateDecode is xpdf's *own* inflate implementation
//! (`FlateStream`), not zlib.  It differs from zlib on damaged input
//! (no Adler-32 check, partial codes accepted at EOF, lengths 286/287
//! accepted, buffered bits discarded before stored blocks, decompression
//! bomb cut-off, ...), so it is ported literally instead of calling a
//! generic inflate.

use crate::lexer::{is_space, EOF};

/// `decompressionBombSizeThreshold`
const BOMB_SIZE_THRESHOLD: u64 = 50_000_000;
/// `decompressionBombRatioThreshold`
const BOMB_RATIO_THRESHOLD: u64 = 200;

/// Simple input cursor with `getChar()` returning EOF (-1) at the end.
pub(crate) struct Input<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Input<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Input { data, pos: 0 }
    }
    #[inline]
    pub(crate) fn get_char(&mut self) -> i32 {
        if self.pos < self.data.len() {
            let c = self.data[self.pos];
            self.pos += 1;
            c as i32
        } else {
            EOF
        }
    }
}

/// C `isspace()` in the "C" locale.
#[inline]
fn c_isspace(c: i32) -> bool {
    c == 0x20 || (0x09..=0x0d).contains(&c)
}

// ------------------------------------------------------------------------
// ASCIIHexStream
// ------------------------------------------------------------------------

/// `ASCIIHexStream::lookChar()` loop.
pub fn ascii_hex_decode(input: &[u8]) -> Vec<u8> {
    let mut str = Input::new(input);
    let mut out = Vec::with_capacity(input.len() / 2);
    let mut eof = false;
    loop {
        if eof {
            break;
        }
        let mut c1;
        loop {
            c1 = str.get_char();
            if !c_isspace(c1) {
                break;
            }
        }
        if c1 == b'>' as i32 {
            break;
        }
        let mut c2;
        loop {
            c2 = str.get_char();
            if !c_isspace(c2) {
                break;
            }
        }
        if c2 == b'>' as i32 {
            eof = true;
            c2 = b'0' as i32;
        }
        let mut x: i32;
        if (b'0' as i32..=b'9' as i32).contains(&c1) {
            x = (c1 - b'0' as i32) << 4;
        } else if (b'A' as i32..=b'F' as i32).contains(&c1) {
            x = (c1 - b'A' as i32 + 10) << 4;
        } else if (b'a' as i32..=b'f' as i32).contains(&c1) {
            x = (c1 - b'a' as i32 + 10) << 4;
        } else if c1 == EOF {
            eof = true;
            x = 0;
        } else {
            x = 0;
        }
        if (b'0' as i32..=b'9' as i32).contains(&c2) {
            x += c2 - b'0' as i32;
        } else if (b'A' as i32..=b'F' as i32).contains(&c2) {
            x += c2 - b'A' as i32 + 10;
        } else if (b'a' as i32..=b'f' as i32).contains(&c2) {
            x += c2 - b'a' as i32 + 10;
        } else if c2 == EOF {
            eof = true;
            x = 0;
        }
        // lookChar() returns buf = x & 0xff even when it just hit EOF
        // (c1 == EOF -> x = 0, and that 0 byte *is* returned once)
        out.push((x & 0xff) as u8);
    }
    out
}

// ------------------------------------------------------------------------
// ASCII85Stream
// ------------------------------------------------------------------------

/// `ASCII85Stream` (state machine ported exactly, including its handling
/// of a final partial group: the terminating '~' / EOF value is used for
/// the digit position where it occurred, and only the following
/// positions are padded with 'u').
pub fn ascii85_decode(input: &[u8]) -> Vec<u8> {
    let mut str = Input::new(input);
    let mut out = Vec::with_capacity(input.len());
    let mut c = [0i32; 5];
    let mut b = [0i32; 4];
    let mut index: i32 = 0;
    let mut n: i32 = 0;
    let mut eof = false;
    loop {
        // lookChar()
        if index >= n {
            if eof {
                break;
            }
            index = 0;
            loop {
                c[0] = str.get_char();
                if !is_space(c[0]) {
                    break;
                }
            }
            if c[0] == b'~' as i32 || c[0] == EOF {
                // eof = true; n = 0; return EOF
                break;
            } else if c[0] == b'z' as i32 {
                b = [0; 4];
                n = 4;
            } else {
                let mut k = 1usize;
                while k < 5 {
                    loop {
                        c[k] = str.get_char();
                        if !is_space(c[k]) {
                            break;
                        }
                    }
                    if c[k] == b'~' as i32 || c[k] == EOF {
                        break;
                    }
                    k += 1;
                }
                n = k as i32 - 1;
                if k < 5 && (c[k] == b'~' as i32 || c[k] == EOF) {
                    k += 1;
                    while k < 5 {
                        c[k] = 0x21 + 84;
                        k += 1;
                    }
                    eof = true;
                }
                let mut t: u32 = 0;
                for &ck in c.iter() {
                    t = t.wrapping_mul(85).wrapping_add((ck - 0x21) as u32);
                }
                for k in (0..4).rev() {
                    b[k] = (t & 0xff) as i32;
                    t >>= 8;
                }
            }
        }
        // getChar(): ch = lookChar(); ++index
        out.push(b[index as usize & 3] as u8);
        index += 1;
    }
    out
}

// ------------------------------------------------------------------------
// RunLengthStream
// ------------------------------------------------------------------------

/// `RunLengthStream::fillBuf()` loop.  A literal run truncated by EOF
/// yields 0xff bytes (`(char)EOF`), as in xpdf.
pub fn run_length_decode(input: &[u8]) -> Vec<u8> {
    let mut str = Input::new(input);
    let mut out = Vec::with_capacity(input.len() * 2);
    loop {
        let c = str.get_char();
        if c == 0x80 || c == EOF {
            break;
        }
        if c < 0x80 {
            let n = c + 1;
            for _ in 0..n {
                out.push(str.get_char() as u8);
            }
        } else {
            let n = 0x101 - c;
            let c = str.get_char();
            for _ in 0..n {
                out.push(c as u8);
            }
        }
    }
    out
}

// ------------------------------------------------------------------------
// LZWStream
// ------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct LzwEntry {
    length: i32,
    head: i32,
    tail: u8,
}

/// `LZWStream` without predictor (the predictor is applied by the caller
/// on the raw output, which is what `getRawChar()` delivers).
pub fn lzw_decode(input: &[u8], early: i32) -> Vec<u8> {
    let mut str = Input::new(input);
    let mut out = Vec::with_capacity(input.len() * 3);
    let mut table = vec![LzwEntry::default(); 4097];
    let mut seq_buf = [0u8; 4097];
    let mut input_buf: u32 = 0;
    let mut input_bits: i32 = 0;
    let mut next_code: i32;
    let mut next_bits: i32;
    let mut seq_length: i32;
    let mut first: bool;
    let mut prev_code: i32 = 0;
    let mut new_char: i32 = 0;
    let mut total_in: u64 = 0;
    let mut total_out: u64 = 0;

    macro_rules! clear_table {
        () => {
            next_code = 258;
            next_bits = 9;
            seq_length = 0;
            first = true;
        };
    }
    clear_table!();

    'outer: loop {
        // processNextCode()
        let mut code;
        loop {
            // getCode()
            let mut got_eof = false;
            while input_bits < next_bits {
                let c = str.get_char();
                if c == EOF {
                    got_eof = true;
                    break;
                }
                input_buf = (input_buf << 8) | (c as u32 & 0xff);
                input_bits += 8;
                total_in += 1;
            }
            if got_eof {
                break 'outer;
            }
            code = ((input_buf >> (input_bits - next_bits)) & ((1u32 << next_bits) - 1)) as i32;
            input_bits -= next_bits;
            if code == 257 {
                break 'outer;
            }
            if code == 256 {
                clear_table!();
                continue;
            }
            break;
        }
        if next_code >= 4097 {
            crate::error::error(
                crate::error::ErrorCategory::SyntaxError,
                -1,
                "Bad LZW stream - expected clear-table code",
            );
            clear_table!();
        }
        let next_length = seq_length + 1;
        if code < 256 {
            seq_buf[0] = code as u8;
            seq_length = 1;
        } else if code < next_code {
            seq_length = table[code as usize].length;
            let mut j = code;
            let mut i = seq_length - 1;
            while i > 0 {
                seq_buf[i as usize] = table[j as usize].tail;
                j = table[j as usize].head;
                i -= 1;
            }
            seq_buf[0] = j as u8;
        } else if code == next_code {
            seq_buf[seq_length as usize] = new_char as u8;
            seq_length += 1;
        } else {
            crate::error::error(crate::error::ErrorCategory::SyntaxError, -1, "Bad LZW stream - unexpected code");
            break 'outer;
        }
        new_char = seq_buf[0] as i32;
        if first {
            first = false;
        } else {
            table[next_code as usize].length = next_length;
            table[next_code as usize].head = prev_code;
            table[next_code as usize].tail = new_char as u8;
            next_code += 1;
            if next_code + early == 512 {
                next_bits = 10;
            } else if next_code + early == 1024 {
                next_bits = 11;
            } else if next_code + early == 2048 {
                next_bits = 12;
            }
        }
        prev_code = code;
        total_out += seq_length as u64;

        // check for a 'decompression bomb'
        if total_out > BOMB_SIZE_THRESHOLD && total_in < total_out / BOMB_RATIO_THRESHOLD {
            crate::error::error(crate::error::ErrorCategory::SyntaxError, -1, "Decompression bomb in LZW stream");
            break 'outer;
        }
        out.extend_from_slice(&seq_buf[..seq_length as usize]);
    }
    out
}

// ------------------------------------------------------------------------
// FlateStream
// ------------------------------------------------------------------------

const FLATE_WINDOW: usize = 32768;
const FLATE_MASK: usize = FLATE_WINDOW - 1;
const FLATE_MAX_CODE_LEN_CODES: usize = 19;
const FLATE_MAX_LIT_CODES: usize = 288;
const FLATE_MAX_DIST_CODES: usize = 30;

/// `codeLenCodeMap`
const CODE_LEN_CODE_MAP: [usize; FLATE_MAX_CODE_LEN_CODES] =
    [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// `lengthDecode` (note codes 286 and 287 decode as length 258)
const LENGTH_DECODE: [(i32, i32); FLATE_MAX_LIT_CODES - 257] = [
    (0, 3), (0, 4), (0, 5), (0, 6), (0, 7), (0, 8), (0, 9), (0, 10), (1, 11), (1, 13), (1, 15),
    (1, 17), (2, 19), (2, 23), (2, 27), (2, 31), (3, 35), (3, 43), (3, 51), (3, 59), (4, 67),
    (4, 83), (4, 99), (4, 115), (5, 131), (5, 163), (5, 195), (5, 227), (0, 258), (0, 258),
    (0, 258),
];

/// `distDecode`
const DIST_DECODE: [(i32, i32); FLATE_MAX_DIST_CODES] = [
    (0, 1), (0, 2), (0, 3), (0, 4), (1, 5), (1, 7), (2, 9), (2, 13), (3, 17), (3, 25), (4, 33),
    (4, 49), (5, 65), (5, 97), (6, 129), (6, 193), (7, 257), (7, 385), (8, 513), (8, 769),
    (9, 1025), (9, 1537), (10, 2049), (10, 3073), (11, 4097), (11, 6145), (12, 8193),
    (12, 12289), (13, 16385), (13, 24577),
];

/// `FlateHuffmanTab`: codes indexed by bit-reversed code, (len, val).
#[derive(Clone, Default)]
pub(crate) struct HuffTab {
    pub(crate) codes: Vec<(u16, u16)>,
    pub(crate) max_len: i32,
}

/// `FlateStream::compHuffmanCodes`.
pub(crate) fn comp_huffman_codes(lengths: &[i32]) -> HuffTab {
    let mut max_len = 0;
    for &l in lengths {
        if l > max_len {
            max_len = l;
        }
    }
    let tab_size = 1usize << max_len;
    let mut codes = vec![(0u16, 0u16); tab_size];
    let mut code: i32 = 0;
    let mut skip: usize = 2;
    let mut len = 1;
    while len <= max_len {
        for (val, &l) in lengths.iter().enumerate() {
            if l == len {
                // bit-reverse the code
                let mut code2: usize = 0;
                let mut t = code;
                for _ in 0..len {
                    code2 = (code2 << 1) | (t & 1) as usize;
                    t >>= 1;
                }
                // fill in the table entries
                let mut i = code2;
                while i < tab_size {
                    codes[i] = (len as u16, val as u16);
                    i += skip;
                }
                code += 1;
            }
        }
        len += 1;
        code <<= 1;
        skip <<= 1;
    }
    HuffTab { codes, max_len }
}

/// `fixedLitCodeTab` / `fixedDistCodeTab` (identical to the static tables
/// in Stream.cc; verified by a unit test against the C source).
pub(crate) fn fixed_tables() -> (HuffTab, HuffTab) {
    let mut lit = [0i32; 288];
    for (i, l) in lit.iter_mut().enumerate() {
        *l = if i < 144 {
            8
        } else if i < 256 {
            9
        } else if i < 280 {
            7
        } else {
            8
        };
    }
    let dist = [5i32; 30];
    (comp_huffman_codes(&lit), comp_huffman_codes(&dist))
}

struct Flate<'a> {
    str: Input<'a>,
    buf: Vec<u8>,
    index: usize,
    remain: usize,
    code_buf: u32,
    code_size: i32,
    code_lengths: [i32; FLATE_MAX_LIT_CODES + FLATE_MAX_DIST_CODES],
    lit_tab: HuffTab,
    dist_tab: HuffTab,
    compressed_block: bool,
    block_len: i32,
    end_of_block: bool,
    eof: bool,
    total_in: u64,
    total_out: u64,
}

impl<'a> Flate<'a> {
    /// `FlateStream::reset()`
    fn new(input: &'a [u8]) -> Flate<'a> {
        let mut f = Flate {
            str: Input::new(input),
            buf: vec![0u8; FLATE_WINDOW],
            index: 0,
            remain: 0,
            code_buf: 0,
            code_size: 0,
            code_lengths: [0; FLATE_MAX_LIT_CODES + FLATE_MAX_DIST_CODES],
            lit_tab: HuffTab::default(),
            dist_tab: HuffTab::default(),
            compressed_block: false,
            block_len: 0,
            end_of_block: true,
            eof: true,
            total_in: 2,
            total_out: 0,
        };
        let cmf = f.str.get_char();
        let flg = f.str.get_char();
        if cmf == EOF || flg == EOF {
            return f;
        }
        if (cmf & 0x0f) != 0x08 {
            crate::error::error(
                crate::error::ErrorCategory::SyntaxError,
                -1,
                "Unknown compression method in flate stream",
            );
            return f;
        }
        if ((cmf << 8) + flg) % 31 != 0 {
            crate::error::error(crate::error::ErrorCategory::SyntaxError, -1, "Bad FCHECK in flate stream");
            return f;
        }
        if flg & 0x20 != 0 {
            crate::error::error(crate::error::ErrorCategory::SyntaxError, -1, "FDICT bit set in flate stream");
            return f;
        }
        f.eof = false;
        f
    }

    /// `FlateStream::getHuffmanCodeWord`
    fn get_huffman_code_word(&mut self, lit: bool) -> i32 {
        let max_len = if lit { self.lit_tab.max_len } else { self.dist_tab.max_len };
        while self.code_size < max_len {
            let c = self.str.get_char();
            if c == EOF {
                break;
            }
            self.code_buf |= ((c as u32) & 0xff) << self.code_size;
            self.code_size += 8;
            self.total_in += 1;
        }
        let tab = if lit { &self.lit_tab } else { &self.dist_tab };
        let idx = (self.code_buf & ((1u32 << tab.max_len) - 1)) as usize;
        let (len, val) = tab.codes[idx];
        if self.code_size == 0 || self.code_size < len as i32 || len == 0 {
            return EOF;
        }
        self.code_buf >>= len;
        self.code_size -= len as i32;
        val as i32
    }

    fn get_huffman_code_word_tab(&mut self, tab: &HuffTab) -> i32 {
        while self.code_size < tab.max_len {
            let c = self.str.get_char();
            if c == EOF {
                break;
            }
            self.code_buf |= ((c as u32) & 0xff) << self.code_size;
            self.code_size += 8;
            self.total_in += 1;
        }
        let idx = (self.code_buf & ((1u32 << tab.max_len) - 1)) as usize;
        let (len, val) = tab.codes[idx];
        if self.code_size == 0 || self.code_size < len as i32 || len == 0 {
            return EOF;
        }
        self.code_buf >>= len;
        self.code_size -= len as i32;
        val as i32
    }

    /// `FlateStream::getCodeWord`
    fn get_code_word(&mut self, bits: i32) -> i32 {
        while self.code_size < bits {
            let c = self.str.get_char();
            if c == EOF {
                return EOF;
            }
            self.code_buf |= ((c as u32) & 0xff) << self.code_size;
            self.code_size += 8;
            self.total_in += 1;
        }
        let c = (self.code_buf & ((1u32 << bits) - 1)) as i32;
        self.code_buf >>= bits;
        self.code_size -= bits;
        c
    }

    /// `FlateStream::startBlock`
    fn start_block(&mut self) -> bool {
        self.lit_tab = HuffTab::default();
        self.dist_tab = HuffTab::default();

        // read block header
        let mut block_hdr = self.get_code_word(3);
        if block_hdr & 1 != 0 {
            self.eof = true;
        }
        block_hdr >>= 1;

        let ok = if block_hdr == 0 {
            // uncompressed block
            self.compressed_block = false;
            let mut ok = true;
            let mut bytes = [0i32; 4];
            for b in bytes.iter_mut() {
                let c = self.str.get_char();
                if c == EOF {
                    ok = false;
                    break;
                }
                *b = c & 0xff;
            }
            if ok {
                self.block_len = bytes[0] | (bytes[1] << 8);
                let check = bytes[2] | (bytes[3] << 8);
                if check != (!self.block_len & 0xffff) {
                    ok = false;
                } else {
                    // NB: xpdf discards *all* buffered bits here, even
                    // whole bytes that were read ahead
                    self.code_buf = 0;
                    self.code_size = 0;
                    self.total_in += 4;
                }
            }
            ok
        } else if block_hdr == 1 {
            // compressed block with fixed codes
            self.compressed_block = true;
            let (l, d) = fixed_tables_cached();
            self.lit_tab = l;
            self.dist_tab = d;
            true
        } else if block_hdr == 2 {
            // compressed block with dynamic codes
            self.compressed_block = true;
            self.read_dynamic_codes()
        } else {
            false
        };
        if !ok {
            crate::error::error(crate::error::ErrorCategory::SyntaxError, -1, "Bad block header in flate stream");
            self.end_of_block = true;
            self.eof = true;
            return false;
        }
        self.end_of_block = false;
        true
    }

    /// `FlateStream::readDynamicCodes`
    fn read_dynamic_codes(&mut self) -> bool {
        let ok = (|| -> Option<()> {
            let num_lit_codes = self.get_code_word(5);
            if num_lit_codes == EOF {
                return None;
            }
            let num_lit_codes = (num_lit_codes + 257) as usize;
            let num_dist_codes = self.get_code_word(5);
            if num_dist_codes == EOF {
                return None;
            }
            let num_dist_codes = (num_dist_codes + 1) as usize;
            let num_code_len_codes = self.get_code_word(4);
            if num_code_len_codes == EOF {
                return None;
            }
            let num_code_len_codes = (num_code_len_codes + 4) as usize;
            if num_lit_codes > FLATE_MAX_LIT_CODES
                || num_dist_codes > FLATE_MAX_DIST_CODES
                || num_code_len_codes > FLATE_MAX_CODE_LEN_CODES
            {
                return None;
            }

            // build the code length code table
            let mut code_len_code_lengths = [0i32; FLATE_MAX_CODE_LEN_CODES];
            for &m in CODE_LEN_CODE_MAP.iter().take(num_code_len_codes) {
                let v = self.get_code_word(3);
                if v == -1 {
                    return None;
                }
                code_len_code_lengths[m] = v;
            }
            let code_len_code_tab = comp_huffman_codes(&code_len_code_lengths);

            // build the literal and distance code tables
            let total = num_lit_codes + num_dist_codes;
            let mut len = 0;
            let mut i = 0usize;
            while i < total {
                let code = self.get_huffman_code_word_tab(&code_len_code_tab);
                if code == EOF {
                    return None;
                }
                if code == 16 {
                    let repeat = self.get_code_word(2);
                    if repeat == EOF {
                        return None;
                    }
                    let repeat = (repeat + 3) as usize;
                    if i + repeat > total {
                        return None;
                    }
                    for _ in 0..repeat {
                        self.code_lengths[i] = len;
                        i += 1;
                    }
                } else if code == 17 {
                    let repeat = self.get_code_word(3);
                    if repeat == EOF {
                        return None;
                    }
                    let repeat = (repeat + 3) as usize;
                    if i + repeat > total {
                        return None;
                    }
                    len = 0;
                    for _ in 0..repeat {
                        self.code_lengths[i] = 0;
                        i += 1;
                    }
                } else if code == 18 {
                    let repeat = self.get_code_word(7);
                    if repeat == EOF {
                        return None;
                    }
                    let repeat = (repeat + 11) as usize;
                    if i + repeat > total {
                        return None;
                    }
                    len = 0;
                    for _ in 0..repeat {
                        self.code_lengths[i] = 0;
                        i += 1;
                    }
                } else {
                    len = code;
                    self.code_lengths[i] = code;
                    i += 1;
                }
            }
            self.lit_tab = comp_huffman_codes(&self.code_lengths[..num_lit_codes]);
            self.dist_tab = comp_huffman_codes(&self.code_lengths[num_lit_codes..total]);
            Some(())
        })();
        if ok.is_none() {
            crate::error::error(
                crate::error::ErrorCategory::SyntaxError,
                -1,
                "Bad dynamic code table in flate stream",
            );
            return false;
        }
        true
    }

    /// `FlateStream::readSome`
    fn read_some(&mut self) {
        if self.end_of_block && !self.start_block() {
            return;
        }
        let mut err = false;
        if self.compressed_block {
            'blk: {
                let code1 = self.get_huffman_code_word(true);
                if code1 == EOF {
                    err = true;
                    break 'blk;
                }
                if code1 < 256 {
                    self.buf[self.index] = code1 as u8;
                    self.remain = 1;
                } else if code1 == 256 {
                    self.end_of_block = true;
                    self.remain = 0;
                } else {
                    let c1 = (code1 - 257) as usize;
                    // c1 < 31 because the literal table has at most 288
                    // codes
                    let (lbits, lfirst) = LENGTH_DECODE[c1];
                    let mut code2 = lbits;
                    if code2 > 0 {
                        code2 = self.get_code_word(code2);
                        if code2 == EOF {
                            err = true;
                            break 'blk;
                        }
                    }
                    let len = (lfirst + code2) as usize;
                    let dcode = self.get_huffman_code_word(false);
                    if dcode == EOF {
                        err = true;
                        break 'blk;
                    }
                    let (dbits, dfirst) = DIST_DECODE[dcode as usize];
                    let mut code2 = dbits;
                    if code2 > 0 {
                        code2 = self.get_code_word(code2);
                        if code2 == EOF {
                            err = true;
                            break 'blk;
                        }
                    }
                    let dist = (dfirst + code2) as usize;
                    let mut dest = self.index;
                    let mut src = (self.index.wrapping_sub(dist)) & FLATE_MASK;
                    for _ in 0..len {
                        self.buf[dest] = self.buf[src];
                        dest = (dest + 1) & FLATE_MASK;
                        src = (src + 1) & FLATE_MASK;
                    }
                    self.remain = len;
                }
            }
        } else {
            let len = if (self.block_len as usize) < FLATE_WINDOW {
                self.block_len as usize
            } else {
                FLATE_WINDOW
            };
            let mut i = 0usize;
            let mut j = self.index;
            while i < len {
                let c = self.str.get_char();
                if c == EOF {
                    self.end_of_block = true;
                    self.eof = true;
                    break;
                }
                self.buf[j] = c as u8;
                i += 1;
                j = (j + 1) & FLATE_MASK;
            }
            self.remain = i;
            self.block_len -= len as i32;
            if self.block_len == 0 {
                self.end_of_block = true;
            }
            self.total_in += self.remain as u64;
        }
        if err {
            crate::error::error(
                crate::error::ErrorCategory::SyntaxError,
                -1,
                "Unexpected end of file in flate stream",
            );
            self.end_of_block = true;
            self.eof = true;
            self.remain = 0;
            return;
        }
        self.total_out += self.remain as u64;

        // check for a 'decompression bomb'
        if self.total_out > BOMB_SIZE_THRESHOLD && self.total_in < self.total_out / BOMB_RATIO_THRESHOLD {
            crate::error::error(crate::error::ErrorCategory::SyntaxError, -1, "Decompression bomb in flate stream");
            self.end_of_block = true;
            self.eof = true;
            self.remain = 0;
        }
    }
}

fn fixed_tables_cached() -> (HuffTab, HuffTab) {
    use std::sync::OnceLock;
    static T: OnceLock<(HuffTab, HuffTab)> = OnceLock::new();
    T.get_or_init(fixed_tables).clone()
}

/// `FlateStream` without predictor: all bytes `getRawChar()` returns.
pub fn flate_decode(input: &[u8]) -> Vec<u8> {
    let mut f = Flate::new(input);
    let mut out = Vec::with_capacity(input.len() * 4);
    loop {
        while f.remain == 0 {
            if f.end_of_block && f.eof {
                return out;
            }
            f.read_some();
        }
        // copy the produced bytes (remain bytes starting at index)
        while f.remain > 0 {
            let k = f.remain.min(FLATE_WINDOW - f.index);
            out.extend_from_slice(&f.buf[f.index..f.index + k]);
            f.index = (f.index + k) & FLATE_MASK;
            f.remain -= k;
        }
    }
}

// ------------------------------------------------------------------------
// StreamPredictor
// ------------------------------------------------------------------------

/// `gfxColorMaxComps` (= `funcMaxOutputs`)
const GFX_COLOR_MAX_COMPS: i32 = 32;

/// Predictor parameters (`/Predictor`, `/Columns`, `/Colors`,
/// `/BitsPerComponent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PredictorParams {
    pub predictor: i32,
    pub width: i32,
    pub n_comps: i32,
    pub n_bits: i32,
}

impl PredictorParams {
    /// `StreamPredictor::StreamPredictor` -> `isOk()`.  When this returns
    /// false xpdf silently drops the predictor.
    pub fn is_ok(&self) -> bool {
        let (width, n_comps, n_bits) = (self.width, self.n_comps, self.n_bits);
        if width <= 0 || n_comps <= 0 || n_bits <= 0 || n_comps > GFX_COLOR_MAX_COMPS || n_bits > 16 {
            return false;
        }
        if width >= i32::MAX / n_comps {
            return false;
        }
        let n_vals = width * n_comps;
        if n_vals >= (i32::MAX - 7) / n_bits {
            return false;
        }
        true
    }
}

/// Apply `StreamPredictor` to the raw decoder output.
pub fn apply_predictor(raw: &[u8], p: &PredictorParams) -> Vec<u8> {
    let predictor = p.predictor;
    let width = p.width;
    let n_comps = p.n_comps;
    let n_bits = p.n_bits;
    let n_vals = width * n_comps;
    let pix_bytes = ((n_comps * n_bits + 7) >> 3) as usize;
    let row_bytes = (((n_vals * n_bits + 7) >> 3) as usize) + pix_bytes;
    let mut pred_line = vec![0u8; row_bytes];
    let mut str = Input::new(raw);
    let mut out = Vec::with_capacity(raw.len());
    let mut up_left_buf = [0u8; (GFX_COLOR_MAX_COMPS * 2 + 1) as usize];

    // getNextLine() until it fails; each successful line yields
    // predLine[pixBytes..rowBytes]
    loop {
        let cur_pred;
        if predictor >= 10 {
            let c = str.get_char();
            if c == EOF {
                break;
            }
            cur_pred = c + 10;
        } else {
            cur_pred = predictor;
        }

        // read the raw line, apply PNG (byte) predictor
        for b in up_left_buf.iter_mut().take(pix_bytes + 1) {
            *b = 0;
        }
        let mut fail = false;
        let mut i = pix_bytes;
        while i < row_bytes {
            let mut j = pix_bytes;
            while j > 0 {
                up_left_buf[j] = up_left_buf[j - 1];
                j -= 1;
            }
            up_left_buf[0] = pred_line[i];
            let c = str.get_char();
            if c == EOF {
                if i > pix_bytes {
                    // this ought to return false, but some (broken) PDF
                    // files contain truncated image data, and Adobe
                    // apparently reads the last partial line
                    break;
                }
                fail = true;
                break;
            }
            let c = c as u8;
            match cur_pred {
                11 => pred_line[i] = pred_line[i - pix_bytes].wrapping_add(c),
                12 => pred_line[i] = pred_line[i].wrapping_add(c),
                13 => {
                    pred_line[i] =
                        (((pred_line[i - pix_bytes] as i32 + pred_line[i] as i32) >> 1) as u8).wrapping_add(c)
                }
                14 => {
                    let left = pred_line[i - pix_bytes] as i32;
                    let up = pred_line[i] as i32;
                    let up_left = up_left_buf[pix_bytes] as i32;
                    let pp = left + up - up_left;
                    let pa = (pp - left).abs();
                    let pb = (pp - up).abs();
                    let pc = (pp - up_left).abs();
                    pred_line[i] = if pa <= pb && pa <= pc {
                        (left as u8).wrapping_add(c)
                    } else if pb <= pc {
                        (up as u8).wrapping_add(c)
                    } else {
                        (up_left as u8).wrapping_add(c)
                    };
                }
                _ => pred_line[i] = c, // 10 (PNG none), no predictor, TIFF
            }
            i += 1;
        }
        if fail {
            break;
        }

        // apply TIFF (component) predictor
        if predictor == 2 {
            if n_bits == 8 {
                let nc = n_comps as usize;
                for i in pix_bytes..row_bytes {
                    pred_line[i] = pred_line[i].wrapping_add(pred_line[i - nc]);
                }
            } else if n_bits == 16 {
                let nc2 = 2 * n_comps as usize;
                let mut i = pix_bytes;
                while i < row_bytes {
                    let c = ((pred_line[i] as i32 + pred_line[i - nc2] as i32) << 8)
                        + pred_line[i + 1] as i32
                        + pred_line[i + 1 - nc2] as i32;
                    pred_line[i] = (c >> 8) as u8;
                    pred_line[i + 1] = (c & 0xff) as u8;
                    i += 2;
                }
            } else {
                let nc = n_comps as usize;
                for b in up_left_buf.iter_mut().take(nc) {
                    *b = 0;
                }
                let bit_mask: u64 = (1u64 << n_bits) - 1;
                let mut in_buf: u64 = 0;
                let mut out_buf: u64 = 0;
                let mut in_bits = 0i32;
                let mut out_bits = 0i32;
                let mut j = pix_bytes;
                let mut k = pix_bytes;
                for _ in 0..width {
                    for kk in 0..nc {
                        if in_bits < n_bits {
                            in_buf = (in_buf << 8) | (pred_line[j] as u64 & 0xff);
                            j += 1;
                            in_bits += 8;
                        }
                        up_left_buf[kk] = ((up_left_buf[kk] as u64 + (in_buf >> (in_bits - n_bits))) & bit_mask) as u8;
                        in_bits -= n_bits;
                        out_buf = (out_buf << n_bits) | up_left_buf[kk] as u64;
                        out_bits += n_bits;
                        if out_bits >= 8 {
                            pred_line[k] = (out_buf >> (out_bits - 8)) as u8;
                            k += 1;
                            out_bits -= 8;
                        }
                    }
                }
                if out_bits > 0 {
                    pred_line[k] = ((out_buf << (8 - out_bits))
                        .wrapping_add(in_buf & ((1u64 << (8 - out_bits)) - 1)))
                        as u8;
                }
            }
        }

        out.extend_from_slice(&pred_line[pix_bytes..row_bytes]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_hex() {
        assert_eq!(ascii_hex_decode(b"48 65\n6c6C6f>"), b"Hello");
        // odd number of digits: last digit padded with 0
        assert_eq!(ascii_hex_decode(b"414>"), &[0x41, 0x40]);
        // EOF without '>' -> a final 0 byte is produced (xpdf quirk)
        assert_eq!(ascii_hex_decode(b"41"), &[0x41, 0x00]);
        assert_eq!(ascii_hex_decode(b""), &[0x00]);
    }

    #[test]
    fn ascii85() {
        assert_eq!(ascii85_decode(b"87cURD]i,\"Ebo80~>"), b"Hello World!");
        assert_eq!(ascii85_decode(b"z~>"), &[0, 0, 0, 0]);
        assert_eq!(ascii85_decode(b"~>"), b"");
        // a lone char before '~' still yields b[0] (n = 0 but lookChar
        // returns b[index]); xpdf does not skip a "<~" prefix
        assert_eq!(ascii85_decode(b"<~"), &[0x57]);
    }

    #[test]
    fn run_length() {
        assert_eq!(run_length_decode(&[2, b'a', b'b', b'c', 0xfe, b'x', 0x80]), b"abcxxx");
        // truncated literal run -> 0xff bytes
        assert_eq!(run_length_decode(&[2, b'a']), &[b'a', 0xff, 0xff]);
    }

    #[test]
    fn lzw_spec_example() {
        // PDF Reference example: 80 0B 60 50 22 0C 0C 85 01
        let enc = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(lzw_decode(&enc, 1), &[45, 45, 45, 45, 45, 65, 45, 45, 45, 66]);
    }

    #[test]
    fn flate_stored_and_fixed() {
        // zlib header + stored block "abc"
        let enc = [0x78, 0x01, 0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c'];
        assert_eq!(flate_decode(&enc), b"abc");
        // fixed Huffman: zlib.compress(b"a")
        let enc = [0x78, 0x9c, 0x4b, 0x04, 0x00, 0x00, 0x62, 0x00, 0x62];
        assert_eq!(flate_decode(&enc), b"a");
        // truncated after the literal: the partial code at EOF is still
        // accepted, the missing end-of-block code ends the stream
        assert_eq!(flate_decode(&enc[..4]), b"a");
        // bad header -> nothing
        assert_eq!(flate_decode(&[0x00, 0x00, 0x01]), b"");
    }

    #[test]
    fn png_up_predictor() {
        // 2 rows of 3 columns, PNG Up (tag 2)
        let raw = [2u8, 1, 2, 3, 2, 1, 1, 1];
        let p = PredictorParams { predictor: 12, width: 3, n_comps: 1, n_bits: 8 };
        assert_eq!(apply_predictor(&raw, &p), &[1, 2, 3, 2, 3, 4]);
        // truncated last row: the stale tail of the previous row is kept
        let raw = [2u8, 1, 2, 3, 2, 1];
        assert_eq!(apply_predictor(&raw, &p), &[1, 2, 3, 2, 2, 3]);
    }
}
