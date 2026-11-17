//! Port of pdftexdir/writet1.c: reading .enc files, Type 1 font embedding and subsetting.
//!
//! The C code works on NUL-terminated lines in the growable arrays t1_line_array and
//! t1_buf_array (which keep stale bytes beyond the terminator); `line`/`buf` below are
//! those arrays, with `line_ptr`/`buf_ptr` the C pointers as indices.

use std::collections::BTreeMap;
use std::ops::Bound;

use super::*;
use crate::gen::globals::Globals;
use crate::rt::kpse;

const T1_BUF_SIZE: usize = 0x10;
const ENC_BUF_SIZE: usize = 0x1000;

const CS_HSTEM: usize = 1;
const CS_VSTEM: usize = 3;
const CS_VMOVETO: usize = 4;
const CS_RLINETO: usize = 5;
const CS_HLINETO: usize = 6;
const CS_VLINETO: usize = 7;
const CS_RRCURVETO: usize = 8;
const CS_CLOSEPATH: usize = 9;
const CS_CALLSUBR: usize = 10;
const CS_RETURN: usize = 11;
const CS_ESCAPE: usize = 12;
const CS_HSBW: usize = 13;
const CS_ENDCHAR: usize = 14;
const CS_RMOVETO: usize = 21;
const CS_HMOVETO: usize = 22;
const CS_VHCURVETO: usize = 30;
const CS_HVCURVETO: usize = 31;
const CS_1BYTE_MAX: usize = CS_HVCURVETO + 1;
const CS_DOTSECTION: usize = CS_1BYTE_MAX;
const CS_VSTEM3: usize = CS_1BYTE_MAX + 1;
const CS_HSTEM3: usize = CS_1BYTE_MAX + 2;
const CS_SEAC: usize = CS_1BYTE_MAX + 6;
const CS_SBW: usize = CS_1BYTE_MAX + 7;
const CS_DIV: usize = CS_1BYTE_MAX + 12;
const CS_CALLOTHERSUBR: usize = CS_1BYTE_MAX + 16;
const CS_POP: usize = CS_1BYTE_MAX + 17;
const CS_SETCURRENTPOINT: usize = CS_1BYTE_MAX + 33;
const CS_2BYTE_MAX: usize = CS_SETCURRENTPOINT + 1;
const CS_MAX: usize = CS_2BYTE_MAX;

const T1_C1: u32 = 52845;
const T1_C2: u32 = 22719;
const POST_SUBRS_SCAN: i32 = 5;

const CS_TOKEN_PAIRS: [(&[u8], &[u8]); 4] = [(b" RD", b"NP"), (b" -|", b"|"), (b" RD", b"noaccess put"), (b" -|", b"noaccess put")];

const CHARSTRINGNAME: &[u8] = b"/CharStrings";

pub static STANDARD_GLYPH_NAMES: [Option<&str>; 256] = {
    const N: Option<&str> = None;
    let mut t = [N; 256];
    let names: [(usize, &str); 149] = [
        (0x20, "space"), (0x21, "exclam"), (0x22, "quotedbl"), (0x23, "numbersign"), (0x24, "dollar"),
        (0x25, "percent"), (0x26, "ampersand"), (0x27, "quoteright"), (0x28, "parenleft"), (0x29, "parenright"),
        (0x2a, "asterisk"), (0x2b, "plus"), (0x2c, "comma"), (0x2d, "hyphen"), (0x2e, "period"), (0x2f, "slash"),
        (0x30, "zero"), (0x31, "one"), (0x32, "two"), (0x33, "three"), (0x34, "four"), (0x35, "five"),
        (0x36, "six"), (0x37, "seven"), (0x38, "eight"), (0x39, "nine"), (0x3a, "colon"), (0x3b, "semicolon"),
        (0x3c, "less"), (0x3d, "equal"), (0x3e, "greater"), (0x3f, "question"), (0x40, "at"), (0x41, "A"),
        (0x42, "B"), (0x43, "C"), (0x44, "D"), (0x45, "E"), (0x46, "F"), (0x47, "G"), (0x48, "H"), (0x49, "I"),
        (0x4a, "J"), (0x4b, "K"), (0x4c, "L"), (0x4d, "M"), (0x4e, "N"), (0x4f, "O"), (0x50, "P"), (0x51, "Q"),
        (0x52, "R"), (0x53, "S"), (0x54, "T"), (0x55, "U"), (0x56, "V"), (0x57, "W"), (0x58, "X"), (0x59, "Y"),
        (0x5a, "Z"), (0x5b, "bracketleft"), (0x5c, "backslash"), (0x5d, "bracketright"), (0x5e, "asciicircum"),
        (0x5f, "underscore"), (0x60, "quoteleft"), (0x61, "a"), (0x62, "b"), (0x63, "c"), (0x64, "d"),
        (0x65, "e"), (0x66, "f"), (0x67, "g"), (0x68, "h"), (0x69, "i"), (0x6a, "j"), (0x6b, "k"), (0x6c, "l"),
        (0x6d, "m"), (0x6e, "n"), (0x6f, "o"), (0x70, "p"), (0x71, "q"), (0x72, "r"), (0x73, "s"), (0x74, "t"),
        (0x75, "u"), (0x76, "v"), (0x77, "w"), (0x78, "x"), (0x79, "y"), (0x7a, "z"), (0x7b, "braceleft"),
        (0x7c, "bar"), (0x7d, "braceright"), (0x7e, "asciitilde"), (0xa1, "exclamdown"), (0xa2, "cent"),
        (0xa3, "sterling"), (0xa4, "fraction"), (0xa5, "yen"), (0xa6, "florin"), (0xa7, "section"),
        (0xa8, "currency"), (0xa9, "quotesingle"), (0xaa, "quotedblleft"), (0xab, "guillemotleft"),
        (0xac, "guilsinglleft"), (0xad, "guilsinglright"), (0xae, "fi"), (0xaf, "fl"), (0xb1, "endash"),
        (0xb2, "dagger"), (0xb3, "daggerdbl"), (0xb4, "periodcentered"), (0xb6, "paragraph"), (0xb7, "bullet"),
        (0xb8, "quotesinglbase"), (0xb9, "quotedblbase"), (0xba, "quotedblright"), (0xbb, "guillemotright"),
        (0xbc, "ellipsis"), (0xbd, "perthousand"), (0xbf, "questiondown"), (0xc1, "grave"), (0xc2, "acute"),
        (0xc3, "circumflex"), (0xc4, "tilde"), (0xc5, "macron"), (0xc6, "breve"), (0xc7, "dotaccent"),
        (0xc8, "dieresis"), (0xca, "ring"), (0xcb, "cedilla"), (0xcd, "hungarumlaut"), (0xce, "ogonek"),
        (0xcf, "caron"), (0xd0, "emdash"), (0xe1, "AE"), (0xe3, "ordfeminine"), (0xe8, "Lslash"), (0xe9, "Oslash"),
        (0xea, "OE"), (0xeb, "ordmasculine"), (0xf1, "ae"), (0xf5, "dotlessi"), (0xf8, "lslash"), (0xf9, "oslash"),
        (0xfa, "oe"), (0xfb, "germandbls"),
    ];
    let mut i = 0;
    while i < names.len() {
        t[names[i].0] = Some(names[i].1);
        i += 1;
    }
    t
};

fn std_glyph(i: i32) -> GlyphName {
    if (0..256).contains(&i) {
        STANDARD_GLYPH_NAMES[i as usize].map(|s| s.as_bytes().to_vec())
    } else {
        None
    }
}

#[derive(Clone, Copy, Default)]
struct CcEntry {
    nargs: u8,
    bottom: bool,
    clear: bool,
    valid: bool,
}

/// writet1.c cs_entry; `name` None = notdef (subrs never use names)
#[derive(Clone, Default)]
struct CsEntry {
    name: GlyphName,
    data: Vec<u8>,
    len: u16,
    cslen: u16,
    used: bool,
    valid: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum T1Enc {
    #[default]
    Standard,
    Builtin,
}

/// writet1.c static state
pub struct T1State {
    pub length1: i32,
    pub length2: i32,
    pub length3: i32,
    save_offset: i32,
    fontname_offset: i32,
    fd_cur: usize,
    encoding: T1Enc,
    dr: u16,
    er: u16,
    cslen: u16,
    len_iv: i16,
    enc_line: Vec<u8>,
    line: Vec<u8>,
    line_ptr: usize,
    line_limit: usize,
    buf: Vec<u8>,
    buf_ptr: usize,
    buf_limit: usize,
    cs_start: usize,
    cs_tab: Option<Vec<CsEntry>>,
    cs_ptr: usize,
    cs_notdef: Option<usize>,
    cs_dict_start: Vec<u8>,
    cs_dict_end: Vec<u8>,
    cs_count: i32,
    cs_size: i32,
    cs_size_pos: usize,
    subr_tab: Option<Vec<CsEntry>>,
    subr_array_start: Vec<u8>,
    subr_array_end: Vec<u8>,
    subr_max: i32,
    subr_size: i32,
    subr_size_pos: usize,
    cs_token_pair: Option<usize>,
    pfa: bool,
    cs: bool,
    scan: bool,
    eexec_encrypt: bool,
    synthetic: bool,
    in_eexec: i32,
    block_length: i32,
    last_hexbyte: i32,
    file: CFile,
    enc_file: CFile,
    cc_stack: [i32; 64],
    stack_ptr: usize,
    cc_tab: [CcEntry; CS_MAX],
    is_cc_init: bool,
    last_arg_other_subr3: i32,
}

impl Default for T1State {
    fn default() -> Self {
        T1State {
            length1: 0,
            length2: 0,
            length3: 0,
            save_offset: 0,
            fontname_offset: 0,
            fd_cur: 0,
            encoding: T1Enc::Standard,
            dr: 0,
            er: 0,
            cslen: 0,
            len_iv: 0,
            enc_line: vec![0; ENC_BUF_SIZE],
            line: Vec::new(),
            line_ptr: 0,
            line_limit: 0,
            buf: Vec::new(),
            buf_ptr: 0,
            buf_limit: 0,
            cs_start: 0,
            cs_tab: None,
            cs_ptr: 0,
            cs_notdef: None,
            cs_dict_start: Vec::new(),
            cs_dict_end: Vec::new(),
            cs_count: 0,
            cs_size: 0,
            cs_size_pos: 0,
            subr_tab: None,
            subr_array_start: Vec::new(),
            subr_array_end: Vec::new(),
            subr_max: 0,
            subr_size: 0,
            subr_size_pos: 0,
            cs_token_pair: None,
            pfa: false,
            cs: false,
            scan: false,
            eexec_encrypt: false,
            synthetic: false,
            in_eexec: 0,
            block_length: 0,
            last_hexbyte: 0,
            file: CFile::default(),
            enc_file: CFile::default(),
            cc_stack: [0; 64],
            stack_ptr: 0,
            cc_tab: [CcEntry::default(); CS_MAX],
            is_cc_init: false,
            last_arg_other_subr3: 3,
        }
    }
}

/// str_suffix(begin_buf, end_buf, s)
fn str_suffix(buf: &[u8], end: usize, s: &[u8]) -> bool {
    if end == 0 {
        return false;
    }
    let mut s1 = end as isize - 1;
    let mut s2 = s.len() as isize - 1;
    if at(buf, s1 as usize) == 10 {
        s1 -= 1;
    }
    while s1 >= 0 && s2 >= 0 {
        if at(buf, s1 as usize) != s[s2 as usize] {
            return false;
        }
        s1 -= 1;
        s2 -= 1;
    }
    s2 < 0
}

fn hexval(c: i32) -> i32 {
    if (b'A' as i32..=b'F' as i32).contains(&c) {
        c - b'A' as i32 + 10
    } else if (b'a' as i32..=b'f' as i32).contains(&c) {
        c - b'a' as i32 + 10
    } else if (b'0' as i32..=b'9' as i32).contains(&c) {
        c - b'0' as i32
    } else {
        -1
    }
}

#[inline]
fn cdecrypt(cipher: u8, cr: &mut u16) -> u8 {
    let plain = cipher ^ (*cr >> 8) as u8;
    *cr = ((cipher as u32 + *cr as u32).wrapping_mul(T1_C1).wrapping_add(T1_C2)) as u16;
    plain
}

#[inline]
fn cencrypt(plain: u8, cr: &mut u16) -> u8 {
    let cipher = plain ^ (*cr >> 8) as u8;
    *cr = ((cipher as u32 + *cr as u32).wrapping_mul(T1_C1).wrapping_add(T1_C2)) as u16;
    cipher
}

impl T1State {
    fn alloc_line(&mut self, n: usize) {
        if self.line_limit == 0 {
            self.line_limit = T1_BUF_SIZE.max(n);
            self.line_ptr = 0;
        } else if self.line_ptr + n > self.line_limit {
            self.line_limit *= 2;
            if self.line_ptr + n > self.line_limit {
                self.line_limit = self.line_ptr + n;
            }
        }
        ensure_len(&mut self.line, self.line_limit + 1);
    }
    fn alloc_buf(&mut self, n: usize, s: usize) {
        if self.buf_limit == 0 {
            self.buf_limit = s.max(n);
            self.buf_ptr = 0;
        } else if self.buf_ptr + n > self.buf_limit {
            self.buf_limit *= 2;
            if self.buf_ptr + n > self.buf_limit {
                self.buf_limit = self.buf_ptr + n;
            }
        }
        ensure_len(&mut self.buf, self.buf_limit + 1);
    }
    fn lset(&mut self, i: usize, b: u8) {
        ensure_len(&mut self.line, i + 2);
        self.line[i] = b;
    }
    /// strcpy(line + i, s)
    fn lstrcpy(&mut self, i: usize, s: &[u8]) {
        ensure_len(&mut self.line, i + s.len() + 2);
        self.line[i..i + s.len()].copy_from_slice(s);
        self.line[i + s.len()] = 0;
    }
    fn bstrcpy(&mut self, i: usize, s: &[u8]) {
        ensure_len(&mut self.buf, i + s.len() + 2);
        self.buf[i..i + s.len()].copy_from_slice(s);
        self.buf[i + s.len()] = 0;
    }
    fn lstr(&self, i: usize) -> Vec<u8> {
        c_str(&self.line, i).to_vec()
    }
    fn prefix(&self, s: &[u8]) -> bool {
        str_prefix(&self.line, s)
    }
    fn suffix(&self, s: &[u8]) -> bool {
        str_suffix(&self.line, self.line_ptr, s)
    }
    fn charstrings(&self) -> bool {
        c_strstr(&self.line, 0, CHARSTRINGNAME).is_some()
    }
    fn subrs(&self) -> bool {
        self.prefix(b"/Subrs")
    }
    fn end_eexec(&self) -> bool {
        self.suffix(b"mark currentfile closefile")
    }
    /// eol(line + i): append a newline if missing; returns the end index
    fn eol(&mut self, i: usize) -> usize {
        let mut p = i + c_strlen(&self.line, i);
        if p - i > 1 && self.line[p - 1] != 10 {
            self.lset(p, 10);
            p += 1;
            self.lset(p, 0);
        }
        p
    }
}

impl Globals {
    fn t1(&mut self) -> &mut T1State {
        &mut self.rt.fonts.t1
    }

    /// writet1.c enc_getline
    fn enc_getline(&mut self) {
        loop {
            if self.t1().enc_file.feof() {
                self.pdftex_fail("unexpected end of file");
            }
            let mut p: Vec<u8> = Vec::new();
            loop {
                let mut c = self.t1().enc_file.getc();
                if !append_char_to_buf(&mut c, &mut p, ENC_BUF_SIZE) {
                    self.check_buf_fail("writet1.c", 223);
                }
                if c == 10 {
                    break;
                }
            }
            if !append_eol(&mut p, ENC_BUF_SIZE) {
                self.check_buf_fail("writet1.c", 225);
            }
            let restart = p.len() < 2 || p[0] == b'%';
            let el = &mut self.t1().enc_line;
            el[..p.len()].copy_from_slice(&p);
            el[p.len()] = 0;
            if !restart {
                return;
            }
        }
    }

    /// remove_eol(p, buf) + the string for the failure message
    fn removed_eol(buf: &[u8]) -> String {
        let mut s = c_str(buf, 0).to_vec();
        if s.last() == Some(&10) {
            s.pop();
        }
        bs(&s)
    }

    /// writet1.c load_enc_file: read an encoding from an .enc file, or pdftex_fail().
    pub fn load_enc_file(&mut self, enc_name: &[u8]) -> Option<GlyphNames> {
        self.set_cur_file_name(enc_name);
        let f = match self.font_open_input(kpse::ENC_FORMAT) {
            Some(f) => f,
            None => self.pdftex_fail("cannot open encoding file for reading"),
        };
        self.t1().enc_file = f;
        let mut glyph_names = new_glyph_names();
        self.tex_printf("{");
        self.cur_file_name_from_nameoffile();
        let n = self.name_of_file();
        self.tex_printf_b(&n);
        self.enc_getline();
        let line = self.t1().enc_line.clone();
        let r0 = c_strchr(&line, 0, b'[');
        if line[0] != b'/' || r0.is_none() {
            let m = format!("invalid encoding vector (a name or `[' missing): `{}'", Self::removed_eol(&line));
            self.pdftex_fail(&m);
        }
        let mut line = line;
        let mut r = r0.unwrap() + 1;
        if line[r] == b' ' {
            r += 1;
        }
        let mut names_count = 0usize;
        loop {
            while line[r] == b'/' {
                r += 1;
                let st = r;
                while !matches!(line[r], b' ' | 10 | b']' | b'/') {
                    r += 1;
                }
                let buf = line[st..r].to_vec();
                if line[r] == b' ' {
                    r += 1;
                }
                if names_count > 255 {
                    self.pdftex_fail("encoding vector contains more than 256 names");
                }
                if buf != NOTDEF {
                    glyph_names[names_count] = Some(buf);
                }
                names_count += 1;
            }
            if line[r] != 10 && line[r] != b'%' {
                if str_prefix(&line[r..], b"] def") {
                    break;
                }
                let m = format!("invalid encoding vector: a name or `] def' expected: `{}'", Self::removed_eol(&line));
                self.pdftex_fail(&m);
            }
            self.enc_getline();
            line = self.t1().enc_line.clone();
            r = 0;
        }
        self.t1().enc_file = CFile::default();
        self.tex_printf("}");
        self.set_cur_file_name_none();
        Some(glyph_names)
    }

    /// writet1.c t1_check_pfa
    fn t1_check_pfa(&mut self) {
        let t = self.t1();
        let c = t.file.getc();
        t.pfa = c != 128;
        t.file.ungetc(c);
    }

    /// writet1.c t1_getbyte
    fn t1_getbyte(&mut self) -> i32 {
        let t = &mut self.rt.fonts.t1;
        let mut c = t.file.getc();
        if t.pfa {
            return c;
        }
        if t.block_length == 0 {
            if c != 128 {
                self.pdftex_fail("invalid marker");
            }
            let t = &mut self.rt.fonts.t1;
            c = t.file.getc();
            if c == 3 {
                while !t.file.feof() {
                    t.file.getc();
                }
                return -1;
            }
            let mut l = t.file.getc() & 0xff;
            l |= (t.file.getc() & 0xff) << 8;
            l |= (t.file.getc() & 0xff) << 16;
            l |= (t.file.getc() & 0xff) << 24;
            t.block_length = l;
            c = t.file.getc();
        }
        let t = &mut self.rt.fonts.t1;
        t.block_length = t.block_length.wrapping_sub(1);
        c
    }

    /// writet1.c edecrypt
    fn edecrypt(&mut self, mut cipher: u8) -> u8 {
        if self.t1().pfa {
            while cipher == 10 || cipher == 13 {
                cipher = self.t1_getbyte() as u8;
            }
            let h = hexval(cipher as i32) << 4;
            let l = hexval(self.t1_getbyte());
            cipher = (h + l) as u8;
            self.t1().last_hexbyte = cipher as i32;
        }
        let t = self.t1();
        let plain = cipher ^ (t.dr >> 8) as u8;
        t.dr = ((cipher as u32 + t.dr as u32).wrapping_mul(T1_C1).wrapping_add(T1_C2)) as u16;
        plain
    }

    /// writet1.c eencrypt
    fn eencrypt(&mut self, plain: u8) -> u8 {
        let t = self.t1();
        let cipher = plain ^ (t.er >> 8) as u8;
        t.er = ((cipher as u32 + t.er as u32).wrapping_mul(T1_C1).wrapping_add(T1_C2)) as u16;
        cipher
    }

    /// writet1.c t1_scan_num on line[p..]; r = end of the number
    fn t1_scan_num(&mut self, p: usize, want_r: bool) -> (f32, usize) {
        let line = &self.rt.fonts.t1.line;
        let mut p = p;
        if at(line, p) == b' ' {
            p += 1;
        }
        let f = match scan_float(line, p) {
            Some((v, _)) => v,
            None => {
                let m = format!("a number expected: `{}'", Self::removed_eol(line));
                self.pdftex_fail(&m);
            }
        };
        let mut r = p;
        if want_r {
            while matches!(at(line, r), b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
                r += 1;
            }
        }
        (f, r)
    }

    /// writet1.c t1_getline
    fn t1_getline(&mut self) {
        const EEXEC_STR: &[u8] = b"currentfile eexec";
        let eexec_len: i32 = 17;
        'exit: loop {
            if self.t1().file.feof() {
                self.pdftex_fail("unexpected end of file");
            }
            let t = self.t1();
            t.line_ptr = 0;
            t.alloc_line(1);
            t.cslen = 0;
            let mut eexec_scan: i32 = 0;
            let mut c = self.t1_getbyte();
            if c == -1 {
                break 'exit;
            }
            while !self.t1().file.feof() {
                if self.t1().in_eexec == 1 {
                    c = self.edecrypt(c as u8) as i32;
                }
                let t = self.t1();
                t.alloc_line(1);
                // append_char_to_buf(c, t1_line_ptr, t1_line_array, t1_line_limit)
                if c == 9 {
                    c = 32;
                }
                if c == 13 || c == -1 {
                    c = 10;
                }
                if c != 32 || (t.line_ptr > 0 && t.line[t.line_ptr - 1] != 32) {
                    let p = t.line_ptr;
                    t.lset(p, c as u8);
                    t.line_ptr += 1;
                }
                if t.in_eexec == 0 && eexec_scan >= 0 && eexec_scan < eexec_len {
                    if t.line[eexec_scan as usize] == EEXEC_STR[eexec_scan as usize] {
                        eexec_scan += 1;
                    } else {
                        eexec_scan = -1;
                    }
                }
                if c == 10 || (t.pfa && eexec_scan == eexec_len && c == 32) {
                    break;
                }
                if t.cs && t.cslen == 0 && t.line_ptr > 4 && (t.suffix(b" RD ") || t.suffix(b" -| ")) {
                    let mut p = t.line_ptr - 5;
                    while t.line[p] != b' ' {
                        p -= 1;
                    }
                    let (v, _) = self.t1_scan_num(p + 1, false);
                    let mut l = v as i32;
                    let t = self.t1();
                    t.cslen = l as u16;
                    t.cs_start = t.line_ptr;
                    t.alloc_line(l.max(0) as usize);
                    // the charstring's bytes, decrypted: a whole slice at once when they all
                    // lie in the current PFB segment (the byte loop below does the same, one
                    // t1_getbyte/edecrypt at a time, and handles segment boundaries and PFA)
                    {
                        let t = &mut self.rt.fonts.t1;
                        let n = l.max(0) as usize;
                        if !t.pfa && n > 0 && t.block_length >= l && t.file.pos + n <= t.file.data.len() {
                            let start = t.file.pos;
                            t.file.pos += n;
                            t.block_length -= l;
                            let lp = t.line_ptr;
                            ensure_len(&mut t.line, lp + n + 2);
                            let mut dr = t.dr;
                            for (k, &cipher) in t.file.data[start..start + n].iter().enumerate() {
                                t.line[lp + k] = cipher ^ (dr >> 8) as u8;
                                dr = ((cipher as u32 + dr as u32).wrapping_mul(T1_C1).wrapping_add(T1_C2)) as u16;
                            }
                            t.dr = dr;
                            t.line_ptr += n;
                            l = 0;
                        }
                    }
                    while l > 0 {
                        l -= 1;
                        let b = self.t1_getbyte() as u8;
                        let d = self.edecrypt(b);
                        let t = self.t1();
                        let p = t.line_ptr;
                        t.lset(p, d);
                        t.line_ptr += 1;
                    }
                }
                c = self.t1_getbyte();
            }
            let t = self.t1();
            t.alloc_line(2);
            // append_eol(t1_line_ptr, t1_line_array, t1_line_limit)
            let p = t.line_ptr;
            if p > 1 && t.line[p - 1] != 10 {
                t.lset(p, 10);
                t.line_ptr += 1;
            }
            let p = t.line_ptr;
            if p > 2 && t.line[p - 2] == 32 {
                t.line[p - 2] = 10;
                t.line_ptr -= 1;
            }
            let p = t.line_ptr;
            t.lset(p, 0);
            if t.line_ptr < 2 {
                continue;
            }
            if eexec_scan == eexec_len {
                t.in_eexec = 1;
            }
            break;
        }
        let t = self.t1();
        t.buf_ptr = 0;
        let ll = t.line_limit;
        t.alloc_buf(ll, ll);
    }

    /// writet1.c t1_putline
    fn t1_putline(&mut self) {
        let t = &self.rt.fonts.t1;
        if t.line_ptr <= 1 {
            return;
        }
        // fb_putchar of each byte (eencrypt'ed when eexec_encrypt), in bulk
        let n = t.line_ptr;
        let mut out = t.line[..n].to_vec();
        if t.eexec_encrypt {
            let mut er = t.er;
            for b in out.iter_mut() {
                let cipher = *b ^ (er >> 8) as u8;
                er = ((cipher as u32 + er as u32).wrapping_mul(T1_C1).wrapping_add(T1_C2)) as u16;
                *b = cipher;
            }
            self.rt.fonts.t1.er = er;
        }
        let u = &mut self.rt.pdfu;
        if u.fb.len() < u.fb_ptr + n {
            u.fb.resize(u.fb_ptr + n, 0);
        }
        u.fb[u.fb_ptr..u.fb_ptr + n].copy_from_slice(&out);
        u.fb_ptr += n;
    }

    /// writet1.c t1_puts
    fn t1_puts(&mut self, s: &[u8]) {
        let t = self.t1();
        t.lstrcpy(0, s);
        t.line_ptr = c_strlen(&t.line, 0);
        self.t1_putline();
    }

    /// writet1.c t1_init_params
    fn t1_init_params(&mut self, open_name_prefix: &str) {
        self.tex_printf(open_name_prefix);
        let c = self.rt.pdfu.cur_file_name.clone().unwrap_or_default();
        self.tex_printf_b(c.as_bytes());
        let t = self.t1();
        t.len_iv = 4;
        t.dr = 55665;
        t.er = 55665;
        t.in_eexec = 0;
        t.cs = false;
        t.scan = true;
        t.synthetic = false;
        t.eexec_encrypt = false;
        t.block_length = 0;
        self.t1_check_pfa();
    }

    /// writet1.c t1_close_font_file
    fn t1_close_font_file(&mut self, close_name_suffix: &str) {
        self.tex_printf(close_name_suffix);
        self.t1().file = CFile::default();
        self.set_cur_file_name_none();
    }

    /// writet1.c t1_check_block_len
    fn t1_check_block_len(&mut self, decrypt: bool) {
        if self.t1().block_length == 0 {
            return;
        }
        let mut c = self.t1_getbyte();
        if decrypt {
            c = self.edecrypt(c as u8) as i32;
        }
        let l = self.t1().block_length;
        if !(l == 0 && (c == 10 || c == 13)) {
            self.pdftex_fail(&format!("{} bytes more than expected", l.wrapping_add(1)));
        }
    }

    /// writet1.c t1_start_eexec
    fn t1_start_eexec(&mut self) {
        let off = self.fb_offset();
        let t = self.t1();
        t.length1 = off - t.save_offset;
        t.save_offset = off;
        if !t.pfa {
            self.t1_check_block_len(false);
        }
        self.t1().line_ptr = 0;
        for _ in 0..4 {
            let b = self.t1_getbyte() as u8;
            self.edecrypt(b);
            let t = self.t1();
            let p = t.line_ptr;
            t.lset(p, 0);
            t.line_ptr += 1;
        }
        self.t1().eexec_encrypt = true;
        self.t1_putline();
    }

    /// writet1.c t1_stop_eexec
    fn t1_stop_eexec(&mut self) {
        let off = self.fb_offset();
        let t = self.t1();
        t.length2 = off - t.save_offset;
        t.save_offset = off;
        t.eexec_encrypt = false;
        if !t.pfa {
            self.t1_check_block_len(true);
        } else {
            let b = self.t1_getbyte() as u8;
            let c = self.edecrypt(b);
            if !(c == 10 || c == 13) {
                if self.t1().last_hexbyte == 0 {
                    self.t1_puts(b"00");
                } else {
                    self.pdftex_fail("unexpected data after eexec");
                }
            }
        }
        let t = self.t1();
        t.cs = false;
        t.in_eexec = 2;
    }

    /// writet1.c t1_modify_fm: apply SlantFont/ExtendFont to /FontMatrix
    fn t1_modify_fm(&mut self) {
        let line = self.t1().lstr(0);
        let p0 = match c_strchr(&line, 0, b'[').or_else(|| c_strchr(&line, 0, b'{')) {
            Some(p) => p,
            None => {
                let m = format!("FontMatrix: an array expected: `{}'", Self::removed_eol(&line));
                self.pdftex_fail(&m);
            }
        };
        let c = line[p0];
        let mut p = p0 + 1;
        let mut out: Vec<u8> = line[..p].to_vec();
        let mut a = [0f32; 6];
        for v in a.iter_mut() {
            let (f, q) = self.t1_scan_num(p, true);
            *v = f;
            p = q;
        }
        let fdc = self.t1().fd_cur;
        let fm = self.fd(fdc).fm;
        let (slant, extend) = (self.fm(fm).slant, self.fm(fm).extend);
        if slant != 0 {
            let s = slant as f64 * 1e-3;
            a[0] = (a[0] as f64 + a[1] as f64 * s) as f32;
            a[2] = (a[2] as f64 + a[3] as f64 * s) as f32;
            a[4] = (a[4] as f64 + a[5] as f64 * s) as f32;
        }
        if extend != 0 {
            let e = extend as f64 * 1e-3;
            a[0] = (a[0] as f64 * e) as f32;
            a[2] = (a[2] as f64 * e) as f32;
            a[4] = (a[4] as f64 * e) as f32;
        }
        for v in a {
            out.extend_from_slice(fmt_g(v as f64).as_bytes());
            out.push(b' ');
        }
        let close = if c == b'[' { b']' } else { b'}' };
        while at(&line, p) != close && at(&line, p) != 0 {
            p += 1;
        }
        if at(&line, p) == 0 {
            let m = format!(
                "FontMatrix: cannot find the corresponding character to '{}': `{}'",
                c as char,
                Self::removed_eol(&line)
            );
            self.pdftex_fail(&m);
        }
        out.extend_from_slice(&line[p..]);
        let t = self.t1();
        t.bstrcpy(0, &out);
        t.lstrcpy(0, &out);
        t.line_ptr = t.eol(0);
    }

    /// writet1.c t1_modify_italic
    fn t1_modify_italic(&mut self) {
        let fd = self.t1().fd_cur;
        let slant = self.fm(self.fd(fd).fm).slant;
        if slant == 0 {
            return;
        }
        let line = self.t1().lstr(0);
        let p = c_strchr(&line, 0, b' ').unwrap_or(line.len());
        let mut out = line[..(p + 1).min(line.len())].to_vec();
        let (a, r) = self.t1_scan_num(p + 1, true);
        let a = (a as f64 - (slant as f64 * 1e-3).atan() * (180.0 / 3.14159265358979323846)) as f32;
        out.extend_from_slice(fmt_g(a as f64).as_bytes());
        out.extend_from_slice(&line[r.min(line.len())..]);
        let t = self.t1();
        t.bstrcpy(0, &out);
        t.lstrcpy(0, &out);
        t.line_ptr = t.eol(0);
        let e = &mut self.fd_mut(fd).font_dim[ITALIC_ANGLE_CODE];
        e.val = d2i((a as f64).round());
        e.set = true;
    }

    /// writet1.c t1_scan_keys
    fn t1_scan_keys(&mut self) {
        let fd = self.t1().fd_cur;
        let fm = self.fd(fd).fm;
        let (slant, extend) = (self.fm(fm).slant, self.fm(fm).extend);
        if extend != 0 || slant != 0 {
            if self.t1().prefix(b"/FontMatrix") {
                self.t1_modify_fm();
                return;
            }
            if self.t1().prefix(b"/ItalicAngle") {
                self.t1_modify_italic();
                return;
            }
        }
        if self.t1().prefix(b"/FontType") {
            let (v, _) = self.t1_scan_num(9, false);
            let i = v as i32;
            if i != 1 {
                self.pdftex_fail(&format!("Type{} fonts unsupported by pdfTeX", i));
            }
            return;
        }
        let line = self.t1().lstr(0);
        let Some(k) = (0..FONT_KEYS_NUM).find(|&k| !FONT_KEY[k].1.is_empty() && str_prefix(&line[1.min(line.len())..], FONT_KEY[k].1.as_bytes()))
        else {
            return;
        };
        let mut p = FONT_KEY[k].1.len() + 1;
        if at(&line, p) == b' ' {
            p += 1;
        }
        if k == FONTNAME_CODE {
            if at(&line, p) != b'/' {
                let m = format!("a name expected: `{}'", Self::removed_eol(&line));
                self.pdftex_fail(&m);
            }
            p += 1;
            let r = p;
            while at(&line, p) != b' ' && at(&line, p) != 10 {
                p += 1;
            }
            let mut name = line[r..p].to_vec();
            if slant != 0 {
                name.extend_from_slice(format!("-Slant_{}", slant).as_bytes());
            }
            if extend != 0 {
                name.extend_from_slice(format!("-Extend_{}", extend).as_bytes());
            }
            self.fd_mut(fd).fontname = Some(name.clone());
            if self.fm(fm).is_subsetted() {
                let off = self.fb_offset();
                self.t1().fontname_offset = off + r as i32;
                let rest = line[p..].to_vec();
                let mut s = b"ABCDEF+".to_vec();
                s.extend_from_slice(&name);
                s.extend_from_slice(&rest);
                let t = self.t1();
                t.bstrcpy(0, &rest);
                t.lstrcpy(r, &s);
                t.line_ptr = t.eol(r);
            }
            return;
        }
        if (k == STEMV_CODE || k == FONTBBOX1_CODE) && (at(&line, p) == b'[' || at(&line, p) == b'{') {
            p += 1;
        }
        if k == FONTBBOX1_CODE {
            for i in 0..4 {
                let (v, r) = self.t1_scan_num(p, true);
                let e = &mut self.fd_mut(fd).font_dim[k + i];
                e.val = d2i(v as f64);
                e.set = true;
                p = r;
            }
            return;
        }
        let (v, _) = self.t1_scan_num(p, false);
        let e = &mut self.fd_mut(fd).font_dim[k];
        e.val = d2i(v as f64);
        e.set = true;
    }

    /// writet1.c t1_scan_param
    fn t1_scan_param(&mut self) {
        let t = self.t1();
        if !t.scan || t.line[0] != b'/' {
            return;
        }
        if t.prefix(b"/lenIV") {
            let (v, _) = self.t1_scan_num(6, false);
            let l = d2i(v as f64) as i16;
            self.t1().len_iv = l;
            if l < 0 {
                self.pdftex_fail("negative value of lenIV is not supported");
            }
            return;
        }
        self.t1_scan_keys();
    }

    /// writet1.c t1_builtin_enc: read the encoding from the font file
    fn t1_builtin_enc(&mut self) -> GlyphNames {
        let mut glyph_names = new_glyph_names();
        if self.t1().suffix(b"def") {
            let line = self.t1().lstr(0);
            let w = scan_word(&line, 9, 255).map(|(w, _)| w);
            if let Some(w) = &w {
                self.t1().bstrcpy(0, w);
            }
            if w.as_deref() == Some(b"StandardEncoding") {
                self.t1().encoding = T1Enc::Standard;
                for i in 0..256 {
                    glyph_names[i] = std_glyph(i as i32);
                }
                return glyph_names;
            }
            let b = bs(c_str(&self.t1().buf, 0));
            self.pdftex_fail(&format!("cannot subset font (unknown predefined encoding `{}')", b));
        }
        self.t1().encoding = T1Enc::Builtin;
        if self.t1().prefix(b"/Encoding [") || self.t1().prefix(b"/Encoding[") {
            let mut line = self.t1().lstr(0);
            line.push(0);
            let mut r = c_strchr(&line, 0, b'[').unwrap() + 1;
            if at(&line, r) == b' ' {
                r += 1;
            }
            let mut counter = 0usize;
            loop {
                while at(&line, r) == b'/' {
                    r += 1;
                    let st = r;
                    while !matches!(at(&line, r), 32 | 10 | b']' | b'/') {
                        r += 1;
                    }
                    let name = line[st..r].to_vec();
                    self.t1().bstrcpy(0, &name);
                    if at(&line, r) == b' ' {
                        r += 1;
                    }
                    if counter > 255 {
                        self.pdftex_fail("encoding vector contains more than 256 names");
                    }
                    if name != NOTDEF {
                        glyph_names[counter] = Some(name);
                    }
                    counter += 1;
                }
                if at(&line, r) != 10 && at(&line, r) != b'%' {
                    if str_prefix(&line[r..], b"] def") || str_prefix(&line[r..], b"] readonly def") {
                        break;
                    }
                    let m = format!("a name or `] def' or `] readonly def' expected: `{}'", Self::removed_eol(&line));
                    self.pdftex_fail(&m);
                }
                self.t1_getline();
                line = self.t1().lstr(0);
                line.push(0);
                r = 0;
            }
        } else {
            let mut line = self.t1().lstr(0);
            line.push(0);
            let mut p = c_strchr(&line, 0, 10).unwrap_or(line.len() - 1);
            loop {
                if at(&line, p) == 10 {
                    self.t1_getline();
                    line = self.t1().lstr(0);
                    line.push(0);
                    p = 0;
                }
                let valid = |c: i64| (0..256).contains(&c);
                // dup <index> <glyph> put
                let m1 = scan_lit(&line, p, b"dup ").and_then(|i1| {
                    let (i, i2) = scan_int(&line, i1, 0)?;
                    let (w, _) = scan_word(&line, i2, 255)?;
                    Some((i as i32 as i64, w))
                });
                if let Some((_, w)) = &m1 {
                    self.t1().bstrcpy(0, w);
                }
                if let Some((i, w)) = m1.clone().filter(|(i, w)| w[0] == b'/' && valid(*i)) {
                    if &w[1..] != NOTDEF {
                        glyph_names[i as usize] = Some(w[1..].to_vec());
                    }
                    p = match c_strstr(&line, p, b" put") {
                        Some(q) => q,
                        None => self.pdftex_fail("invalid pfb, no put found in dup"),
                    };
                    p += 4;
                    if at(&line, p) == b' ' {
                        p += 1;
                    }
                    continue;
                }
                // dup dup <to> exch <from> get put
                let m2 = scan_lit(&line, p, b"dup dup ").and_then(|i1| {
                    let (b, i2) = scan_int(&line, i1, 0)?;
                    let i3 = scan_lit(&line, i2, b" exch ")?;
                    let (a, _) = scan_int(&line, i3, 0)?;
                    Some((b as i32 as i64, a as i32 as i64))
                });
                if let Some((b, a)) = m2.filter(|(b, a)| valid(*a) && valid(*b)) {
                    copy_glyph_names(&mut glyph_names, a as usize, b as usize);
                    p = match c_strstr(&line, p, b" get put") {
                        Some(q) => q,
                        None => self.pdftex_fail("invalid pfb, no get put found in dup dup"),
                    };
                    p += 8;
                    if at(&line, p) == b' ' {
                        p += 1;
                    }
                    continue;
                }
                // dup dup <from> <size> getinterval <to> exch putinterval
                let m3 = scan_lit(&line, p, b"dup dup ").and_then(|i1| {
                    let (a, i2) = scan_int(&line, i1, 0)?;
                    let (c, i3) = scan_int(&line, i2, 0)?;
                    let i4 = scan_lit(&line, i3, b" getinterval ")?;
                    let (b, _) = scan_int(&line, i4, 0)?;
                    Some((a as i32 as i64, c as i32 as i64, b as i32 as i64))
                });
                if let Some((a, c, b)) = m3.filter(|(a, c, b)| valid(*a) && valid(*b) && valid(*c)) {
                    for i in 0..c {
                        let (x, y) = ((a + i) as usize, (b + i) as usize);
                        if x < 256 && y < 256 {
                            copy_glyph_names(&mut glyph_names, x, y);
                        }
                    }
                    p = match c_strstr(&line, p, b" putinterval") {
                        Some(q) => q,
                        None => self.pdftex_fail("invalid pfb, no putinterval found in dup dup"),
                    };
                    p += 12;
                    if at(&line, p) == b' ' {
                        p += 1;
                    }
                    continue;
                }
                // def or readonly def
                if (p == 0 || line[p - 1] == b' ') && c_str(&line, p) == b"def\n" {
                    return glyph_names;
                }
                // skip an unrecognizable word
                while at(&line, p) != b' ' && at(&line, p) != 10 && at(&line, p) != 0 {
                    p += 1;
                }
                if at(&line, p) == b' ' {
                    p += 1;
                }
                if at(&line, p) == 0 {
                    p = line.len() - 1;
                    line[p] = 10;
                }
            }
        }
        glyph_names
    }

    /// writet1.c t1_open_fontfile
    fn t1_open_fontfile(&mut self, open_name_prefix: &str) -> bool {
        let fd = self.t1().fd_cur;
        let fm = self.fd(fd).fm;
        let ff_name = self.fm(fm).ff().to_vec();
        let is_tt = self.fm(fm).is_truetype();
        match self.check_ff_exist(&ff_name, is_tt) {
            Some(path) => {
                self.rt.pdfu.cur_file_name = Some(bs(&path));
                let f = self.font_xfopen(&path);
                self.t1().file = f;
                self.recorder_record_input(&path);
            }
            None => {
                self.set_cur_file_name(&ff_name);
                self.pdftex_fail("cannot open Type 1 font file for reading");
            }
        }
        self.t1_init_params(open_name_prefix);
        true
    }

    /// writet1.c t1_include
    fn t1_include(&mut self) {
        loop {
            self.t1_getline();
            self.t1_scan_param();
            self.t1_putline();
            if self.t1().in_eexec != 0 {
                break;
            }
        }
        self.t1_start_eexec();
        loop {
            self.t1_getline();
            self.t1_scan_param();
            self.t1_putline();
            if self.t1().charstrings() || self.t1().subrs() {
                break;
            }
        }
        self.t1().cs = true;
        loop {
            self.t1_getline();
            self.t1_putline();
            if self.t1().end_eexec() {
                break;
            }
        }
        self.t1_stop_eexec();
        self.t1().length3 = 0;
    }

    fn check_subr(&mut self, subr: i32) {
        if subr >= self.t1().subr_size || subr < 0 {
            self.pdftex_fail(&format!("Subrs array: entry index out of range ({})", subr));
        }
    }

    /// writet1.c check_cs_token_pair
    fn check_cs_token_pair(&self) -> Option<usize> {
        let t = &self.rt.fonts.t1;
        (0..CS_TOKEN_PAIRS.len()).find(|&i| str_prefix(&t.buf, CS_TOKEN_PAIRS[i].0) && str_suffix(&t.buf, t.buf_ptr, CS_TOKEN_PAIRS[i].1))
    }

    /// writet1.c cs_store
    fn cs_store(&mut self, is_subr: bool) {
        let t = self.t1();
        let mut p = 0;
        while t.line[p] != b' ' {
            p += 1;
        }
        let word = t.line[..p].to_vec();
        t.bstrcpy(0, &word);
        let slot;
        if is_subr {
            let (v, _) = self.t1_scan_num(p + 1, false);
            let subr = d2i(v as f64);
            self.check_subr(subr);
            slot = subr as usize;
        } else {
            let t = self.t1();
            slot = t.cs_ptr;
            t.cs_ptr += 1;
            if t.cs_ptr as i32 > t.cs_size {
                let n = t.cs_size;
                self.pdftex_fail(&format!("CharStrings dict: more entries than dict size ({})", n));
            }
            let name = if &word[1..] == NOTDEF { None } else { Some(word[1..].to_vec()) };
            self.t1().cs_tab.as_mut().unwrap()[slot].name = name;
        }
        let t = self.t1();
        let cslen = t.cslen as usize;
        let cs_start = t.cs_start;
        let mut data: Vec<u8> = t.line[cs_start - 4..cs_start + cslen].to_vec();
        let mut q = cs_start + cslen;
        while t.line[q] != 10 {
            data.push(t.line[q]);
            q += 1;
        }
        data.push(10);
        ensure_len(&mut t.buf, data.len() + 1);
        t.buf[..data.len()].copy_from_slice(&data);
        t.buf_ptr = data.len();
        if is_subr && t.cs_token_pair.is_none() {
            let tp = self.check_cs_token_pair();
            self.t1().cs_token_pair = tp;
        }
        let t = self.t1();
        let len = data.len() as u16;
        let cslen = t.cslen;
        let tab = if is_subr { t.subr_tab.as_mut().unwrap() } else { t.cs_tab.as_mut().unwrap() };
        let e = &mut tab[slot];
        e.len = len;
        e.cslen = cslen;
        e.data = data;
        e.valid = true;
    }

    /// writet1.c cc_init
    fn cc_init(&mut self) {
        let t = self.t1();
        if t.is_cc_init {
            return;
        }
        for e in t.cc_tab.iter_mut() {
            e.valid = false;
        }
        let mut set = |n: usize, b: bool, a: u8, c: bool| {
            t.cc_tab[n] = CcEntry { nargs: a, bottom: b, clear: c, valid: true };
        };
        set(CS_HSTEM, true, 2, true);
        set(CS_VSTEM, true, 2, true);
        set(CS_VMOVETO, true, 1, true);
        set(CS_RLINETO, true, 2, true);
        set(CS_HLINETO, true, 1, true);
        set(CS_VLINETO, true, 1, true);
        set(CS_RRCURVETO, true, 6, true);
        set(CS_CLOSEPATH, false, 0, true);
        set(CS_CALLSUBR, false, 1, false);
        set(CS_RETURN, false, 0, false);
        set(CS_HSBW, true, 2, true);
        set(CS_ENDCHAR, false, 0, true);
        set(CS_RMOVETO, true, 2, true);
        set(CS_HMOVETO, true, 1, true);
        set(CS_VHCURVETO, true, 4, true);
        set(CS_HVCURVETO, true, 4, true);
        set(CS_DOTSECTION, false, 0, true);
        set(CS_VSTEM3, true, 6, true);
        set(CS_HSTEM3, true, 6, true);
        set(CS_SEAC, true, 5, true);
        set(CS_SBW, true, 4, true);
        set(CS_DIV, false, 2, false);
        set(CS_CALLOTHERSUBR, false, 0, false);
        set(CS_POP, false, 0, false);
        set(CS_SETCURRENTPOINT, true, 2, true);
        t.is_cc_init = true;
    }

    /// writet1.c cs_fail
    fn cs_fail(&mut self, cs_name: Option<&[u8]>, subr: i32, msg: &str) -> ! {
        let mut m = msg.as_bytes().to_vec();
        m.truncate(SMALL_BUF_SIZE - 1);
        match cs_name {
            None => self.pdftex_fail(&format!("Subr ({}): {}", subr, bs(&m))),
            Some(n) => self.pdftex_fail(&format!("CharString (/{}): {}", bs(n), bs(&m))),
        }
    }

    /// writet1.c append_cs_return: fix a return-less subr by appending CS_RETURN
    fn append_cs_return(&mut self, subr: usize) {
        let t = self.t1();
        let e = t.subr_tab.as_ref().unwrap()[subr].clone();
        let mut cr: u16 = 4330;
        let mut plain: Vec<u8> = Vec::with_capacity(e.cslen as usize + 1);
        for i in 0..e.cslen as usize {
            plain.push(cdecrypt(e.data[4 + i], &mut cr));
        }
        plain.push(CS_RETURN as u8);
        ensure_len(&mut t.buf, plain.len() + 1);
        t.buf[..plain.len()].copy_from_slice(&plain);
        let mut nd: Vec<u8> = e.data[..4].to_vec();
        let mut cr: u16 = 4330;
        for &b in &plain {
            nd.push(cencrypt(b, &mut cr));
        }
        nd.extend_from_slice(&e.data[4 + e.cslen as usize..e.len as usize]);
        let s = &mut t.subr_tab.as_mut().unwrap()[subr];
        s.data = nd;
        s.len = s.len.wrapping_add(1);
        s.cslen = s.cslen.wrapping_add(1);
    }

    fn cc_get(&self, n: i32) -> i32 {
        let t = &self.rt.fonts.t1;
        let i = if n < 0 { t.stack_ptr as i32 + n } else { n };
        if (0..t.cc_stack.len() as i32).contains(&i) {
            t.cc_stack[i as usize]
        } else {
            0
        }
    }

    fn cc_pop(&mut self, n: i32) {
        let sp = self.t1().stack_ptr as i32;
        if sp < n {
            self.pdftex_fail(&format!("CharString: invalid access ({}) to stack ({} entries)", n, sp));
        }
        self.t1().stack_ptr = (sp - n) as usize;
    }

    fn cc_push(&mut self, v: i32) {
        let t = self.t1();
        if t.stack_ptr < t.cc_stack.len() {
            t.cc_stack[t.stack_ptr] = v;
            t.stack_ptr += 1;
        }
    }

    /// writet1.c cs_mark: mark a CharString (cs_name) or a Subr (cs_name None) as used
    fn cs_mark(&mut self, cs_name: Option<&[u8]>, subr: i32) {
        let is_subr = cs_name.is_none();
        let idx: usize;
        if is_subr {
            self.check_subr(subr);
            idx = subr as usize;
            if !self.t1().subr_tab.as_ref().unwrap()[idx].valid {
                return;
            }
        } else {
            let name = cs_name.unwrap();
            let t = self.t1();
            if t.cs_notdef.is_some() && name == NOTDEF {
                idx = t.cs_notdef.unwrap();
            } else {
                let tab = t.cs_tab.as_ref().unwrap();
                match (0..t.cs_ptr).find(|&i| gname(&tab[i].name) == name) {
                    None => {
                        self.pdftex_warn(&format!("glyph `{}' undefined", bs(name)));
                        return;
                    }
                    Some(i) => {
                        idx = i;
                        if tab[i].name.is_none() {
                            t.cs_notdef = Some(i);
                        }
                    }
                }
            }
        }
        let e = {
            let t = self.t1();
            let tab = if is_subr { t.subr_tab.as_mut().unwrap() } else { t.cs_tab.as_mut().unwrap() };
            let e = &mut tab[idx];
            if !e.valid || (e.used && !is_subr) {
                return;
            }
            e.used = true;
            e.clone()
        };
        let mut last_cmd = 0usize;
        let mut cr: u16 = 4330;
        let mut cs_len = e.cslen as i32;
        let mut data = 4usize;
        let getc = |data: &mut usize, cr: &mut u16| -> u8 {
            let b = e.data.get(*data).copied().unwrap_or(0);
            *data += 1;
            cdecrypt(b, cr)
        };
        let len_iv = self.t1().len_iv as i32;
        for _ in 0..len_iv {
            getc(&mut data, &mut cr);
            cs_len -= 1;
        }
        while cs_len > 0 {
            cs_len -= 1;
            let mut b = getc(&mut data, &mut cr) as usize;
            if b >= 32 {
                let a: i32 = if b <= 246 {
                    b as i32 - 139
                } else if b <= 250 {
                    cs_len -= 1;
                    ((b as i32 - 247) << 8) + 108 + getc(&mut data, &mut cr) as i32
                } else if b <= 254 {
                    cs_len -= 1;
                    -((b as i32 - 251) << 8) - 108 - getc(&mut data, &mut cr) as i32
                } else {
                    cs_len -= 4;
                    let mut a = (getc(&mut data, &mut cr) as i32 & 0xff) << 24;
                    a |= (getc(&mut data, &mut cr) as i32 & 0xff) << 16;
                    a |= (getc(&mut data, &mut cr) as i32 & 0xff) << 8;
                    a |= getc(&mut data, &mut cr) as i32 & 0xff;
                    a
                };
                self.cc_push(a);
            } else {
                if b == CS_ESCAPE {
                    b = getc(&mut data, &mut cr) as usize + CS_1BYTE_MAX;
                    cs_len -= 1;
                }
                if b >= CS_MAX {
                    self.cs_fail(cs_name, subr, &format!("command value out of range: {}", b));
                }
                let cc = self.t1().cc_tab[b];
                if !cc.valid {
                    self.cs_fail(cs_name, subr, &format!("command not valid: {}", b));
                }
                if cc.bottom {
                    let sp = self.t1().stack_ptr;
                    if sp < cc.nargs as usize {
                        self.cs_fail(cs_name, subr, &format!("less arguments on stack ({}) than required ({})", sp, cc.nargs));
                    } else if sp > cc.nargs as usize {
                        self.cs_fail(cs_name, subr, &format!("more arguments on stack ({}) than required ({})", sp, cc.nargs));
                    }
                }
                last_cmd = b;
                match b {
                    CS_CALLSUBR => {
                        let a1 = self.cc_get(-1);
                        self.cc_pop(1);
                        self.cs_mark(None, a1);
                        if !self.t1().subr_tab.as_ref().unwrap()[a1 as usize].valid {
                            self.cs_fail(cs_name, subr, &format!("cannot call subr ({})", a1));
                        }
                    }
                    CS_DIV => {
                        self.cc_pop(2);
                        self.cc_push(0);
                    }
                    CS_CALLOTHERSUBR => {
                        if self.cc_get(-1) == 3 {
                            self.t1().last_arg_other_subr3 = self.cc_get(-3);
                        }
                        let a1 = self.cc_get(-2) + 2;
                        self.cc_pop(a1);
                    }
                    CS_POP => {
                        let v = self.t1().last_arg_other_subr3;
                        self.cc_push(v);
                    }
                    CS_SEAC => {
                        let a1 = self.cc_get(3);
                        let a2 = self.cc_get(4);
                        self.t1().stack_ptr = 0;
                        let g1 = std_glyph(a1);
                        let g2 = std_glyph(a2);
                        self.cs_mark(Some(gname(&g1)), 0);
                        self.cs_mark(Some(gname(&g2)), 0);
                        let fd = self.t1().fd_cur;
                        if let Some(gl) = self.fd_mut(fd).gl_tree.as_mut() {
                            gl.insert(gname(&g1).to_vec());
                            gl.insert(gname(&g2).to_vec());
                        }
                    }
                    _ => {
                        if cc.clear {
                            self.t1().stack_ptr = 0;
                        }
                    }
                }
            }
        }
        if is_subr && last_cmd != CS_RETURN {
            self.pdftex_warn(&format!(
                "last command in subr `{}' is not a RETURN; I will add it now but please consider fixing the font",
                subr
            ));
            self.append_cs_return(idx);
        }
    }

    /// writet1.c t1_subset_ascii_part
    fn t1_subset_ascii_part(&mut self) {
        let fd = self.t1().fd_cur;
        self.t1_getline();
        while !self.t1().prefix(b"/Encoding") {
            self.t1_scan_param();
            let t = self.t1();
            let l = c_strlen(&t.line, 0);
            let ends_def = l >= 4 && str_prefix(&t.line[l - 4..], b"def");
            if !(t.prefix(b"/UniqueID") && ends_def) {
                self.t1_putline();
            }
            self.t1_getline();
        }
        let glyph_names = self.t1_builtin_enc();
        self.fd_mut(fd).builtin_glyph_names = Some(glyph_names.clone());
        let fm = self.fd(fd).fm;
        if self.fm(fm).is_subsetted() {
            if let Some(tx) = self.fd(fd).tx_tree.clone() {
                for p in tx {
                    let g = gname(&glyph_names[p as usize]).to_vec();
                    self.fd_mut(fd).gl_tree.as_mut().unwrap().insert(g);
                }
            }
            self.make_subset_tag(fd);
            let off = self.t1().fontname_offset as usize;
            let tag = self.fd(fd).subset_tag.clone().unwrap();
            let fb = &mut self.rt.pdfu.fb;
            ensure_len(fb, off + 6);
            fb[off..off + 6].copy_from_slice(&tag[..6]);
        }
        if self.t1().encoding == T1Enc::Standard {
            self.t1_puts(b"/Encoding StandardEncoding def\n");
        } else {
            self.t1_puts(b"/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\n");
            // create_t1_glyph_tree: name -> first code
            let mut gl_tree: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
            for (i, g) in glyph_names.iter().enumerate() {
                if let Some(g) = g {
                    gl_tree.entry(g.clone()).or_insert(i);
                }
            }
            let glyphs: Vec<Vec<u8>> = self.fd(fd).gl_tree.as_ref().unwrap().iter().cloned().collect();
            let mut j = 0;
            for g in glyphs {
                if let Some(&i) = gl_tree.get(&g) {
                    let mut s = format!("dup {} /", i).into_bytes();
                    s.extend_from_slice(gname(&glyph_names[i]));
                    s.extend_from_slice(b" put\n");
                    self.t1_puts(&s);
                    j += 1;
                }
            }
            if j == 0 {
                self.t1_puts(b"dup 0 /.notdef put\n");
            }
            self.t1_puts(b"readonly def\n");
        }
        loop {
            self.t1_getline();
            self.t1_scan_param();
            if !self.t1().prefix(b"/UniqueID") {
                self.t1_putline();
            }
            if self.t1().in_eexec != 0 {
                break;
            }
        }
    }

    /// writet1.c cs_init
    fn cs_init(&mut self) {
        let t = self.t1();
        t.cs_tab = None;
        t.cs_ptr = 0;
        t.cs_dict_start.clear();
        t.cs_dict_end.clear();
        t.cs_count = 0;
        t.cs_size = 0;
        t.cs_size_pos = 0;
        t.cs_token_pair = None;
        t.subr_tab = None;
        t.subr_array_start.clear();
        t.subr_array_end.clear();
        t.subr_max = 0;
        t.subr_size = 0;
        t.subr_size_pos = 0;
    }

    /// writet1.c t1_read_subrs
    fn t1_read_subrs(&mut self) {
        self.t1_getline();
        while !(self.t1().charstrings() || self.t1().subrs()) {
            self.t1_scan_param();
            if !self.t1().prefix(b"/UniqueID") {
                self.t1_putline();
            }
            self.t1_getline();
        }
        loop {
            // found:
            let t = self.t1();
            t.cs = true;
            t.scan = false;
            if !t.subrs() {
                return;
            }
            t.subr_size_pos = 7;
            let (v, _) = self.t1_scan_num(7, false);
            let size = d2i(v as f64);
            self.t1().subr_size = size;
            if size == 0 {
                while !self.t1().charstrings() {
                    self.t1_getline();
                }
                return;
            }
            let t = self.t1();
            t.subr_tab = Some(vec![CsEntry::default(); size.max(0) as usize]);
            t.subr_array_start = t.lstr(0);
            self.t1_getline();
            while self.t1().cslen != 0 {
                self.cs_store(true);
                self.t1_getline();
            }
            let t = self.t1();
            let tab = t.subr_tab.as_mut().unwrap();
            for e in tab.iter_mut().take(4) {
                e.used = true;
            }
            let mut s = 0usize;
            t.buf[0] = 0;
            let mut i = 0;
            while i < POST_SUBRS_SCAN {
                if self.t1().charstrings() {
                    break;
                }
                let t = self.t1();
                s += t.line_ptr;
                t.alloc_buf(s, T1_BUF_SIZE);
                let cur = c_strlen(&t.buf, 0);
                let l = t.lstr(0);
                t.bstrcpy(cur, &l);
                self.t1_getline();
                i += 1;
            }
            let t = self.t1();
            t.subr_array_end = c_str(&t.buf, 0).to_vec();
            if i == POST_SUBRS_SCAN {
                self.cs_init();
                let t = self.t1();
                t.cs = false;
                t.synthetic = true;
                while !(self.t1().charstrings() || self.t1().subrs()) {
                    self.t1_getline();
                }
                continue;
            }
            return;
        }
    }

    /// writet1.c t1_flush_cs
    fn t1_flush_cs(&mut self, is_subr: bool) {
        let t = self.t1();
        let (start_line, line_end, size_pos, tab, count) = if is_subr {
            let c = t.subr_max + 1;
            let mut tab = t.subr_tab.take().unwrap_or_default();
            tab.truncate(c.max(0) as usize);
            (std::mem::take(&mut t.subr_array_start), std::mem::take(&mut t.subr_array_end), t.subr_size_pos, tab, c)
        } else {
            let mut tab = t.cs_tab.take().unwrap_or_default();
            tab.truncate(t.cs_ptr);
            (std::mem::take(&mut t.cs_dict_start), std::mem::take(&mut t.cs_dict_end), t.cs_size_pos, tab, t.cs_count)
        };
        let mut l: Vec<u8> = start_line[..size_pos.min(start_line.len())].to_vec();
        let mut p = size_pos;
        while at(&start_line, p).is_ascii_digit() {
            p += 1;
        }
        l.extend_from_slice(format!("{}", count as u32).as_bytes());
        l.extend_from_slice(&start_line[p.min(start_line.len())..]);
        let t = self.t1();
        t.lstrcpy(0, &l);
        t.line_ptr = t.eol(0);
        self.t1_putline();
        let mut return_cs: Vec<u8> = Vec::new();
        if is_subr {
            let mut cr: u16 = 4330;
            let liv = self.t1().len_iv.max(0) as usize;
            for _ in 0..liv {
                return_cs.push(cencrypt(0, &mut cr));
            }
            return_cs.push(cencrypt(CS_RETURN as u8, &mut cr));
        }
        let tp = self.t1().cs_token_pair;
        for (i, e) in tab.iter().enumerate() {
            if e.used {
                let mut s = if is_subr {
                    format!("dup {} {}", i, e.cslen).into_bytes()
                } else {
                    let mut s = b"/".to_vec();
                    s.extend_from_slice(gname(&e.name));
                    s.extend_from_slice(format!(" {}", e.cslen).as_bytes());
                    s
                };
                let n = s.len();
                s.extend_from_slice(&e.data[..e.len as usize]);
                let t = self.t1();
                ensure_len(&mut t.line, s.len() + 2);
                t.line[..s.len()].copy_from_slice(&s);
                t.line[n] = if n < s.len() { s[n] } else { 0 };
                t.line_ptr = s.len();
                self.t1_putline();
            } else if is_subr {
                let (open, close) = CS_TOKEN_PAIRS[tp.unwrap()];
                let mut s = format!("dup {} {}", i, return_cs.len()).into_bytes();
                s.extend_from_slice(open);
                s.push(b' ');
                s.extend_from_slice(&return_cs);
                let t = self.t1();
                ensure_len(&mut t.line, s.len() + 2);
                t.line[..s.len()].copy_from_slice(&s);
                t.line_ptr = s.len();
                self.t1_putline();
                let mut s = b" ".to_vec();
                s.extend_from_slice(close);
                let t = self.t1();
                t.lstrcpy(0, &s);
                t.line_ptr = t.eol(0);
                self.t1_putline();
            }
        }
        let t = self.t1();
        t.lstrcpy(0, &line_end);
        t.line_ptr = t.eol(0);
        self.t1_putline();
    }

    /// writet1.c t1_mark_glyphs
    fn t1_mark_glyphs(&mut self) {
        let fd = self.t1().fd_cur;
        let t = self.t1();
        if t.synthetic || self.rt.fonts.wf.fd[fd].all_glyphs {
            let t = self.t1();
            if let Some(tab) = t.cs_tab.as_mut() {
                for e in tab.iter_mut().take(t.cs_ptr) {
                    if e.valid {
                        e.used = true;
                    }
                }
            }
            if let Some(tab) = t.subr_tab.as_mut() {
                for e in tab.iter_mut() {
                    if e.valid {
                        e.used = true;
                    }
                }
                t.subr_max = t.subr_size - 1;
            }
            return;
        }
        self.cs_mark(Some(NOTDEF), 0);
        // libavl traversal tolerates insertions (seac components): visit successors in order
        let mut last: Option<Vec<u8>> = None;
        loop {
            let next = {
                let gl = self.fd(fd).gl_tree.as_ref().unwrap();
                match &last {
                    None => gl.iter().next().cloned(),
                    Some(l) => gl.range::<Vec<u8>, _>((Bound::Excluded(l), Bound::Unbounded)).next().cloned(),
                }
            };
            match next {
                None => break,
                Some(g) => {
                    self.cs_mark(Some(&g), 0);
                    last = Some(g);
                }
            }
        }
        let t = self.t1();
        if let Some(tab) = t.subr_tab.as_ref() {
            t.subr_max = -1;
            for (i, e) in tab.iter().enumerate() {
                if e.used && i as i32 > t.subr_max {
                    t.subr_max = i as i32;
                }
            }
        }
    }

    /// writet1.c t1_check_unusual_charstring
    fn t1_check_unusual_charstring(&mut self) {
        let t = self.t1();
        let p = c_strstr(&t.line, 0, CHARSTRINGNAME).unwrap() + CHARSTRINGNAME.len();
        if scan_int(&t.line, p, 0).is_none() {
            let mut b = t.lstr(0);
            if let Some(last) = b.last_mut() {
                *last = b' ';
            }
            t.bstrcpy(0, &b);
            self.t1_getline();
            let t = self.t1();
            let l = t.lstr(0);
            let mut b = c_str(&t.buf, 0).to_vec();
            b.extend_from_slice(&l);
            t.bstrcpy(0, &b);
            t.lstrcpy(0, &b);
            t.line_ptr = t.eol(0);
        }
    }

    /// writet1.c t1_subset_charstrings
    fn t1_subset_charstrings(&mut self) {
        self.t1_check_unusual_charstring();
        let t = self.t1();
        t.cs_size_pos = c_strstr(&t.line, 0, CHARSTRINGNAME).unwrap() + CHARSTRINGNAME.len() + 1;
        let pos = t.cs_size_pos;
        let (v, _) = self.t1_scan_num(pos, false);
        let t = self.t1();
        t.cs_size = d2i(v as f64);
        t.cs_tab = Some(vec![CsEntry::default(); t.cs_size.max(0) as usize]);
        t.cs_ptr = 0;
        t.cs_notdef = None;
        t.cs_dict_start = t.lstr(0);
        self.t1_getline();
        while self.t1().cslen != 0 {
            self.cs_store(false);
            self.t1_getline();
        }
        let t = self.t1();
        t.cs_dict_end = t.lstr(0);
        self.t1_mark_glyphs();
        if self.t1().subr_tab.is_some() {
            if self.t1().cs_token_pair.is_none() {
                self.pdftex_fail("This Type 1 font uses mismatched subroutine begin/end token pairs.");
            }
            self.t1_flush_cs(true);
        }
        let t = self.t1();
        t.cs_count = t.cs_tab.as_ref().unwrap()[..t.cs_ptr].iter().filter(|e| e.used).count() as i32;
        self.t1_flush_cs(false);
    }

    /// writet1.c t1_subset_end
    fn t1_subset_end(&mut self) {
        if self.t1().synthetic {
            while c_strstr(&self.t1().line, 0, b"definefont").is_none() {
                self.t1_getline();
                self.t1_putline();
            }
            while !self.t1().end_eexec() {
                self.t1_getline();
            }
            self.t1_putline();
        } else {
            while !self.t1().end_eexec() {
                self.t1_getline();
                self.t1_putline();
            }
        }
        self.t1_stop_eexec();
        self.t1().length3 = 0;
    }

    /// writet1.c writet1
    pub fn writet1(&mut self, fd: usize) {
        self.t1().fd_cur = fd;
        self.t1().save_offset = 0;
        let fm = self.fd(fd).fm;
        if !self.fm(fm).is_subsetted() {
            let ok = self.t1_open_fontfile("<<");
            self.fd_mut(fd).ff_found = ok;
            if !ok {
                return;
            }
            self.t1_include();
            self.t1_close_font_file(">>");
            return;
        }
        let ok = self.t1_open_fontfile("<");
        self.fd_mut(fd).ff_found = ok;
        if !ok {
            return;
        }
        self.t1_subset_ascii_part();
        self.t1_start_eexec();
        self.cc_init();
        self.cs_init();
        self.t1_read_subrs();
        self.t1_subset_charstrings();
        self.t1_subset_end();
        self.t1_close_font_file(">");
    }

    /// writet1.c t1_free
    pub fn t1_free(&mut self) {
        let t = self.t1();
        t.line = Vec::new();
        t.buf = Vec::new();
        t.line_limit = 0;
        t.buf_limit = 0;
    }
}

/// writet1.c copy_glyph_names
fn copy_glyph_names(glyph_names: &mut GlyphNames, a: usize, b: usize) {
    if glyph_names[b].is_some() {
        glyph_names[b] = None;
    }
    if glyph_names[a].is_some() {
        glyph_names[b] = glyph_names[a].clone();
    }
}
