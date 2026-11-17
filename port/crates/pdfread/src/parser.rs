//! Port of xpdf `Parser.h` / `Parser.cc`.

use std::rc::{Rc, Weak};

use crate::decrypt::{decrypt_bytes, FileKey};
use crate::error::{error, ErrorCategory};
use crate::lexer::{is_space, Lexer, EOF};
use crate::object::{Array, Dict, Object};
use crate::stream::Stream;
use crate::xref::XRef;

/// `objectRecursionLimit`: max number of nested objects.
pub const OBJECT_RECURSION_LIMIT: i32 = 500;

/// Mirrors `class Parser`.
pub struct Parser<'a> {
    xref: Option<&'a XRef>,
    xref_weak: Weak<XRef>,
    lexer: Lexer,
    allow_streams: bool,
    buf1: Object,
    buf2: Object,
    inline_img: i32,
}

impl<'a> Parser<'a> {
    /// `Parser(XRef *xrefA, Lexer *lexerA, GBool allowStreamsA)` — reads
    /// the first two tokens immediately.
    pub fn new(xref: Option<&'a XRef>, mut lexer: Lexer, allow_streams: bool) -> Parser<'a> {
        let buf1 = lexer.get_obj();
        let buf2 = lexer.get_obj();
        Parser {
            xref,
            xref_weak: xref.map(|x| x.weak()).unwrap_or_default(),
            lexer,
            allow_streams,
            buf1,
            buf2,
            inline_img: 0,
        }
    }

    /// `getObj(obj)` with default arguments.
    pub fn get_obj(&mut self) -> Object {
        self.get_obj_full(false, None, 0, 0, 0)
    }

    /// `getObj(obj, simpleOnly)`.
    pub fn get_obj_simple(&mut self, simple_only: bool) -> Object {
        self.get_obj_full(simple_only, None, 0, 0, 0)
    }

    /// `Parser::getObj(Object *obj, GBool simpleOnly, Guchar *fileKey,
    /// CryptAlgorithm encAlgorithm, int keyLength, int objNum, int objGen,
    /// int recursion)`.
    pub fn get_obj_full(
        &mut self,
        simple_only: bool,
        file_key: Option<&FileKey>,
        obj_num: i32,
        obj_gen: i32,
        recursion: i32,
    ) -> Object {
        // refill buffer after inline image data
        if self.inline_img == 2 {
            self.buf1 = self.lexer.get_obj();
            self.buf2 = self.lexer.get_obj();
            self.inline_img = 0;
        }

        // array
        if !simple_only && recursion < OBJECT_RECURSION_LIMIT && self.buf1.is_cmd_of("[") {
            self.shift();
            let mut arr = Array::new_weak(self.xref_weak.clone());
            while !self.buf1.is_cmd_of("]") && !self.buf1.is_eof() {
                let o = self.get_obj_full(false, file_key, obj_num, obj_gen, recursion + 1);
                arr.add(o);
            }
            if self.buf1.is_eof() {
                error(ErrorCategory::SyntaxError, self.get_pos(), "End of file inside array");
            }
            self.shift();
            Object::Array(Rc::new(arr))

        // dictionary or stream
        } else if !simple_only && recursion < OBJECT_RECURSION_LIMIT && self.buf1.is_cmd_of("<<") {
            self.shift();
            let mut dict = Dict::new_weak(self.xref_weak.clone());
            while !self.buf1.is_cmd_of(">>") && !self.buf1.is_eof() {
                if !self.buf1.is_name() {
                    error(ErrorCategory::SyntaxError, self.get_pos(), "Dictionary key must be a name object");
                    self.shift();
                } else {
                    let key: Vec<u8> = self.buf1.get_name().to_vec();
                    self.shift();
                    if self.buf1.is_eof() || self.buf1.is_error() {
                        break;
                    }
                    let v = self.get_obj_full(false, file_key, obj_num, obj_gen, recursion + 1);
                    dict.add(&key, v);
                }
            }
            if self.buf1.is_eof() {
                error(ErrorCategory::SyntaxError, self.get_pos(), "End of file inside dictionary");
            }
            // stream objects are not allowed inside content streams or
            // object streams
            if self.allow_streams && self.buf2.is_cmd_of("stream") {
                let dict = Rc::new(dict);
                match self.make_stream(dict, file_key, obj_num, obj_gen, recursion + 1) {
                    Some(s) => Object::Stream(Rc::new(s)),
                    None => Object::Error,
                }
            } else {
                self.shift();
                Object::Dict(Rc::new(dict))
            }

        // indirect reference or integer
        } else if let Object::Int(num) = self.buf1 {
            self.shift();
            if self.buf1.is_int() && self.buf2.is_cmd_of("R") {
                let gen = self.buf1.get_int();
                let r = if num >= 0 && gen >= 0 {
                    Object::new_ref(num, gen)
                } else {
                    error(
                        ErrorCategory::SyntaxError,
                        self.get_pos(),
                        "Negative number or generation in indirect reference",
                    );
                    Object::Error
                };
                self.shift();
                self.shift();
                r
            } else {
                Object::Int(num)
            }

        // string
        } else if let (Object::String(s), Some(k)) = (&self.buf1, file_key) {
            let s2 = decrypt_bytes(s, k, obj_num, obj_gen);
            self.shift();
            Object::String(Rc::from(s2))

        // simple object
        } else {
            let o = self.buf1.clone();
            self.shift();
            o
        }
    }

    /// `Parser::makeStream(...)`.
    fn make_stream(
        &mut self,
        dict: Rc<Dict>,
        file_key: Option<&FileKey>,
        obj_num: i32,
        obj_gen: i32,
        recursion: i32,
    ) -> Option<Stream> {
        // get stream start position
        self.lexer.skip_to_next_line();
        if !self.lexer.has_stream() {
            return None;
        }
        let pos = self.lexer.raw_get_pos();

        let mut have_length = false;
        let mut length: u64 = 0;

        // check for length in damaged file
        let stream_end = self.xref.and_then(|x| x.get_stream_end(pos));
        if let Some(end_pos) = stream_end {
            length = end_pos.wrapping_sub(pos);
            have_length = true;
        } else {
            // get length from the stream object
            let obj = dict.lookup_rec("Length", recursion);
            if let Object::Int(i) = obj {
                length = i as u32 as u64; // (GFileOffset)(Guint)obj.getInt()
                have_length = true;
            } else {
                error(ErrorCategory::SyntaxError, self.get_pos(), "Missing or invalid 'Length' attribute in stream");
            }
        }

        // in badly damaged PDF files, we can run off the end of the input
        // stream immediately after the "stream" token
        if !self.lexer.has_stream() {
            return None;
        }
        let base = self.lexer.data().clone();

        // 'Length' attribute is missing -- search for 'endstream'
        if !have_length {
            let mut found = false;
            if self.lexer.has_stream() {
                loop {
                    let c = self.lexer.raw_get_char();
                    if c == EOF {
                        break;
                    }
                    if c == b'e' as i32 {
                        let mut buf = [0u8; 8];
                        if self.lexer.raw_get_block(&mut buf) == 8 && &buf == b"ndstream" {
                            length = self.lexer.raw_get_pos() - 9 - pos;
                            found = true;
                            break;
                        }
                    }
                }
            }
            if !found {
                error(ErrorCategory::SyntaxError, self.get_pos(), "Couldn't find 'endstream' for stream");
                return None;
            }
        }

        // look for the 'endstream' marker
        if have_length {
            // skip over stream data
            self.lexer.set_pos(pos.wrapping_add(length));

            // check for 'endstream'
            let mut found = false;
            if self.lexer.has_stream() {
                // skip up to 100 whitespace chars
                let mut c = EOF;
                for _ in 0..100 {
                    c = self.lexer.raw_get_char();
                    if !is_space(c) {
                        break;
                    }
                }
                if c == b'e' as i32 {
                    let mut buf = [0u8; 8];
                    if self.lexer.raw_get_block(&mut buf) == 8 && &buf == b"ndstream" {
                        found = true;
                    }
                }
            }
            if !found {
                error(ErrorCategory::SyntaxError, self.get_pos(), "Missing 'endstream'");
                // kludge for broken PDF files: just add 5k to the length,
                // and hope it's enough
                length = length.wrapping_add(5000);
            }
        }

        // make new base stream
        let mut str = Stream::new_sub_stream(base, pos, length, dict.clone());

        // handle decryption
        if let Some(k) = file_key {
            // the 'Crypt' filter is used to mark unencrypted metadata
            // streams
            let mut encrypted = true;
            let obj = dict.lookup_rec("Filter", recursion);
            if obj.is_name_of("Crypt") {
                encrypted = false;
            } else if obj.is_array() && obj.array_get_length() >= 1 && obj.array_get(0).is_name_of("Crypt") {
                encrypted = false;
            }
            if encrypted {
                str.set_decrypt(k, obj_num, obj_gen);
            }
        }

        // get filters
        str.add_filters(recursion);

        Some(str)
    }

    /// `Parser::shift()`.
    fn shift(&mut self) {
        if self.inline_img > 0 {
            if self.inline_img < 2 {
                self.inline_img += 1;
            } else {
                // in a damaged content stream, if 'ID' shows up in the
                // middle of a dictionary, we need to reset
                self.inline_img = 0;
            }
        } else if self.buf2.is_cmd_of("ID") {
            self.lexer.skip_char(); // skip char after 'ID' command
            self.inline_img = 1;
        }
        self.buf1 = std::mem::take(&mut self.buf2);
        if self.inline_img > 0 {
            // don't buffer inline image data
            self.buf2 = Object::Null;
        } else {
            self.buf2 = self.lexer.get_obj();
        }
    }

    /// `getPos()`.
    pub fn get_pos(&self) -> i64 {
        self.lexer.get_pos()
    }

    /// Access the lexer (e.g. for `lexer->skipToEOF()`).
    pub fn lexer_mut(&mut self) -> &mut Lexer {
        &mut self.lexer
    }
}
