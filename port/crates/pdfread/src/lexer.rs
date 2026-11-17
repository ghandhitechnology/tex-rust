//! Port of xpdf `Lexer.h` / `Lexer.cc`.
//!
//! The xpdf lexer reads from a `Stream` (a `FileStream` sub-stream
//! starting at some file offset, or an `EmbedStream` window of a decoded
//! object stream).  Here the input is a byte buffer plus a current
//! position and an end limit, which behaves identically for every stream
//! type the parser uses:
//!
//! * `FileStream` (unlimited) — `data` = whole file, `end` = file length;
//!   positions are absolute file offsets, like `FileStream::getPos()`.
//! * `EmbedStream` (limited window of a decoded object stream) — `data` =
//!   decoded bytes, `[pos, end)` = the window.
//!
//! As in xpdf, when `getChar()` hits EOF the current stream is dropped
//! (`curStr.free()`): afterwards `getPos()` returns -1, `lookChar()`
//! returns EOF and `getStream()` returns "NULL" (see [`Lexer::has_stream`]).

use std::rc::Rc;

use crate::error::{error, ErrorCategory};
use crate::object::Object;

pub const EOF: i32 = -1;

/// `tokBufSize` from Lexer.h.
pub const TOK_BUF_SIZE: usize = 128;

/// A '1' in this array means the character is white space.  A '1' or
/// '2' means the character ends a name or command.  (`specialChars[]`)
pub(crate) static SPECIAL_CHARS: [u8; 256] = [
    1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 1, 1, 0, 0, // 0x
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 1x
    1, 0, 0, 0, 0, 2, 0, 0, 2, 2, 0, 0, 0, 0, 0, 2, // 2x
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, // 3x
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 4x
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 0, // 5x
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 6x
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 0, // 7x
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 8x
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 9x
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // ax
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // bx
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // cx
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // dx
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // ex
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // fx
];

#[inline]
fn special(c: i32) -> u8 {
    if (0..=255).contains(&c) {
        SPECIAL_CHARS[c as usize]
    } else {
        0
    }
}

#[inline]
fn is_digit(c: i32) -> bool {
    (b'0' as i32..=b'9' as i32).contains(&c)
}

/// `Lexer::isSpace(int c)`.
pub fn is_space(c: i32) -> bool {
    (0..=0xff).contains(&c) && SPECIAL_CHARS[c as usize] == 1
}

/// Mirrors `class Lexer` (single-stream form).
pub struct Lexer {
    data: Rc<[u8]>,
    pos: u64,
    end: u64,
    dead: bool,
}

impl Lexer {
    /// Lexer over `data[start..end]` (`end` = `None` means "to the end of
    /// the data", i.e. an unlimited stream).
    pub fn new(data: Rc<[u8]>, start: u64, end: Option<u64>) -> Lexer {
        let len = data.len() as u64;
        let end = end.unwrap_or(len).min(len);
        Lexer { data, pos: start, end, dead: false }
    }

    /// The underlying buffer (the whole file for file-based lexers).
    pub(crate) fn data(&self) -> &Rc<[u8]> {
        &self.data
    }

    /// `Lexer::getChar()` — frees the stream on EOF.
    #[inline]
    pub fn get_char(&mut self) -> i32 {
        if self.dead {
            return EOF;
        }
        if self.pos < self.end {
            let c = self.data[self.pos as usize] as i32;
            self.pos += 1;
            c
        } else {
            self.dead = true;
            EOF
        }
    }

    /// `Lexer::lookChar()`.
    #[inline]
    pub fn look_char(&self) -> i32 {
        if self.dead {
            return EOF;
        }
        if self.pos < self.end {
            self.data[self.pos as usize] as i32
        } else {
            EOF
        }
    }

    /// `Lexer::getPos()`.
    pub fn get_pos(&self) -> i64 {
        if self.dead {
            -1
        } else {
            self.pos as i64
        }
    }

    /// `Lexer::setPos()` (ignored once the stream has been freed).
    pub fn set_pos(&mut self, pos: u64) {
        if !self.dead {
            self.pos = pos;
        }
    }

    /// `getStream() != NULL`.
    pub fn has_stream(&self) -> bool {
        !self.dead
    }

    /// `getStream()->getChar()` — direct stream access that does not free
    /// the stream at EOF.
    pub(crate) fn raw_get_char(&mut self) -> i32 {
        if self.pos < self.end {
            let c = self.data[self.pos as usize] as i32;
            self.pos += 1;
            c
        } else {
            EOF
        }
    }

    /// `getStream()->getBlock(buf, n)`.
    pub(crate) fn raw_get_block(&mut self, buf: &mut [u8]) -> usize {
        let mut n = 0;
        while n < buf.len() && self.pos < self.end {
            buf[n] = self.data[self.pos as usize];
            self.pos += 1;
            n += 1;
        }
        n
    }

    /// `getStream()->getPos()`.
    pub(crate) fn raw_get_pos(&self) -> u64 {
        self.pos
    }

    /// `Lexer::skipChar()`.
    pub fn skip_char(&mut self) {
        self.get_char();
    }

    /// `Lexer::skipToNextLine()`.
    pub fn skip_to_next_line(&mut self) {
        loop {
            let c = self.get_char();
            if c == EOF || c == b'\n' as i32 {
                return;
            }
            if c == b'\r' as i32 {
                if self.look_char() == b'\n' as i32 {
                    self.get_char();
                }
                return;
            }
        }
    }

    /// `Lexer::skipToEOF()`.
    pub fn skip_to_eof(&mut self) {
        while self.get_char() != EOF {}
    }

    /// `Lexer::getObj(Object *obj)`.
    pub fn get_obj(&mut self) -> Object {
        // skip whitespace and comments
        let mut comment = false;
        let mut c;
        loop {
            c = self.get_char();
            if c == EOF {
                return Object::EOF;
            }
            if comment {
                if c == b'\r' as i32 || c == b'\n' as i32 {
                    comment = false;
                }
            } else if c == b'%' as i32 {
                comment = true;
            } else if special(c) != 1 {
                break;
            }
        }

        // start reading token
        match c as u8 {
            // number
            b'0'..=b'9' | b'+' | b'-' | b'.' => self.lex_number(c),
            // string
            b'(' => self.lex_string(),
            // name
            b'/' => self.lex_name(),
            // array punctuation
            b'[' | b']' => Object::new_cmd(&[c as u8]),
            // hex string or dict punctuation
            b'<' => {
                if self.look_char() == b'<' as i32 {
                    self.get_char();
                    Object::new_cmd(b"<<")
                } else {
                    self.lex_hex_string()
                }
            }
            // dict punctuation
            b'>' => {
                if self.look_char() == b'>' as i32 {
                    self.get_char();
                    Object::new_cmd(b">>")
                } else {
                    error(ErrorCategory::SyntaxError, self.get_pos(), "Illegal character '>'");
                    Object::Error
                }
            }
            // error
            b')' | b'{' | b'}' => {
                error(
                    ErrorCategory::SyntaxError,
                    self.get_pos(),
                    &format!("Illegal character '{}'", c as u8 as char),
                );
                Object::Error
            }
            // command
            _ => {
                let mut tok: Vec<u8> = Vec::with_capacity(16);
                tok.push(c as u8);
                let mut n = 1usize;
                loop {
                    let c = self.look_char();
                    if c == EOF || special(c) != 0 {
                        break;
                    }
                    self.get_char();
                    n += 1;
                    if n == TOK_BUF_SIZE {
                        error(ErrorCategory::SyntaxError, self.get_pos(), "Command token too long");
                        break;
                    }
                    tok.push(c as u8);
                }
                match &tok[..] {
                    b"true" => Object::Bool(true),
                    b"false" => Object::Bool(false),
                    b"null" => Object::Null,
                    _ => Object::new_cmd(&tok),
                }
            }
        }
    }

    // Number lexing.  Adobe's number lexer has some "interesting"
    // behavior (comments copied from Lexer.cc):
    // "--123" is interpreted as 0
    // "--123.4" is interpreted as -123.4 [I've seen this in the wild]
    // "50-100" is interpreted as 50 [I've seen this in the wild]
    // "50--100" is interpreted as 50
    // "50-100.0" is an error -- but older versions of Acrobat may
    //   have interpreted it as 50100.0 (?)
    // "50--100.0" is an error -- but older versions of Acrobat may
    //   have interpreted it as 50100.0 (?)
    // "50.0-100" is interpreted as 50.0 (or maybe 50.0100?)
    // "50.0--100" is interpreted as 50.0 (or maybe 50.0100?)
    // "-50-100" is interpreted as -50
    // "-" is interpreted as 0
    // "-." is interpreted as 0.0
    fn lex_number(&mut self, first: i32) -> Object {
        let mut neg = false;
        let mut double_minus = false;
        let mut xi: i32 = 0;
        let mut xf: f64 = 0.0;
        let mut c = first;
        let mut real = false;
        if c == b'+' as i32 {
            // just ignore it
        } else if c == b'-' as i32 {
            neg = true;
            if self.look_char() == b'-' as i32 {
                double_minus = true;
                loop {
                    self.get_char();
                    if self.look_char() != b'-' as i32 {
                        break;
                    }
                }
            }
        } else if c == b'.' as i32 {
            real = true;
        } else {
            xi = c - b'0' as i32;
            xf = xi as f64;
        }
        if !real {
            loop {
                c = self.look_char();
                if is_digit(c) {
                    self.get_char();
                    // int overflow wraps (two's complement), as in the
                    // compiled C++
                    xi = xi.wrapping_mul(10).wrapping_add(c - b'0' as i32);
                    if xf < 1e20 {
                        xf = xf * 10.0 + (c - b'0' as i32) as f64;
                    }
                } else if c == b'.' as i32 {
                    self.get_char();
                    real = true;
                    break;
                } else {
                    break;
                }
            }
        }
        if !real {
            loop {
                c = self.look_char();
                if c == b'-' as i32 || is_digit(c) {
                    self.get_char();
                } else {
                    break;
                }
            }
            if neg {
                xi = xi.wrapping_neg();
            }
            if double_minus {
                xi = 0;
            }
            return Object::Int(xi);
        }
        // doReal:
        let mut scale = 0.1f64;
        loop {
            c = self.look_char();
            if c == b'-' as i32 {
                error(ErrorCategory::SyntaxWarning, self.get_pos(), "Badly formatted number");
                self.get_char();
                continue;
            }
            if !is_digit(c) {
                break;
            }
            self.get_char();
            xf += scale * (c - b'0' as i32) as f64;
            scale *= 0.1;
        }
        loop {
            c = self.look_char();
            if c == b'-' as i32 || is_digit(c) {
                self.get_char();
            } else {
                break;
            }
        }
        if neg {
            xf = -xf;
        }
        Object::Real(xf)
    }

    fn lex_string(&mut self) -> Object {
        let mut s: Vec<u8> = Vec::new();
        let mut num_paren = 1;
        let mut done = false;
        loop {
            let mut c2: i32 = EOF;
            let mut c = self.get_char();
            match c {
                EOF => {
                    error(ErrorCategory::SyntaxError, self.get_pos(), "Unterminated string");
                    done = true;
                }
                0x28 /* ( */ => {
                    num_paren += 1;
                    c2 = c;
                }
                0x29 /* ) */ => {
                    num_paren -= 1;
                    if num_paren == 0 {
                        done = true;
                    } else {
                        c2 = c;
                    }
                }
                0x0d /* \r */ => {
                    // The PDF spec says that any literal end-of-line
                    // sequence (LF, CR, CR+LF) is translated to a single
                    // LF char.
                    c = self.look_char();
                    if c == b'\n' as i32 {
                        self.get_char();
                    }
                    c2 = b'\n' as i32;
                }
                0x5c /* \ */ => {
                    c = self.get_char();
                    match c {
                        0x6e /* n */ => c2 = b'\n' as i32,
                        0x72 /* r */ => c2 = b'\r' as i32,
                        0x74 /* t */ => c2 = b'\t' as i32,
                        0x62 /* b */ => c2 = 0x08,
                        0x66 /* f */ => c2 = 0x0c,
                        0x5c | 0x28 | 0x29 => c2 = c,
                        0x30..=0x37 => {
                            c2 = c - b'0' as i32;
                            c = self.look_char();
                            if (b'0' as i32..=b'7' as i32).contains(&c) {
                                self.get_char();
                                c2 = (c2 << 3) + (c - b'0' as i32);
                                c = self.look_char();
                                if (b'0' as i32..=b'7' as i32).contains(&c) {
                                    self.get_char();
                                    c2 = (c2 << 3) + (c - b'0' as i32);
                                }
                            }
                        }
                        0x0d => {
                            c = self.look_char();
                            if c == b'\n' as i32 {
                                self.get_char();
                            }
                        }
                        0x0a => {}
                        EOF => {
                            error(ErrorCategory::SyntaxError, self.get_pos(), "Unterminated string");
                            done = true;
                        }
                        _ => c2 = c,
                    }
                }
                _ => c2 = c,
            }
            if c2 != EOF {
                // (char)c2: octal escapes up to \777 keep the low 8 bits
                s.push(c2 as u8);
            }
            if done {
                break;
            }
        }
        Object::String(Rc::from(s))
    }

    fn lex_name(&mut self) -> Object {
        let mut s: Vec<u8> = Vec::new();
        let mut invalid = false;
        loop {
            let mut c = self.look_char();
            if c == EOF || special(c) != 0 {
                break;
            }
            self.get_char();
            'esc: {
                if c == b'#' as i32 {
                    let c2 = self.look_char();
                    if let Some(v) = hex_val(c2) {
                        c = v;
                    } else {
                        error(ErrorCategory::SyntaxError, self.get_pos(), "Invalid hex escape in name");
                        break 'esc;
                    }
                    self.get_char();
                    let c2 = self.look_char();
                    if let Some(v) = hex_val(c2) {
                        c = (c << 4) + v;
                    } else {
                        error(ErrorCategory::SyntaxError, self.get_pos(), "Invalid hex escape in name");
                        break 'esc;
                    }
                    self.get_char();
                    if c == 0 {
                        invalid = true;
                    }
                }
            }
            // notEscChar:
            s.push(c as u8);
        }
        if invalid {
            error(ErrorCategory::SyntaxError, self.get_pos(), "Null character in name");
            return Object::Error;
        }
        // initName(tokBuf) / initName(s->getCString()): C string, so a NUL
        // byte stored by a half-valid escape like "#0G" truncates the name
        Object::new_name(&s)
    }

    fn lex_hex_string(&mut self) -> Object {
        let mut s: Vec<u8> = Vec::new();
        let mut m = 0;
        let mut c2: i32 = 0;
        let mut n_errors = 0;
        while n_errors < 100 {
            let c = self.get_char();
            if c == b'>' as i32 {
                break;
            } else if c == EOF {
                error(ErrorCategory::SyntaxError, self.get_pos(), "Unterminated hex string");
                break;
            } else if special(c) != 1 {
                c2 <<= 4;
                if let Some(v) = hex_val(c) {
                    c2 += v;
                } else {
                    error(
                        ErrorCategory::SyntaxError,
                        self.get_pos(),
                        &format!("Illegal character <{:02x}> in hex string", c),
                    );
                    n_errors += 1;
                }
                m += 1;
                if m == 2 {
                    s.push(c2 as u8);
                    c2 = 0;
                    m = 0;
                }
            }
        }
        if m == 1 {
            s.push((c2 << 4) as u8);
        }
        Object::String(Rc::from(s))
    }
}

#[inline]
fn hex_val(c: i32) -> Option<i32> {
    if (b'0' as i32..=b'9' as i32).contains(&c) {
        Some(c - b'0' as i32)
    } else if (b'A' as i32..=b'F' as i32).contains(&c) {
        Some(c - b'A' as i32 + 10)
    } else if (b'a' as i32..=b'f' as i32).contains(&c) {
        Some(c - b'a' as i32 + 10)
    } else {
        None
    }
}
