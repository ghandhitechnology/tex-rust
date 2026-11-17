//! pdfTeX's font subsystem (texk/web2c/pdftexdir: mapfile.c, writefont.c, writet1.c,
//! writet3.c, pkin.c, writettf.c, writeenc.c, subfont.c, tounicode.c, vfpacket.c,
//! macnames.c, the font parts of avlstuff.c and make_subset_tag from utils.c), ported to
//! Rust. One module per C file; each Rust function names the C function it mirrors.
//!
//! Data model. The C code links `fm_entry`/`fd_entry`/`fo_entry`/`fe_entry` structures by
//! pointer and keeps them in libavl trees. Here every structure lives in an arena
//! (`Vec`, never shrunk, so indices are stable) and is referred to by index; the AVL trees
//! become `BTreeMap`/`BTreeSet`s whose keys compare exactly like the C comparison
//! functions (strcmp = byte order, then the integer fields), so in-order traversals visit
//! entries in the same order. `avl_probe` ("insert unless an equal key exists") is
//! `entry().or_insert()`.
//!
//! Glyph names: the C code distinguishes the static `notdef` pointer from strings that
//! merely contain ".notdef"; `GlyphName = Option<Vec<u8>>` with `None` = the `notdef`
//! pointer keeps that distinction.

pub mod macnames;
pub mod mapcache;
pub mod mapfile;
pub mod pkin;
pub mod subfont;
pub mod tounicode;
pub mod utils;
pub mod vfpacket;
pub mod writeenc;
pub mod writefont;
pub mod writet1;
pub mod writet3;
pub mod writettf;

use crate::gen::globals::Globals;
use crate::rt::*;

pub type GlyphName = Option<Vec<u8>>;
pub type GlyphNames = Vec<GlyphName>;

pub const NOTDEF: &[u8] = b".notdef";

/// `glyph_names[i]` as the C string it points to (`notdef` for the static pointer).
pub fn gname(g: &GlyphName) -> &[u8] {
    match g {
        Some(s) => s,
        None => NOTDEF,
    }
}

pub fn new_glyph_names() -> GlyphNames {
    vec![None; 256]
}

// ---------------------------------------------------------------------------- ptexmac.h
pub const SMALL_BUF_SIZE: usize = 256;
pub const PRINTF_BUF_SIZE: usize = 1024;

pub const ASCENT_CODE: usize = 0;
pub const CAPHEIGHT_CODE: usize = 1;
pub const DESCENT_CODE: usize = 2;
pub const ITALIC_ANGLE_CODE: usize = 3;
pub const STEMV_CODE: usize = 4;
pub const XHEIGHT_CODE: usize = 5;
pub const FONTBBOX1_CODE: usize = 6;
pub const FONTBBOX2_CODE: usize = 7;
pub const FONTBBOX3_CODE: usize = 8;
pub const FONTBBOX4_CODE: usize = 9;
pub const FONTNAME_CODE: usize = 10;
pub const GEN_KEY_NUM: usize = XHEIGHT_CODE + 1;
pub const INT_KEYS_NUM: usize = FONTBBOX4_CODE + 1;
pub const FONT_KEYS_NUM: usize = FONTNAME_CODE + 1;

pub const F_INCLUDED: u16 = 0x01;
pub const F_SUBSETTED: u16 = 0x02;
pub const F_STDT1FONT: u16 = 0x04;
pub const F_SUBFONT: u16 = 0x08;
pub const F_TYPE1: u16 = 0x10;
pub const F_TRUETYPE: u16 = 0x20;
pub const F_OTF: u16 = 0x40;
pub const F_PK: u16 = 0x80;

pub const LINK_TFM: u16 = 0x01;
pub const LINK_PS: u16 = 0x02;

pub const FD_FLAGS_NOT_SET_IN_MAPLINE: i32 = -1;
pub const FD_FLAGS_DEFAULT_EMBED: i32 = 4;
pub const FD_FLAGS_DEFAULT_NON_EMBED: i32 = 0x22;

/// ptexlib.h font_key[]: (pdfname, t1name)
pub const FONT_KEY: [(&str, &str); FONT_KEYS_NUM] = [
    ("Ascent", "Ascender"),
    ("CapHeight", "CapHeight"),
    ("Descent", "Descender"),
    ("ItalicAngle", "ItalicAngle"),
    ("StemV", "StdVW"),
    ("XHeight", "XHeight"),
    ("FontBBox", "FontBBox"),
    ("", ""),
    ("", ""),
    ("", ""),
    ("FontName", "FontName"),
];

// ---------------------------------------------------------------------------- ptexlib.h types
/// ptexlib.h fm_entry
#[derive(Clone, Default, Debug)]
pub struct FmEntry {
    pub tfm_name: Option<Vec<u8>>,
    pub ps_name: Option<Vec<u8>>,
    pub fd_flags: i32,
    pub slant: i32,
    pub extend: i32,
    pub encname: Option<Vec<u8>>,
    pub ff_name: Option<Vec<u8>>,
    pub type_: u16,
    pub pid: i16,
    pub eid: i16,
    /// index into SfdState::subfonts
    pub subfont: Option<usize>,
    pub links: u16,
    pub in_use: bool,
}

impl FmEntry {
    pub fn is_included(&self) -> bool {
        self.type_ & F_INCLUDED != 0
    }
    pub fn is_subsetted(&self) -> bool {
        self.type_ & F_SUBSETTED != 0
    }
    pub fn is_std_t1font(&self) -> bool {
        self.type_ & F_STDT1FONT != 0
    }
    pub fn is_subfont(&self) -> bool {
        self.type_ & F_SUBFONT != 0
    }
    pub fn is_type1(&self) -> bool {
        self.type_ & F_TYPE1 != 0
    }
    pub fn is_truetype(&self) -> bool {
        self.type_ & F_TRUETYPE != 0
    }
    pub fn is_opentype(&self) -> bool {
        self.type_ & F_OTF != 0
    }
    pub fn is_pk(&self) -> bool {
        self.type_ & F_PK != 0
    }
    pub fn is_reencoded(&self) -> bool {
        self.encname.is_some()
    }
    pub fn is_fontfile(&self) -> bool {
        self.ff_name.is_some()
    }
    pub fn is_t1fontfile(&self) -> bool {
        self.is_fontfile() && self.is_type1()
    }
    pub fn is_builtin(&self) -> bool {
        !self.is_fontfile()
    }
    pub fn has_tfmlink(&self) -> bool {
        self.links & LINK_TFM != 0
    }
    pub fn has_pslink(&self) -> bool {
        self.links & LINK_PS != 0
    }
    pub fn tfm(&self) -> &[u8] {
        self.tfm_name.as_deref().unwrap_or(b"")
    }
    pub fn ff(&self) -> &[u8] {
        self.ff_name.as_deref().unwrap_or(b"")
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct IntParm {
    pub val: i32,
    pub set: bool,
}

/// ptexlib.h fd_entry
#[derive(Default)]
pub struct FdEntry {
    pub fd_objnum: i32,
    pub fontname: Option<Vec<u8>>,
    pub subset_tag: Option<Vec<u8>>,
    pub ff_found: bool,
    pub ff_objnum: i32,
    pub fn_objnum: i32,
    pub all_glyphs: bool,
    pub write_ttf_glyph_names: bool,
    pub font_dim: [IntParm; FONT_KEYS_NUM],
    /// index into EncState::fe
    pub fe: Option<usize>,
    pub builtin_glyph_names: Option<GlyphNames>,
    /// index into MapState::fm
    pub fm: usize,
    pub tx_tree: Option<std::collections::BTreeSet<i32>>,
    pub gl_tree: Option<std::collections::BTreeSet<Vec<u8>>>,
}

/// ptexlib.h fe_entry
pub struct FeEntry {
    pub fe_objnum: i32,
    pub name: Vec<u8>,
    pub glyph_names: GlyphNames,
    pub tx_tree: Option<std::collections::BTreeSet<i32>>,
}

/// ptexlib.h fo_entry (cw_entry folded in)
pub struct FoEntry {
    pub fo_objnum: i32,
    pub tex_font: i32,
    pub fm: usize,
    pub fd: Option<usize>,
    pub fe: Option<usize>,
    pub cw_objnum: i32,
    pub cw_width: Option<Vec<i32>>,
    pub first_char: i32,
    pub last_char: i32,
    pub tounicode_objnum: i32,
}

/// All static state of the font C files.
#[derive(Default)]
pub struct FontState {
    pub map: mapfile::MapState,
    pub wf: writefont::WriteFontState,
    pub enc: writeenc::EncState,
    pub t1: writet1::T1State,
    pub t3: writet3::T3State,
    pub pk: pkin::PkState,
    pub ttf: writettf::TtfState,
    pub sfd: subfont::SfdState,
    pub tu: tounicode::ToUnicodeState,
    pub vf: vfpacket::VfState,
    pub st_tree: std::collections::BTreeSet<Vec<u8>>,
}

// ---------------------------------------------------------------------------- C FILE emulation
/// A `FILE*` opened for reading in binary mode: getc/ungetc/feof/fseek/ftell semantics.
#[derive(Default)]
pub struct CFile {
    pub data: Vec<u8>,
    pub pos: usize,
    pub eof: bool,
}

impl CFile {
    pub fn new(data: Vec<u8>) -> CFile {
        CFile { data, pos: 0, eof: false }
    }
    #[inline]
    pub fn getc(&mut self) -> i32 {
        if self.pos < self.data.len() {
            let c = self.data[self.pos];
            self.pos += 1;
            c as i32
        } else {
            self.eof = true;
            -1
        }
    }
    pub fn ungetc(&mut self, c: i32) {
        if c < 0 {
            return;
        }
        self.eof = false;
        if self.pos > 0 {
            self.pos -= 1;
        }
    }
    #[inline]
    pub fn feof(&self) -> bool {
        self.eof
    }
    pub fn seek(&mut self, off: i64) {
        self.pos = off.max(0) as usize;
        self.eof = false;
    }
    pub fn tell(&self) -> i64 {
        self.pos as i64
    }
}

impl Globals {
    /// openclose.c open_input(&f, fmt, FOPEN_RBIN_MODE) for the font code: the name is in
    /// nameoffile; on success nameoffile holds the full path.
    pub fn font_open_input(&mut self, fmt: i32) -> Option<CFile> {
        let (f, ok) = self.open_input(fmt);
        if !ok {
            return None;
        }
        let data = match self.rt.files.in_data(f) {
            Some((d, _, _)) => std::mem::take(d),
            None => Vec::new(),
        };
        self.rt.files.close(f);
        Some(CFile::new(data))
    }

    /// xfopen(name, FOPEN_RBIN_MODE) for a name kpathsea returned
    pub fn font_xfopen(&mut self, name: &[u8]) -> CFile {
        let p = self.os_path(name);
        match std::fs::read(p) {
            Ok(d) => CFile::new(d),
            Err(_) => texmfmp::fatal_perror(&bytes_to_string(name)),
        }
    }

    /// ptexmac.h set_cur_file_name(s): cur_file_name = s; packfilename(maketexstring(s), "", "")
    pub fn set_cur_file_name(&mut self, s: &[u8]) {
        self.rt.pdfu.cur_file_name = Some(bytes_to_string(s));
        let n = self.maketexstring_u(s);
        let e = self.getnullstr();
        self.packfilename(n, e, e);
    }

    pub fn set_cur_file_name_none(&mut self) {
        self.rt.pdfu.cur_file_name = None;
    }

    /// cur_file_name = (char *) nameoffile + 1
    pub fn cur_file_name_from_nameoffile(&mut self) {
        let n = self.name_of_file_str();
        self.rt.pdfu.cur_file_name = Some(n);
    }

    /// utils.c tex_printf with a byte string argument (vsnprintf into PRINTF_BUF_SIZE).
    pub fn tex_printf_b(&mut self, s: &[u8]) {
        let b = &s[..s.len().min(PRINTF_BUF_SIZE - 1)];
        let b = b.to_vec();
        let st = self.maketexstring_u(&b);
        self.print(st);
        let l = self.rt.pdfu.last_tex_string;
        self.flushstr(l);
        self.rt.files.flush(FileRef::STDOUT);
    }

    /// utils.c pdf_printf with bytes (vsnprintf into PRINTF_BUF_SIZE, then pdf_puts).
    pub fn pdf_printf_b(&mut self, s: &[u8]) {
        let b = &s[..s.len().min(PRINTF_BUF_SIZE - 1)];
        let b = b.to_vec();
        self.pdf_puts(&b);
    }

    /// pdftex_fail for check_buf(): "buffer overflow at file %s, line %d"
    pub fn check_buf_fail(&mut self, file: &str, line: i32) -> ! {
        self.pdftex_fail(&format!("buffer overflow at file {}, line {}", file, line))
    }
}

/// `s` as text for messages (%s of a char*).
pub fn bs(s: &[u8]) -> String {
    bytes_to_string(s)
}

// ---------------------------------------------------------------------------- C string helpers
/// strlen of the NUL-terminated string starting at buf[i] (end of buffer counts as NUL).
pub fn c_strlen(buf: &[u8], i: usize) -> usize {
    let mut n = 0;
    while i + n < buf.len() && buf[i + n] != 0 {
        n += 1;
    }
    n
}

/// The C string at buf[i..] (without the NUL).
pub fn c_str(buf: &[u8], i: usize) -> &[u8] {
    &buf[i.min(buf.len())..i + c_strlen(buf, i)]
}

/// strchr: index of the first `c` in the C string at buf[i..]
pub fn c_strchr(buf: &[u8], i: usize, c: u8) -> Option<usize> {
    let s = c_str(buf, i);
    s.iter().position(|&x| x == c).map(|p| i + p)
}

/// strstr: index of the first occurrence of `pat` in the C string at buf[i..]
pub fn c_strstr(buf: &[u8], i: usize, pat: &[u8]) -> Option<usize> {
    let s = c_str(buf, i);
    if pat.is_empty() {
        return Some(i);
    }
    s.windows(pat.len()).position(|w| w == pat).map(|p| i + p)
}

/// str_prefix(s1, s2): strncmp(s1, s2, strlen(s2)) == 0
pub fn str_prefix(s: &[u8], pre: &[u8]) -> bool {
    let n = pre.len();
    let a = &s[..s.len().min(n)];
    let a_len = a.iter().position(|&c| c == 0).unwrap_or(a.len());
    &a[..a_len] == pre
}

pub fn is_c_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

pub fn strcasecmp_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

/// ptexmac.h append_char_to_buf(c, p, buf, buf_size); false = check_buf failure.
pub fn append_char_to_buf(c: &mut i32, p: &mut Vec<u8>, size: usize) -> bool {
    if *c == 9 {
        *c = 32;
    }
    if *c == 13 || *c == -1 {
        *c = 10;
    }
    if *c != 32 || (!p.is_empty() && *p.last().unwrap() != 32) {
        if p.len() + 1 > size {
            return false;
        }
        p.push(*c as u8);
    }
    true
}

/// ptexmac.h append_eol(p, buf, buf_size) (without storing the NUL); false = overflow.
pub fn append_eol(p: &mut Vec<u8>, size: usize) -> bool {
    if p.len() + 2 > size {
        return false;
    }
    if p.len() > 1 && *p.last().unwrap() != 10 {
        p.push(10);
    }
    let n = p.len();
    if n > 2 && p[n - 2] == 32 {
        p[n - 2] = 10;
        p.pop();
    }
    true
}

/// Ensure buf has at least n bytes (alloc_array growth; contents kept, new bytes zero).
pub fn ensure_len(buf: &mut Vec<u8>, n: usize) {
    if buf.len() < n {
        buf.resize(n, 0);
    }
}

// ---------------------------------------------------------------------------- sscanf
/// Byte at s[i], 0 past the end (C strings).
#[inline]
pub fn at(s: &[u8], i: usize) -> u8 {
    if i < s.len() {
        s[i]
    } else {
        0
    }
}

/// Skip C white space from s[i].
pub fn skip_ws(s: &[u8], mut i: usize) -> usize {
    while i < s.len() && s[i] != 0 && is_c_space(s[i]) {
        i += 1;
    }
    i
}

/// The extent of a floating-point number at s[i..] as scanf's %f/%g reads it (no leading
/// white space skipping): returns the end index, or None if no number.
fn float_extent(s: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    if at(s, i) == b'+' || at(s, i) == b'-' {
        i += 1;
    }
    let lower: Vec<u8> = (0..8).map(|k| at(s, i + k).to_ascii_lowercase()).collect();
    if lower.starts_with(b"infinity") {
        return Some(i + 8);
    }
    if lower.starts_with(b"inf") || lower.starts_with(b"nan") {
        return Some(i + 3);
    }
    let mut saw = false;
    if at(s, i) == b'0' && (at(s, i + 1) == b'x' || at(s, i + 1) == b'X') && at(s, i + 2).is_ascii_hexdigit() {
        i += 2;
        while at(s, i).is_ascii_hexdigit() {
            i += 1;
        }
        if at(s, i) == b'.' {
            i += 1;
            while at(s, i).is_ascii_hexdigit() {
                i += 1;
            }
        }
        if (at(s, i) == b'p' || at(s, i) == b'P') && {
            let j = if at(s, i + 1) == b'+' || at(s, i + 1) == b'-' { i + 2 } else { i + 1 };
            at(s, j).is_ascii_digit()
        } {
            i += 1;
            if at(s, i) == b'+' || at(s, i) == b'-' {
                i += 1;
            }
            while at(s, i).is_ascii_digit() {
                i += 1;
            }
        }
        return Some(i);
    }
    while at(s, i).is_ascii_digit() {
        i += 1;
        saw = true;
    }
    if at(s, i) == b'.' {
        i += 1;
        while at(s, i).is_ascii_digit() {
            i += 1;
            saw = true;
        }
    }
    if !saw {
        return None;
    }
    if at(s, i) == b'e' || at(s, i) == b'E' {
        let mut j = i + 1;
        if at(s, j) == b'+' || at(s, j) == b'-' {
            j += 1;
        }
        if at(s, j).is_ascii_digit() {
            while at(s, j).is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    Some(i)
}

fn parse_float_text(t: &[u8]) -> f64 {
    let txt = String::from_utf8_lossy(t).to_ascii_lowercase();
    let (neg, body) = match txt.strip_prefix('-') {
        Some(b) => (true, b.to_string()),
        None => (false, txt.strip_prefix('+').unwrap_or(&txt).to_string()),
    };
    let v = if body.starts_with("inf") {
        f64::INFINITY
    } else if body.starts_with("nan") {
        f64::NAN
    } else if let Some(h) = body.strip_prefix("0x") {
        parse_hex_float(h)
    } else {
        let mut b = body.clone();
        if b.starts_with('.') {
            b.insert(0, '0');
        }
        if let Some(p) = b.find(|c| c == 'e') {
            if b[..p].ends_with('.') {
                b.insert(p, '0');
            }
        } else if b.ends_with('.') {
            b.push('0');
        }
        b.parse::<f64>().unwrap_or(0.0)
    };
    if neg {
        -v
    } else {
        v
    }
}

fn parse_float_text_f32(t: &[u8]) -> f32 {
    let txt = String::from_utf8_lossy(t).to_ascii_lowercase();
    if txt.contains("0x") || txt.contains("inf") || txt.contains("nan") {
        return parse_float_text(t) as f32;
    }
    let (neg, body) = match txt.strip_prefix('-') {
        Some(b) => (true, b.to_string()),
        None => (false, txt.strip_prefix('+').unwrap_or(&txt).to_string()),
    };
    let mut b = body;
    if b.starts_with('.') {
        b.insert(0, '0');
    }
    if let Some(p) = b.find('e') {
        if b[..p].ends_with('.') {
            b.insert(p, '0');
        }
    } else if b.ends_with('.') {
        b.push('0');
    }
    let v = b.parse::<f32>().unwrap_or(0.0);
    if neg {
        -v
    } else {
        v
    }
}

fn parse_hex_float(h: &str) -> f64 {
    let (mant, exp) = match h.find('p') {
        Some(p) => (&h[..p], h[p + 1..].parse::<i32>().unwrap_or(0)),
        None => (h, 0),
    };
    let (ip, fp) = match mant.find('.') {
        Some(p) => (&mant[..p], &mant[p + 1..]),
        None => (mant, ""),
    };
    let mut v = 0f64;
    for c in ip.chars() {
        v = v * 16.0 + c.to_digit(16).unwrap_or(0) as f64;
    }
    let mut scale = 1.0 / 16.0;
    for c in fp.chars() {
        v += c.to_digit(16).unwrap_or(0) as f64 * scale;
        scale /= 16.0;
    }
    v * 2f64.powi(exp)
}

/// sscanf %f / %g into a float: skips white space; returns (value, end index).
pub fn scan_float(s: &[u8], i: usize) -> Option<(f32, usize)> {
    let st = skip_ws(s, i);
    let e = float_extent(s, st)?;
    Some((parse_float_text_f32(&s[st..e]), e))
}

/// sscanf %i (base from prefix) / %d (base 10) / %x (base 16) into a C int.
pub fn scan_int(s: &[u8], i: usize, base: u32) -> Option<(i64, usize)> {
    let mut j = skip_ws(s, i);
    let mut neg = false;
    if at(s, j) == b'+' || at(s, j) == b'-' {
        neg = at(s, j) == b'-';
        j += 1;
    }
    let mut b = base;
    if b == 0 {
        if at(s, j) == b'0' {
            if (at(s, j + 1) == b'x' || at(s, j + 1) == b'X') && at(s, j + 2).is_ascii_hexdigit() {
                b = 16;
                j += 2;
            } else {
                b = 8;
            }
        } else {
            b = 10;
        }
    } else if b == 16 && at(s, j) == b'0' && (at(s, j + 1) == b'x' || at(s, j + 1) == b'X') && at(s, j + 2).is_ascii_hexdigit() {
        j += 2;
    }
    let start = j;
    let mut v: i64 = 0;
    while (at(s, j) as char).to_digit(b).is_some() && at(s, j) != 0 {
        v = v.wrapping_mul(b as i64).wrapping_add((at(s, j) as char).to_digit(b).unwrap() as i64);
        j += 1;
    }
    if j == start {
        return None;
    }
    Some((if neg { v.wrapping_neg() } else { v }, j))
}

/// sscanf %s into a buffer: skips white space, reads up to the next white space.
pub fn scan_word(s: &[u8], i: usize, max: usize) -> Option<(Vec<u8>, usize)> {
    let st = skip_ws(s, i);
    let mut j = st;
    while j < s.len() && s[j] != 0 && !is_c_space(s[j]) && j - st < max {
        j += 1;
    }
    if j == st {
        return None;
    }
    Some((s[st..j].to_vec(), j))
}

/// Match a literal in a scanf format: white space in the literal skips any white space.
pub fn scan_lit(s: &[u8], mut i: usize, lit: &[u8]) -> Option<usize> {
    for &c in lit {
        if is_c_space(c) {
            i = skip_ws(s, i);
        } else if at(s, i) == c {
            i += 1;
        } else {
            return None;
        }
    }
    Some(i)
}

/// atoi
pub fn atoi(s: &[u8], i: usize) -> i32 {
    scan_int(s, i, 10).map(|(v, _)| v as i32).unwrap_or(0)
}

// ---------------------------------------------------------------------------- printf
/// C printf "%g" (precision 6) of a double.
pub fn fmt_g(v: f64) -> String {
    fmt_g_prec(v, 6)
}

pub fn fmt_g_prec(v: f64, p: usize) -> String {
    let p = p.max(1);
    if v.is_nan() {
        return if v.is_sign_negative() { "-nan(ind)".into() } else { "nan".into() };
    }
    if v.is_infinite() {
        return if v < 0.0 { "-inf".into() } else { "inf".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let e_repr = format!("{:.*e}", p - 1, v);
    let (mant, exp) = e_repr.split_once('e').unwrap();
    let x: i32 = exp.parse().unwrap();
    if x < -4 || x >= p as i32 {
        let m = strip_zeros(mant);
        format!("{}e{}{:02}", m, if x < 0 { '-' } else { '+' }, x.abs())
    } else {
        let s = format!("{:.*}", (p as i32 - 1 - x) as usize, v);
        strip_zeros(&s)
    }
}

fn strip_zeros(s: &str) -> String {
    if s.contains('.') {
        let t = s.trim_end_matches('0');
        t.trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

/// C (int) conversion of a double (truncation; out of range gives INT_MIN like MSVC/x86).
pub fn d2i(v: f64) -> i32 {
    if v.is_nan() || v >= 2147483648.0 || v < -2147483648.0 {
        i32::MIN
    } else {
        v as i32
    }
}
