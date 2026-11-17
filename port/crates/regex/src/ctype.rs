//! `<ctype.h>` as seen by the regex engine inside TeX Live's `pdftex.dll`.
//!
//! pdfTeX never calls `setlocale`, so the C runtime stays in the "C" locale.
//! In the Windows C runtime's "C" locale only the 7-bit ASCII range is
//! classified; bytes 0x80..=0xFF belong to no class and are not case-mapped.
//! The classification below is the standard C-locale table for 0..=127.
//! (`isblank` is either the CRT function or the fallback macro in
//! regex_internal.h, `(ch) == ' ' || (ch) == '\t'`; both agree.)

#[inline]
pub(crate) fn isupper(c: i32) -> bool {
    (b'A' as i32..=b'Z' as i32).contains(&c)
}
#[inline]
pub(crate) fn islower(c: i32) -> bool {
    (b'a' as i32..=b'z' as i32).contains(&c)
}
#[inline]
pub(crate) fn isalpha(c: i32) -> bool {
    isupper(c) || islower(c)
}
#[inline]
pub(crate) fn isdigit(c: i32) -> bool {
    (b'0' as i32..=b'9' as i32).contains(&c)
}
#[inline]
pub(crate) fn isalnum(c: i32) -> bool {
    isalpha(c) || isdigit(c)
}
#[inline]
pub(crate) fn isxdigit(c: i32) -> bool {
    isdigit(c) || (b'a' as i32..=b'f' as i32).contains(&c) || (b'A' as i32..=b'F' as i32).contains(&c)
}
#[inline]
pub(crate) fn isspace(c: i32) -> bool {
    c == b' ' as i32 || (9..=13).contains(&c)
}
#[inline]
pub(crate) fn isblank(c: i32) -> bool {
    c == b' ' as i32 || c == b'\t' as i32
}
#[inline]
pub(crate) fn iscntrl(c: i32) -> bool {
    (0..=31).contains(&c) || c == 127
}
#[inline]
pub(crate) fn isprint(c: i32) -> bool {
    (32..=126).contains(&c)
}
#[inline]
pub(crate) fn isgraph(c: i32) -> bool {
    (33..=126).contains(&c)
}
#[inline]
pub(crate) fn ispunct(c: i32) -> bool {
    isgraph(c) && !isalnum(c)
}
#[inline]
pub(crate) fn toupper(c: i32) -> i32 {
    if islower(c) {
        c - 32
    } else {
        c
    }
}
#[inline]
pub(crate) fn tolower(c: i32) -> i32 {
    if isupper(c) {
        c + 32
    } else {
        c
    }
}
