//! The parts of xpdf's `fofi` library that `Gfx8BitFont` needs to build
//! a font's encoding: `FoFiIdentifier::identifyStream`, and the font
//! name / built-in encoding readers of `FoFiType1` and `FoFiType1C`.

use crate::doc::atof;
use crate::fofi_tables::*;

/// Mirrors `enum FoFiIdentifierType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoFiIdentifierType {
    Type1PFA,
    Type1PFB,
    CFF8Bit,
    CFFCID,
    TrueType,
    TrueTypeCollection,
    OpenTypeCFF8Bit,
    OpenTypeCFFCID,
    Dfont,
    Unknown,
    Error,
}

/// Random-access reader with the semantics of FoFiIdentifier's
/// `StreamReader` (reads are bounded by the data; all accesses made by
/// `identify()` are at non-decreasing positions, so the sliding buffer
/// of the C++ reader never makes a difference).
struct Reader<'a> {
    d: &'a [u8],
}

impl<'a> Reader<'a> {
    fn get_byte(&self, pos: i64) -> i32 {
        if pos < 0 || pos as usize >= self.d.len() {
            -1
        } else {
            self.d[pos as usize] as i32
        }
    }
    fn get_u16be(&self, pos: i64) -> Option<i32> {
        if pos < 0 || pos as usize + 2 > self.d.len() {
            return None;
        }
        let p = pos as usize;
        Some(((self.d[p] as i32) << 8) + self.d[p + 1] as i32)
    }
    fn get_u32be(&self, pos: i64) -> Option<u32> {
        self.get_uvar_be(pos, 4)
    }
    fn get_u32le(&self, pos: i64) -> Option<u32> {
        if pos < 0 || pos as usize + 4 > self.d.len() {
            return None;
        }
        let p = pos as usize;
        Some(u32::from_le_bytes([self.d[p], self.d[p + 1], self.d[p + 2], self.d[p + 3]]))
    }
    fn get_uvar_be(&self, pos: i64, size: i32) -> Option<u32> {
        if !(1..=4).contains(&size) || pos < 0 || pos as usize + size as usize > self.d.len() {
            return None;
        }
        let mut v: u32 = 0;
        for i in 0..size as usize {
            v = (v << 8) + self.d[pos as usize + i] as u32;
        }
        Some(v)
    }
    fn cmp(&self, pos: i64, s: &[u8]) -> bool {
        pos >= 0 && pos as usize + s.len() <= self.d.len() && &self.d[pos as usize..pos as usize + s.len()] == s
    }
}

/// `FoFiIdentifier::identifyStream` (applied to the decoded font file).
pub fn identify(data: &[u8]) -> FoFiIdentifierType {
    let r = Reader { d: data };
    //----- PFA
    if r.cmp(0, b"%!PS-AdobeFont-1") || r.cmp(0, b"%!FontType1") {
        return FoFiIdentifierType::Type1PFA;
    }
    //----- PFB
    if r.get_byte(0) == 0x80 && r.get_byte(1) == 0x01 {
        if let Some(n) = r.get_u32le(2) {
            if (n >= 16 && r.cmp(6, b"%!PS-AdobeFont-1")) || (n >= 11 && r.cmp(6, b"%!FontType1")) {
                return FoFiIdentifierType::Type1PFB;
            }
        }
    }
    //----- TrueType
    let b = |i| r.get_byte(i);
    if (b(0) == 0x00 && b(1) == 0x01 && b(2) == 0x00 && b(3) == 0x00)
        || (b(0) == 0x74 && b(1) == 0x72 && b(2) == 0x75 && b(3) == 0x65)
    {
        return FoFiIdentifierType::TrueType;
    }
    if b(0) == 0x74 && b(1) == 0x74 && b(2) == 0x63 && b(3) == 0x66 {
        return FoFiIdentifierType::TrueTypeCollection;
    }
    //----- OpenType
    if b(0) == 0x4f && b(1) == 0x54 && b(2) == 0x54 && b(3) == 0x4f {
        return identify_open_type(&r);
    }
    //----- CFF
    if b(0) == 0x01 && b(1) == 0x00 {
        return identify_cff(&r, 0);
    }
    // some tools embed CFF fonts with an extra whitespace char at the
    // beginning
    if b(1) == 0x01 && b(2) == 0x00 {
        return identify_cff(&r, 1);
    }
    FoFiIdentifierType::Unknown
}

fn identify_open_type(r: &Reader) -> FoFiIdentifierType {
    let n_tables = match r.get_u16be(4) {
        Some(n) => n,
        None => return FoFiIdentifierType::Unknown,
    };
    for i in 0..n_tables as i64 {
        if r.cmp(12 + i * 16, b"CFF ") {
            if let Some(offset) = r.get_u32be(12 + i * 16 + 8) {
                if offset < i32::MAX as u32 {
                    return match identify_cff(r, offset as i64) {
                        FoFiIdentifierType::CFF8Bit => FoFiIdentifierType::OpenTypeCFF8Bit,
                        FoFiIdentifierType::CFFCID => FoFiIdentifierType::OpenTypeCFFCID,
                        t => t,
                    };
                }
            }
            return FoFiIdentifierType::Unknown;
        }
    }
    FoFiIdentifierType::Unknown
}

fn identify_cff(r: &Reader, start: i64) -> FoFiIdentifierType {
    use FoFiIdentifierType::*;
    //----- read the header
    if r.get_byte(start) != 0x01 || r.get_byte(start + 1) != 0x00 {
        return Unknown;
    }
    let hdr_size = r.get_byte(start + 2);
    if hdr_size < 0 {
        return Unknown;
    }
    let off_size0 = r.get_byte(start + 3);
    if !(1..=4).contains(&off_size0) {
        return Unknown;
    }
    let mut pos = start + hdr_size as i64;
    //----- skip the name index
    let n = match r.get_u16be(pos) {
        Some(n) => n as i64,
        None => return Unknown,
    };
    if n == 0 {
        pos += 2;
    } else {
        let off_size1 = r.get_byte(pos + 2);
        if !(1..=4).contains(&off_size1) {
            return Unknown;
        }
        let offset1 = match r.get_uvar_be(pos + 3 + n * off_size1 as i64, off_size1) {
            Some(o) if o <= i32::MAX as u32 => o,
            _ => return Unknown,
        };
        pos += 3 + (n + 1) * off_size1 as i64 + offset1 as i64 - 1;
    }
    if pos < 0 {
        return Unknown;
    }
    //----- parse the top dict index
    let n = match r.get_u16be(pos) {
        Some(n) if n >= 1 => n as i64,
        _ => return Unknown,
    };
    let off_size1 = r.get_byte(pos + 2);
    if !(1..=4).contains(&off_size1) {
        return Unknown;
    }
    let os = off_size1 as i64;
    let offset0 = r.get_uvar_be(pos + 3, off_size1);
    let offset1 = r.get_uvar_be(pos + 3 + os, off_size1);
    let (offset0, offset1) = match (offset0, offset1) {
        (Some(a), Some(b)) if a <= i32::MAX as u32 && b <= i32::MAX as u32 && a <= b => (a as i64, b as i64),
        _ => return Unknown,
    };
    let p2 = pos + 3 + (n + 1) * os + offset0 - 1;
    let end_pos = pos + 3 + (n + 1) * os + offset1 - 1;
    let mut pos = p2;
    if pos < 0 || end_pos < 0 || pos > end_pos {
        return Unknown;
    }
    //----- parse the top dict, look for ROS as first entry
    while pos >= 0 && pos < end_pos {
        let b0 = r.get_byte(pos);
        if b0 == 0x1c {
            pos += 3;
        } else if b0 == 0x1d {
            pos += 5;
        } else if (0xf7..=0xfe).contains(&b0) {
            pos += 2;
        } else if (0x20..=0xf6).contains(&b0) {
            pos += 1;
        } else {
            break;
        }
    }
    if pos + 1 < end_pos && r.get_byte(pos) == 12 && r.get_byte(pos + 1) == 30 {
        CFFCID
    } else {
        CFF8Bit
    }
}

/// A font built-in encoding: 256 optional glyph names.
pub type Encoding = Vec<Option<Vec<u8>>>;

fn table_encoding(t: &[Option<&str>; 256]) -> Encoding {
    t.iter().map(|o| o.map(|s| s.as_bytes().to_vec())).collect()
}

// ------------------------------------------------------------------------
// FoFiType1
// ------------------------------------------------------------------------

/// Mirrors `class FoFiType1` (name and encoding only).
pub struct FoFiType1 {
    file: Vec<u8>,
    name: Option<Vec<u8>>,
    encoding: Option<Encoding>,
}

impl FoFiType1 {
    /// `FoFiType1::make(char *fileA, int lenA)` (never fails); the font is
    /// parsed immediately (xpdf parses on the first getName()).
    pub fn make(data: &[u8]) -> FoFiType1 {
        let mut f = FoFiType1 { file: undo_pfb(data), name: None, encoding: None };
        f.parse();
        f
    }
    /// `getName()`
    pub fn get_name(&self) -> Option<&[u8]> {
        self.name.as_deref()
    }
    /// `getEncoding()`
    pub fn get_encoding(&self) -> Option<&Encoding> {
        self.encoding.as_ref()
    }

    /// `getNextLine(char *line)`
    fn get_next_line(&self, mut line: usize) -> Option<usize> {
        let len = self.file.len();
        while line < len && self.file[line] != 0x0a && self.file[line] != 0x0d {
            line += 1;
        }
        if line < len && self.file[line] == 0x0d {
            line += 1;
        }
        if line < len && self.file[line] == 0x0a {
            line += 1;
        }
        if line >= len {
            None
        } else {
            Some(line)
        }
    }

    /// `strncpy(buf, src, n); buf[n] = 0` -> NUL-terminated buffer
    fn copy_buf(&self, start: usize, n: usize) -> Vec<u8> {
        let mut buf = vec![0u8; n + 1];
        for i in 0..n {
            let c = self.file.get(start + i).copied().unwrap_or(0);
            if c == 0 {
                break; // strncpy stops at NUL (and pads with NULs)
            }
            buf[i] = c;
        }
        buf
    }

    fn starts(&self, pos: usize, s: &[u8]) -> bool {
        // strncmp(line, s, n) == 0 with the file's bytes; a NUL in the
        // file never equals a char of s
        pos + s.len() <= self.file.len() && &self.file[pos..pos + s.len()] == s
    }

    /// `FoFiType1::parse()`
    fn parse(&mut self) {
        let len = self.file.len();
        let mut got_matrix = false;
        let mut line = Some(0usize);
        let mut i = 1;
        while i <= 100 && line.is_some() && (self.name.is_none() || self.encoding.is_none() || !got_matrix) {
            let l = line.unwrap();
            if self.name.is_none() && l + 9 <= len && self.starts(l, b"/FontName") {
                // get font name
                let n = 255.min(len - l);
                let buf = self.copy_buf(l, n);
                if let Some(p) = cstr_chr(&buf, 9, b'/') {
                    if let Some((s, e)) = strtok_first(&buf, p + 1, b" \t\n\r") {
                        self.name = Some(buf[s..e].to_vec());
                    }
                }
                line = self.get_next_line(l);
            } else if self.encoding.is_none() && l + 30 <= len && self.starts(l, b"/Encoding StandardEncoding def") {
                // get encoding
                self.encoding = Some(table_encoding(&FOFI_TYPE1_STANDARD_ENCODING));
            } else if self.encoding.is_none() && l + 19 <= len && self.starts(l, b"/Encoding 256 array") {
                let mut enc: Encoding = vec![None; 256];
                let mut j = 0;
                let mut cur = self.get_next_line(l);
                loop {
                    if j >= 300 {
                        break;
                    }
                    let ln = match cur {
                        Some(x) => x,
                        None => break,
                    };
                    let line1 = match self.get_next_line(ln) {
                        Some(x) => x,
                        None => break,
                    };
                    let n = (line1 - ln).min(255);
                    let buf = self.copy_buf(ln, n);
                    let mut p = 0usize;
                    while buf[p] == b' ' || buf[p] == b'\t' {
                        p += 1;
                    }
                    let ends_with_dup = (ln >= 4 && &self.file[ln - 4..ln] == b"dup\x0a")
                        || (ln >= 5 && &self.file[ln - 5..ln - 1] == b"dup\x0d");
                    let starts_with_dup = cstr_starts(&buf, p, b"dup");
                    if ends_with_dup || starts_with_dup {
                        if starts_with_dup {
                            p += 3;
                        }
                        loop {
                            while buf[p] == b' ' || buf[p] == b'\t' {
                                p += 1;
                            }
                            let mut code: u32 = 0;
                            let base: u32;
                            if buf[p] == b'8' && buf[p + 1] == b'#' {
                                base = 8;
                                p += 2;
                            } else if buf[p].is_ascii_digit() {
                                base = 10;
                            } else {
                                break;
                            }
                            while buf[p] >= b'0' && (buf[p] as u32) < b'0' as u32 + base {
                                code = code.wrapping_mul(base).wrapping_add((buf[p] - b'0') as u32);
                                p += 1;
                            }
                            while buf[p] == b' ' || buf[p] == b'\t' {
                                p += 1;
                            }
                            if buf[p] != b'/' {
                                break;
                            }
                            p += 1;
                            let mut p2 = p;
                            while buf[p2] != 0 && buf[p2] != b' ' && buf[p2] != b'\t' {
                                p2 += 1;
                            }
                            if code < 256 {
                                enc[code as usize] = Some(buf[p..p2].to_vec());
                            }
                            p = p2;
                            while buf[p] == b' ' || buf[p] == b'\t' {
                                p += 1;
                            }
                            if !cstr_starts(&buf, p, b"put") {
                                break;
                            }
                            p += 3;
                            while buf[p] == b' ' || buf[p] == b'\t' {
                                p += 1;
                            }
                            if !cstr_starts(&buf, p, b"dup") {
                                break;
                            }
                            p += 3;
                        }
                    } else {
                        // strtok(buf, " \t") && (p = strtok(NULL, " \t\n\r"))
                        // && !strcmp(p, "def")
                        if let Some((_, e1)) = strtok_first(&buf, 0, b" \t") {
                            // strtok replaced the delimiter after the first
                            // token with NUL; continue after it
                            let next = if buf[e1] == 0 && e1 < n { e1 + 1 } else { e1 };
                            if next > e1 || buf[e1] != 0 {
                                if let Some((s2, e2)) = strtok_first(&buf, next, b" \t\n\r") {
                                    if &buf[s2..e2] == b"def" {
                                        cur = Some(ln);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    j += 1;
                    cur = Some(line1);
                }
                line = cur;
                self.encoding = Some(enc);
                //~ check for getinterval/putinterval junk
            } else if !got_matrix && l + 11 <= len && self.starts(l, b"/FontMatrix") {
                got_matrix = true;
            } else {
                line = self.get_next_line(l);
            }
            i += 1;
        }
    }
}

/// `strchr(buf + from, c)` on a NUL-terminated buffer.
fn cstr_chr(buf: &[u8], from: usize, c: u8) -> Option<usize> {
    let mut i = from;
    while i < buf.len() && buf[i] != 0 {
        if buf[i] == c {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// `!strncmp(buf + p, s, strlen(s))` on a NUL-terminated buffer.
fn cstr_starts(buf: &[u8], p: usize, s: &[u8]) -> bool {
    p + s.len() <= buf.len() && &buf[p..p + s.len()] == s
}

/// First `strtok(buf + from, delims)` token as (start, end); the token ends
/// at a delimiter or NUL.
fn strtok_first(buf: &[u8], from: usize, delims: &[u8]) -> Option<(usize, usize)> {
    let mut s = from;
    while s < buf.len() && buf[s] != 0 && delims.contains(&buf[s]) {
        s += 1;
    }
    if s >= buf.len() || buf[s] == 0 {
        return None;
    }
    let mut e = s;
    while e < buf.len() && buf[e] != 0 && !delims.contains(&buf[e]) {
        e += 1;
    }
    Some((s, e))
}

/// `FoFiType1::undoPFB()`: remove the PFB segment headers.
fn undo_pfb(data: &[u8]) -> Vec<u8> {
    if data.first() != Some(&0x80) {
        return data.to_vec();
    }
    let len = data.len();
    let mut out = Vec::with_capacity(len);
    let mut pos1 = 0usize;
    while pos1 < len && data[pos1] == 0x80 {
        if pos1 + 1 >= len {
            break;
        }
        let t = data[pos1 + 1];
        if !(1..=2).contains(&t) {
            break;
        }
        if pos1 + 6 > len {
            break;
        }
        let seg_len = u32::from_le_bytes([data[pos1 + 2], data[pos1 + 3], data[pos1 + 4], data[pos1 + 5]]) as usize;
        pos1 += 6;
        // checkRegion(pos1, segLen)
        if seg_len > i32::MAX as usize - pos1 || pos1 + seg_len > len {
            break;
        }
        out.extend_from_slice(&data[pos1..pos1 + seg_len]);
        pos1 += seg_len;
    }
    out
}

// ------------------------------------------------------------------------
// FoFiType1C
// ------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Index {
    pos: i32,
    len: i32,
    off_size: i32,
    start_pos: i32,
    end_pos: i32,
}

#[derive(Clone, Copy)]
enum Op {
    Int(i32),
    Float(f64),
    Operator(i32),
}

impl Op {
    fn to_int(self) -> i32 {
        match self {
            Op::Int(i) => i,
            Op::Float(f) => {
                if !(-2e9..=2e9).contains(&f) {
                    0
                } else {
                    f as i32
                }
            }
            Op::Operator(_) => 0,
        }
    }
}

/// Mirrors `class FoFiType1C` (parsing for name and encoding only).
pub struct FoFiType1C {
    file: Vec<u8>,
    ok: bool,
    name: Vec<u8>,
    encoding: Option<Encoding>,
    string_idx: Index,
    n_glyphs: i32,
    charset: Vec<u16>,
    ops: Vec<Op>,
    // top dict values used here
    first_op: i32,
    charset_offset: i32,
    encoding_offset: i32,
    char_strings_offset: i32,
    private_size: i32,
    private_offset: i32,
    fd_array_offset: i32,
    fd_select_offset: i32,
    n_fds: i32,
}

impl FoFiType1C {
    /// `FoFiType1C::make(char *fileA, int lenA)`: `None` if parsing fails.
    pub fn make(data: &[u8]) -> Option<FoFiType1C> {
        let mut f = FoFiType1C {
            file: data.to_vec(),
            ok: true,
            name: Vec::new(),
            encoding: None,
            string_idx: Index::default(),
            n_glyphs: 0,
            charset: Vec::new(),
            ops: Vec::new(),
            first_op: -1,
            charset_offset: 0,
            encoding_offset: 0,
            char_strings_offset: 0,
            private_size: 0,
            private_offset: 0,
            fd_array_offset: 0,
            fd_select_offset: 0,
            n_fds: 0,
        };
        if f.parse() {
            Some(f)
        } else {
            None
        }
    }
    /// `getName()`
    pub fn get_name(&self) -> Option<&[u8]> {
        Some(&self.name)
    }
    /// `getEncoding()`
    pub fn get_encoding(&self) -> Option<&Encoding> {
        self.encoding.as_ref()
    }

    fn len(&self) -> i32 {
        self.file.len() as i32
    }
    fn get_u8(&mut self, pos: i32) -> i32 {
        if pos < 0 || pos >= self.len() {
            self.ok = false;
            return 0;
        }
        self.file[pos as usize] as i32
    }
    fn get_u16be(&mut self, pos: i32) -> i32 {
        if pos < 0 || pos > i32::MAX - 1 || pos + 1 >= self.len() {
            self.ok = false;
            return 0;
        }
        ((self.file[pos as usize] as i32) << 8) + self.file[pos as usize + 1] as i32
    }
    fn get_uvar_be(&mut self, pos: i32, size: i32) -> u32 {
        if pos < 0 || pos > i32::MAX - size || pos + size > self.len() {
            self.ok = false;
            return 0;
        }
        let mut x: u32 = 0;
        for i in 0..size {
            x = (x << 8).wrapping_add(self.file[(pos + i) as usize] as u32);
        }
        x
    }
    fn check_region(&self, pos: i32, size: i32) -> bool {
        pos >= 0 && size >= 0 && size <= i32::MAX - pos && pos + size <= self.len()
    }

    /// `getIndex(int pos, Type1CIndex *idx, GBool *ok)`
    fn get_index(&mut self, pos: i32) -> Index {
        let mut idx = Index { pos, ..Default::default() };
        idx.len = self.get_u16be(pos);
        if idx.len == 0 {
            // empty indexes are legal and contain just the length field
            idx.off_size = 0;
            idx.start_pos = pos.wrapping_add(2);
            idx.end_pos = idx.start_pos;
        } else {
            idx.off_size = self.get_u8(pos + 2);
            if idx.off_size < 1 || idx.off_size > 4 {
                self.ok = false;
            }
            idx.start_pos = pos
                .wrapping_add(3)
                .wrapping_add((idx.len + 1).wrapping_mul(idx.off_size))
                .wrapping_sub(1);
            if idx.start_pos < 0 || idx.start_pos >= self.len() {
                self.ok = false;
            }
            let v = self.get_uvar_be(pos.wrapping_add(3).wrapping_add(idx.len.wrapping_mul(idx.off_size)), idx.off_size);
            idx.end_pos = idx.start_pos.wrapping_add(v as i32);
            if idx.end_pos < idx.start_pos || idx.end_pos > self.len() {
                self.ok = false;
            }
        }
        idx
    }

    /// `getIndexVal(Type1CIndex *idx, int i, Type1CIndexVal *val, GBool *ok)`
    fn get_index_val(&mut self, idx: &Index, i: i32) -> (i32, i32) {
        if i < 0 || i >= idx.len {
            self.ok = false;
            return (0, 0);
        }
        let a = self.get_uvar_be(idx.pos + 3 + i * idx.off_size, idx.off_size);
        let pos0 = idx.start_pos.wrapping_add(a as i32);
        let b = self.get_uvar_be(idx.pos + 3 + (i + 1) * idx.off_size, idx.off_size);
        let pos1 = idx.start_pos.wrapping_add(b as i32);
        if pos0 < idx.start_pos || pos0 > idx.end_pos || pos1 <= idx.start_pos || pos1 > idx.end_pos || pos1 < pos0 {
            self.ok = false;
        }
        (pos0, pos1.wrapping_sub(pos0))
    }

    /// `getString(int sid, char *buf, GBool *ok)`
    fn get_string(&mut self, sid: i32) -> Vec<u8> {
        if sid < 0 {
            Vec::new()
        } else if sid < 391 {
            FOFI_TYPE1C_STD_STRINGS[sid as usize].as_bytes().to_vec()
        } else {
            let si = self.string_idx;
            let (pos, len) = self.get_index_val(&si, sid - 391);
            if self.ok {
                let n = len.min(255).max(0) as usize;
                // strncpy: stops at NUL
                let mut v = Vec::with_capacity(n);
                for k in 0..n {
                    let c = self.file.get(pos as usize + k).copied().unwrap_or(0);
                    if c == 0 {
                        break;
                    }
                    v.push(c);
                }
                v
            } else {
                Vec::new()
            }
        }
    }

    /// `getOp(int pos, GBool charstring, GBool *ok)` (charstring = false)
    fn get_op(&mut self, mut pos: i32) -> i32 {
        const NYB_CHARS: &[u8; 16] = b"0123456789.ee -\0";
        let b0 = self.get_u8(pos);
        pos += 1;
        let op;
        if b0 == 28 {
            let mut x = self.get_u8(pos);
            pos += 1;
            x = (x << 8) | self.get_u8(pos);
            pos += 1;
            if x & 0x8000 != 0 {
                x |= !0xffff;
            }
            op = Op::Int(x);
        } else if b0 == 29 {
            let mut x: i32 = 0;
            for _ in 0..4 {
                x = (x << 8) | self.get_u8(pos);
                pos += 1;
            }
            op = Op::Int(x);
        } else if b0 == 30 {
            let mut buf: Vec<u8> = Vec::new();
            loop {
                let b1 = self.get_u8(pos);
                pos += 1;
                let nyb0 = (b1 >> 4) as usize;
                let nyb1 = (b1 & 0x0f) as usize;
                if nyb0 == 0xf {
                    break;
                }
                buf.push(NYB_CHARS[nyb0]);
                if buf.len() == 64 {
                    break;
                }
                if nyb0 == 0xc {
                    buf.push(b'-');
                }
                if buf.len() == 64 {
                    break;
                }
                if nyb1 == 0xf {
                    break;
                }
                buf.push(NYB_CHARS[nyb1]);
                if buf.len() == 64 {
                    break;
                }
                if nyb1 == 0xc {
                    buf.push(b'-');
                }
                if buf.len() >= 64 {
                    break;
                }
            }
            op = Op::Float(atof(&buf));
        } else if (32..=246).contains(&b0) {
            op = Op::Int(b0 - 139);
        } else if (247..=250).contains(&b0) {
            let v = self.get_u8(pos);
            pos += 1;
            op = Op::Int(((b0 - 247) << 8) + v + 108);
        } else if (251..=254).contains(&b0) {
            let v = self.get_u8(pos);
            pos += 1;
            op = Op::Int(-((b0 - 251) << 8) - v - 108);
        } else if b0 == 12 {
            let v = self.get_u8(pos);
            pos += 1;
            op = Op::Operator(0x0c00 + v);
        } else {
            op = Op::Operator(b0);
        }
        if self.ops.len() < 49 {
            self.ops.push(op);
        }
        pos
    }

    /// Run a DICT through getOp, calling `f(op, operands)` for each
    /// operator; stops on parse errors.  Returns early when `f` says so.
    fn read_dict(&mut self, offset: i32, length: i32, mut f: impl FnMut(&mut Self, i32, &[Op]) -> bool) {
        let mut pos = offset;
        self.ops.clear();
        while pos < offset.wrapping_add(length) {
            pos = self.get_op(pos);
            if !self.ok {
                break;
            }
            if let Some(&Op::Operator(o)) = self.ops.last() {
                let n = self.ops.len() - 1;
                let operands: Vec<Op> = self.ops[..n].to_vec();
                if !f(self, o, &operands) {
                    break;
                }
                self.ops.clear();
            }
        }
    }

    /// `FoFiType1C::parse()`
    fn parse(&mut self) -> bool {
        self.ok = true;
        // some tools embed Type 1C fonts with an extra whitespace char at
        // the beginning
        if !self.file.is_empty() && self.file[0] != 0x01 {
            self.file.remove(0);
        }
        // find the indexes
        let p = self.get_u8(2);
        let name_idx = self.get_index(p);
        let top_dict_idx = self.get_index(name_idx.end_pos);
        let string_idx = self.get_index(top_dict_idx.end_pos);
        let _gsubr_idx = self.get_index(string_idx.end_pos);
        self.string_idx = string_idx;
        if !self.ok {
            return false;
        }
        // read the first font name
        let (pos, len) = self.get_index_val(&name_idx, 0);
        if !self.ok {
            return false;
        }
        self.name = self.file[pos as usize..(pos + len) as usize].to_vec();

        // read the top dict for the first font
        self.read_top_dict(&top_dict_idx);

        // for CID fonts: read the FDArray dicts and private dicts
        if self.first_op == 0x0c1e {
            if self.fd_array_offset == 0 {
                self.n_fds = 1;
                self.read_private_dict(0, 0);
            } else {
                let fd_idx = self.get_index(self.fd_array_offset);
                if !self.ok {
                    return false;
                }
                self.n_fds = fd_idx.len;
                if self.n_fds < 1 {
                    return false;
                }
                for i in 0..self.n_fds {
                    let (p, l) = self.get_index_val(&fd_idx, i);
                    if !self.ok {
                        return false;
                    }
                    self.read_fd(p, l);
                }
            }
        // for 8-bit fonts: read the private dict
        } else {
            self.n_fds = 1;
            self.read_private_dict(self.private_offset, self.private_size);
        }
        // check for parse errors in the private dict(s)
        if !self.ok {
            return false;
        }
        // get the charstrings index
        if self.char_strings_offset <= 0 {
            return false;
        }
        let cs = self.get_index(self.char_strings_offset);
        if !self.ok {
            return false;
        }
        self.n_glyphs = cs.len;
        // for CID fonts: read the FDSelect table
        if self.first_op == 0x0c1e {
            self.read_fd_select();
            if !self.ok {
                return false;
            }
        }
        // read the charset
        if !self.read_charset() {
            return false;
        }
        // for 8-bit fonts: build the encoding
        if self.first_op != 0x0c14 && self.first_op != 0x0c1e {
            self.build_encoding();
            if !self.ok {
                return false;
            }
        }
        self.ok
    }

    /// `readTopDict()` (only the fields this crate needs)
    fn read_top_dict(&mut self, top_dict_idx: &Index) {
        self.first_op = -1;
        let (pos, len) = self.get_index_val(top_dict_idx, 0);
        self.read_dict(pos, len, |s, op, a| {
            if s.first_op < 0 {
                s.first_op = op;
            }
            let i0 = a.first().map(|o| o.to_int()).unwrap_or(0);
            let i1 = a.get(1).map(|o| o.to_int()).unwrap_or(0);
            match op {
                0x000f => s.charset_offset = i0,
                0x0010 => s.encoding_offset = i0,
                0x0011 => s.char_strings_offset = i0,
                0x0012 => {
                    s.private_size = i0;
                    s.private_offset = i1;
                }
                0x0c24 => s.fd_array_offset = i0,
                0x0c25 => s.fd_select_offset = i0,
                _ => {}
            }
            true
        });
    }

    /// `readFD(int offset, int length, Type1CPrivateDict *pDict)`
    fn read_fd(&mut self, offset: i32, length: i32) {
        let mut p_size = 0;
        let mut p_offset = 0;
        let mut pos = offset;
        self.ops.clear();
        while pos < offset.wrapping_add(length) {
            pos = self.get_op(pos);
            if !self.ok {
                return;
            }
            if let Some(&Op::Operator(o)) = self.ops.last() {
                if o == 0x0012 {
                    if self.ops.len() < 3 {
                        self.ok = false;
                        return;
                    }
                    p_size = self.ops[0].to_int();
                    p_offset = self.ops[1].to_int();
                    break;
                }
                self.ops.clear();
            }
        }
        self.read_private_dict(p_offset, p_size);
    }

    /// `readPrivateDict(int offset, int length, ...)`: only its effect on
    /// the parse status matters here.
    fn read_private_dict(&mut self, offset: i32, length: i32) {
        if offset == 0 || length == 0 {
            return;
        }
        self.read_dict(offset, length, |_, _, _| true);
    }

    /// `readFDSelect()`
    fn read_fd_select(&mut self) {
        if self.fd_select_offset == 0 {
            return;
        }
        let mut pos = self.fd_select_offset;
        let fmt = self.get_u8(pos);
        pos += 1;
        if !self.ok {
            return;
        }
        if fmt == 0 {
            if !self.check_region(pos, self.n_glyphs) {
                self.ok = false;
                return;
            }
            for gid0 in 0..self.n_glyphs {
                if self.file[(pos + gid0) as usize] as i32 >= self.n_fds {
                    self.ok = false;
                    return;
                }
            }
        } else if fmt == 3 {
            let n_ranges = self.get_u16be(pos);
            pos += 2;
            let mut gid0 = self.get_u16be(pos);
            pos += 2;
            for _ in 1..=n_ranges {
                let fd = self.get_u8(pos);
                pos += 1;
                let gid1 = self.get_u16be(pos);
                if !self.ok {
                    return;
                }
                pos += 2;
                if gid0 > gid1 || gid1 > self.n_glyphs || fd >= self.n_fds {
                    self.ok = false;
                    return;
                }
                gid0 = gid1;
            }
        }
    }

    /// `readCharset()`
    fn read_charset(&mut self) -> bool {
        match self.charset_offset {
            0 => {
                self.charset = FOFI_TYPE1C_ISO_ADOBE_CHARSET.to_vec();
                self.n_glyphs = self.n_glyphs.min(229);
            }
            1 => {
                self.charset = FOFI_TYPE1C_EXPERT_CHARSET.to_vec();
                self.n_glyphs = self.n_glyphs.min(166);
            }
            2 => {
                self.charset = FOFI_TYPE1C_EXPERT_SUBSET_CHARSET.to_vec();
                self.n_glyphs = self.n_glyphs.min(87);
            }
            _ => {
                let n = self.n_glyphs.max(0) as usize;
                let mut cs = vec![0u16; n];
                let mut pos = self.charset_offset;
                let fmt = self.get_u8(pos);
                pos += 1;
                if fmt == 0 {
                    for c in cs.iter_mut().skip(1) {
                        *c = self.get_u16be(pos) as u16;
                        pos += 2;
                        if !self.ok {
                            break;
                        }
                    }
                } else if fmt == 1 || fmt == 2 {
                    let mut i = 1usize;
                    while i < n {
                        let mut c = self.get_u16be(pos);
                        pos += 2;
                        let n_left = if fmt == 1 {
                            let v = self.get_u8(pos);
                            pos += 1;
                            v
                        } else {
                            let v = self.get_u16be(pos);
                            pos += 2;
                            v
                        };
                        if !self.ok {
                            break;
                        }
                        let mut j = 0;
                        while j <= n_left && i < n {
                            cs[i] = c as u16;
                            i += 1;
                            c += 1;
                            j += 1;
                        }
                    }
                }
                if !self.ok {
                    return false;
                }
                self.charset = cs;
            }
        }
        true
    }

    /// `buildEncoding()`
    fn build_encoding(&mut self) {
        if self.encoding_offset == 0 {
            self.encoding = Some(table_encoding(&FOFI_TYPE1_STANDARD_ENCODING));
            return;
        } else if self.encoding_offset == 1 {
            self.encoding = Some(table_encoding(&FOFI_TYPE1_EXPERT_ENCODING));
            return;
        }
        let mut enc: Encoding = vec![None; 256];
        let mut pos = self.encoding_offset;
        let enc_format = self.get_u8(pos);
        pos += 1;
        if !self.ok {
            self.encoding = Some(enc);
            return;
        }
        let charset = |s: &Self, i: i32| -> i32 { s.charset.get(i as usize).copied().unwrap_or(0) as i32 };
        if (enc_format & 0x7f) == 0 {
            let mut n_codes = 1 + self.get_u8(pos);
            pos += 1;
            if !self.ok {
                self.encoding = Some(enc);
                return;
            }
            if n_codes > self.n_glyphs {
                n_codes = self.n_glyphs;
            }
            for i in 1..n_codes {
                let c = self.get_u8(pos);
                pos += 1;
                if !self.ok {
                    self.encoding = Some(enc);
                    return;
                }
                let sid = charset(self, i);
                enc[c as usize] = Some(self.get_string(sid));
            }
        } else if (enc_format & 0x7f) == 1 {
            let n_ranges = self.get_u8(pos);
            pos += 1;
            if !self.ok {
                self.encoding = Some(enc);
                return;
            }
            let mut n_codes = 1;
            for _ in 0..n_ranges {
                let mut c = self.get_u8(pos);
                pos += 1;
                let n_left = self.get_u8(pos);
                pos += 1;
                if !self.ok {
                    self.encoding = Some(enc);
                    return;
                }
                let mut j = 0;
                while j <= n_left && n_codes < self.n_glyphs {
                    if c < 256 {
                        let sid = charset(self, n_codes);
                        enc[c as usize] = Some(self.get_string(sid));
                    }
                    n_codes += 1;
                    c += 1;
                    j += 1;
                }
            }
        }
        if enc_format & 0x80 != 0 {
            let n_sups = self.get_u8(pos);
            pos += 1;
            if !self.ok {
                self.encoding = Some(enc);
                return;
            }
            for _ in 0..n_sups {
                let c = self.get_u8(pos);
                pos += 1;
                if !self.ok {
                    self.encoding = Some(enc);
                    return;
                }
                let sid = self.get_u16be(pos);
                pos += 2;
                if !self.ok {
                    self.encoding = Some(enc);
                    return;
                }
                enc[c as usize] = Some(self.get_string(sid));
            }
        }
        self.encoding = Some(enc);
    }
}
