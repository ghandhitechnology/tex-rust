//! Small string helpers mirroring kpathsea's lib.h / c-pathch.h / c-ctype.h
//! and the tiny one-function C files (absolute.c, find-suffix.c,
//! rm-suffix.c, make-suffix.c, extend-fname.c, xbasename.c, xdirname.c,
//! uppercasify.c, line.c, kdefault.c helpers).
//!
//! All strings are byte strings (`[u8]`), like C `char *`, because file
//! names in TeX are bytes and the Windows build of kpathsea works in the
//! ANSI / UTF-8 code page depending on `command_line_encoding`.

/// `DIR_SEP` on DOSISH systems.
pub const DIR_SEP: u8 = b'/';
/// `ENV_SEP` on DOSISH systems.
pub const ENV_SEP: u8 = b';';

/// `IS_DIR_SEP` (DOSISH): `/` or `\`.
#[inline]
pub fn is_dir_sep(c: u8) -> bool {
    c == b'/' || c == b'\\'
}

/// `IS_DIR_SEP_CH` (WIN32): only `/`, used after slashification.
#[inline]
pub fn is_dir_sep_ch(c: u8) -> bool {
    c == b'/'
}

/// `IS_DEVICE_SEP`.
#[inline]
pub fn is_device_sep(c: u8) -> bool {
    c == b':'
}

/// `IS_KPSE_SEP` / `IS_ENV_SEP` on Windows: only `;`.
#[inline]
pub fn is_kpse_sep(c: u8) -> bool {
    c == ENV_SEP
}

/// Byte at index `i`, or NUL past the end (C string semantics).
#[inline]
pub fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// `NAME_BEGINS_WITH_DEVICE`: `*(name) && IS_DEVICE_SEP((name)[1])`.
#[inline]
pub fn name_begins_with_device(s: &[u8]) -> bool {
    at(s, 0) != 0 && is_device_sep(at(s, 1))
}

/// `IS_UNC_NAME` (WIN32).
#[inline]
pub fn is_unc_name(s: &[u8]) -> bool {
    s.len() >= 3 && is_dir_sep(s[0]) && is_dir_sep(s[1]) && s[2].is_ascii_alphanumeric()
}

/// `ISSPACE` from c-ctype.h (ASCII only).
#[inline]
pub fn isspace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `ISALNUM` (ASCII only).
#[inline]
pub fn isalnum(c: u8) -> bool {
    c.is_ascii_alphanumeric()
}

/// `FILESTRCASEEQ` with MONOCASE_FILENAMES: `_stricmp` in the C locale.
#[inline]
pub fn file_str_case_eq(a: &[u8], b: &[u8]) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// `FILECHARCASEEQ`: `toupper(c1) == toupper(c2)`.
#[inline]
pub fn file_char_case_eq(a: u8, b: u8) -> bool {
    a.to_ascii_uppercase() == b.to_ascii_uppercase()
}

/// `kpse_cnf_p`: true unless NULL, empty, or starting with `f` or `0`.
pub fn cnf_p(s: Option<&[u8]>) -> bool {
    match s {
        Some(s) => !s.is_empty() && s[0] != b'f' && s[0] != b'0',
        None => false,
    }
}

/// C `atoi` (MSVC): skip white space, optional sign, decimal digits.
pub fn atoi(s: &[u8]) -> i32 {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut n: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        n = n * 10 + (s[i] - b'0') as i64;
        if n > i32::MAX as i64 + 1 {
            n = i32::MAX as i64 + 1;
        }
        i += 1;
    }
    let n = if neg { -n } else { n };
    n.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// `uppercasify`: ASCII upper-casing.
pub fn uppercasify(s: &[u8]) -> Vec<u8> {
    s.to_ascii_uppercase()
}

/// absolute.c `kpathsea_absolute_p` (DOSISH + WIN32 branches).
pub fn absolute_p(filename: &[u8], relative_ok: bool) -> bool {
    let c0 = at(filename, 0);
    let c1 = at(filename, 1);
    let absolute = is_dir_sep(c0)
        || (c0 != 0 && is_device_sep(c1))
        || (c0 == b'\\' && c1 == b'\\')
        || (c0 == b'/' && c1 == b'/');
    let explicit_relative = relative_ok
        && (c0 == b'.' && (is_dir_sep(c1) || (c1 == b'.' && is_dir_sep(at(filename, 2)))));
    absolute || explicit_relative
}

/// find-suffix.c: index of the first character after the last `.` in the
/// last directory element, or `None`.
pub fn find_suffix(name: &[u8]) -> Option<usize> {
    let dot = name.iter().rposition(|&c| c == b'.')?;
    if name[dot + 1..].iter().any(|&c| is_dir_sep(c)) {
        return None;
    }
    Some(dot + 1)
}

/// rm-suffix.c.
pub fn remove_suffix(s: &[u8]) -> Vec<u8> {
    match find_suffix(s) {
        Some(i) => s[..i - 1].to_vec(),
        None => s.to_vec(),
    }
}

/// extend-fname.c.
pub fn extend_filename(name: &[u8], suffix: &[u8]) -> Vec<u8> {
    if find_suffix(name).is_none() {
        cat(&[name, b".", suffix])
    } else {
        name.to_vec()
    }
}

/// make-suffix.c (WIN32: result is slashified).
#[allow(dead_code)]
pub fn make_suffix(s: &[u8], suffix: &[u8]) -> Vec<u8> {
    let mut dot = s.iter().rposition(|&c| c == b'.');
    if let Some(d) = dot {
        if s[d + 1..].iter().any(|&c| is_dir_sep(c)) {
            dot = None;
        }
    }
    let mut r = match dot {
        None => cat(&[s, b".", suffix]),
        Some(d) => cat(&[&s[..d + 1], suffix]),
    };
    for c in r.iter_mut() {
        if *c == b'\\' {
            *c = b'/';
        }
    }
    r
}

/// xbasename.c: the part of NAME after the last directory separator,
/// skipping a device or UNC prefix.
pub fn xbasename(name: &[u8]) -> &[u8] {
    let mut base = 0usize;
    if name_begins_with_device(name) {
        base = 2;
    } else if is_unc_name(name) {
        let mut limit = 2;
        while limit < name.len() && !is_dir_sep(name[limit]) {
            limit += 1;
        }
        let had = at(name, limit) != 0;
        limit += 1;
        if had && at(name, limit) != 0 && !is_dir_sep(at(name, limit)) {
            while limit < name.len() && !is_dir_sep(name[limit]) {
                limit += 1;
            }
        } else {
            limit = 0;
        }
        base = limit;
    }
    let mut b = base;
    for (i, &c) in name.iter().enumerate().skip(base) {
        if is_dir_sep(c) {
            b = i + 1;
        }
    }
    &name[b.min(name.len())..]
}

/// xdirname.c (WIN32: result is slashified).
pub fn xdirname(name: &[u8]) -> Vec<u8> {
    let mut limit = 0usize;
    if name_begins_with_device(name) {
        limit = 2;
    } else if is_unc_name(name) {
        limit = 2;
        while limit < name.len() && !is_dir_sep(name[limit]) {
            limit += 1;
        }
        let had = at(name, limit) != 0;
        limit += 1;
        if had && at(name, limit) != 0 && !is_dir_sep(at(name, limit)) {
            while limit < name.len() && !is_dir_sep(name[limit]) {
                limit += 1;
            }
            limit -= 1;
        } else {
            limit = 0;
        }
    }
    let mut loc = name.len();
    while loc > limit && !is_dir_sep(name[loc - 1]) {
        loc -= 1;
    }
    let mut ret = if loc == limit {
        if limit == 0 {
            b".".to_vec()
        } else if limit == 2 {
            vec![name[0], name[1], b'.']
        } else {
            name.to_vec()
        }
    } else {
        while loc > limit + 1 && is_dir_sep(name[loc - 1]) {
            loc -= 1;
        }
        name[..loc].to_vec()
    };
    for c in ret.iter_mut() {
        if *c == b'\\' {
            *c = b'/';
        }
    }
    ret
}

/// Concatenate byte slices.
pub fn cat(parts: &[&[u8]]) -> Vec<u8> {
    let n = parts.iter().map(|p| p.len()).sum();
    let mut v = Vec::with_capacity(n);
    for p in parts {
        v.extend_from_slice(p);
    }
    v
}

/// Find `needle` in `hay` starting at `from` (C `strstr`).
pub fn find_sub(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(hay.len()));
    }
    if from >= hay.len() || hay.len() - from < needle.len() {
        return None;
    }
    let first = needle[0];
    let last_start = hay.len() - needle.len();
    let mut i = from;
    while i <= last_start {
        match hay[i..=last_start].iter().position(|&c| c == first) {
            None => return None,
            Some(off) => {
                i += off;
                if &hay[i..i + needle.len()] == needle {
                    return Some(i);
                }
                i += 1;
            }
        }
    }
    None
}

/// Truncate at the first NUL (C string view of a buffer).
pub fn c_str(s: &[u8]) -> &[u8] {
    match s.iter().position(|&b| b == 0) {
        Some(i) => &s[..i],
        None => s,
    }
}

/// Read a whole file as the MSVC CRT does in text mode (`"r"`): CR-LF
/// becomes LF and Ctrl-Z ends the file.
pub fn read_text_file(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    let raw = std::fs::read(path)?;
    Ok(text_mode(raw))
}

/// Apply MSVC text-mode input translation to a buffer.
pub fn text_mode(mut raw: Vec<u8>) -> Vec<u8> {
    if raw.contains(&0x1a) {
        if let Some(z) = raw.iter().position(|&b| b == 0x1a) {
            raw.truncate(z);
        }
    }
    if !raw.contains(&b'\r') {
        return raw;
    }
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\r' && i + 1 < raw.len() && raw[i + 1] == b'\n' {
            out.push(b'\n');
            i += 2;
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    out
}

/// line.c `read_line` over an in-memory buffer: lines end at LF or CR
/// (CR-LF counts once), NUL bytes are dropped, a final unterminated line is
/// returned.  The callback receives each line.
pub struct Lines<'a> {
    buf: &'a [u8],
    pos: usize,
    scratch: Vec<u8>,
}

impl<'a> Lines<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Lines { buf, pos: 0, scratch: Vec::new() }
    }

    /// Next line, or `None` at end of file.  The returned slice is valid
    /// until the next call.
    pub fn next_line(&mut self) -> Option<&[u8]> {
        let buf = self.buf;
        if self.pos >= buf.len() {
            return None;
        }
        let start = self.pos;
        let mut i = start;
        while i < buf.len() && buf[i] != b'\n' && buf[i] != b'\r' {
            i += 1;
        }
        let end = i;
        if i < buf.len() {
            if buf[i] == b'\r' && i + 1 < buf.len() && buf[i + 1] == b'\n' {
                i += 2;
            } else {
                i += 1;
            }
        }
        self.pos = i;
        let line = &buf[start..end];
        if line.contains(&0) {
            self.scratch.clear();
            self.scratch.extend(line.iter().copied().filter(|&b| b != 0));
            Some(&self.scratch)
        } else {
            Some(line)
        }
    }
}

/// Things that can be appended to a byte buffer; used by the `bcat!` macro
/// to build messages from mixed byte strings, `str`s and numbers.
pub trait Piece {
    fn put(&self, v: &mut Vec<u8>);
}
impl Piece for [u8] {
    fn put(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(self);
    }
}
impl<const N: usize> Piece for [u8; N] {
    fn put(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(self);
    }
}
impl Piece for Vec<u8> {
    fn put(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(self);
    }
}
impl Piece for str {
    fn put(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(self.as_bytes());
    }
}
impl Piece for String {
    fn put(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(self.as_bytes());
    }
}
impl<T: Piece + ?Sized> Piece for &T {
    fn put(&self, v: &mut Vec<u8>) {
        (**self).put(v);
    }
}
macro_rules! impl_piece_num {
    ($($t:ty),*) => {$(
        impl Piece for $t {
            fn put(&self, v: &mut Vec<u8>) {
                v.extend_from_slice(self.to_string().as_bytes());
            }
        }
    )*};
}
impl_piece_num!(u32, i32, i64, u64, usize);
impl Piece for bool {
    fn put(&self, v: &mut Vec<u8>) {
        v.push(if *self { b'1' } else { b'0' });
    }
}

/// Concatenate pieces into a `Vec<u8>`.
#[macro_export]
#[doc(hidden)]
macro_rules! bcat {
    ($($e:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut v: Vec<u8> = Vec::new();
        $( $crate::util::Piece::put(&$e, &mut v); )*
        v
    }};
}

/// `(s ? s : "(none)")`, the MAYBE macro of tex-file.c.
pub fn maybe(s: &Option<Vec<u8>>) -> &[u8] {
    match s {
        Some(v) => v,
        None => b"(none)",
    }
}

/// `(s ? s : "(nil)")`.
pub fn or_nil(s: Option<&[u8]>) -> &[u8] {
    s.unwrap_or(b"(nil)")
}
