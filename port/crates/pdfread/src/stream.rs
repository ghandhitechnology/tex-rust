//! Port of the parts of xpdf `Stream.h` / `Stream.cc` used when reading
//! PDF files: the stream object created by `Parser::makeStream()`
//! (a `FileStream` sub-stream, optionally wrapped in a `DecryptStream`,
//! then wrapped in the filter chain built by `Stream::addFilters()`).
//!
//! Instead of a chain of stateful decoder objects, a [`Stream`] records
//! the base byte range, the decryption parameters and the filter list
//! (with parameters resolved at parse time, exactly when xpdf resolves
//! them).  Reading produces the same bytes xpdf's `reset()` +
//! `getChar()` loop would:
//!
//! * [`Stream::get_undecoded_data`] — `getUndecodedStream()`: the raw
//!   bytes after decryption but before any filter (`DecryptStream`
//!   overrides `getUndecodedStream()` to return itself).
//! * [`Stream::get_decoded_data`] — the fully decoded stream.

use std::rc::Rc;

use crate::decrypt::{decrypt_bytes, FileKey};
use crate::error::{error, ErrorCategory};
use crate::filters;
use crate::filters::PredictorParams;
use crate::lexer::EOF;
use crate::object::{Dict, Object};

/// Mirrors `enum StreamKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamKind {
    File,
    ASCIIHex,
    ASCII85,
    LZW,
    RunLength,
    CCITTFax,
    DCT,
    Flate,
    JBIG2,
    JPX,
    Weird,
}

/// One filter of the chain built by `Stream::makeFilter()`.
#[derive(Clone, Debug)]
pub enum Filter {
    ASCIIHex,
    ASCII85,
    LZW { pred: Option<PredictorParams>, early: i32 },
    RunLength,
    CCITTFax {
        encoding: i32,
        end_of_line: bool,
        byte_align: bool,
        columns: i32,
        rows: i32,
        end_of_block: bool,
        black: bool,
    },
    DCT { color_xform: i32 },
    Flate { pred: Option<PredictorParams> },
    JBIG2 { globals: Object },
    JPX,
    /// `EOFStream` (unknown filter name / bad filter entry): yields no
    /// data.
    EOF,
}

impl Filter {
    pub fn kind(&self) -> StreamKind {
        match self {
            Filter::ASCIIHex => StreamKind::ASCIIHex,
            Filter::ASCII85 => StreamKind::ASCII85,
            Filter::LZW { .. } => StreamKind::LZW,
            Filter::RunLength => StreamKind::RunLength,
            Filter::CCITTFax { .. } => StreamKind::CCITTFax,
            Filter::DCT { .. } => StreamKind::DCT,
            Filter::Flate { .. } => StreamKind::Flate,
            Filter::JBIG2 { .. } => StreamKind::JBIG2,
            Filter::JPX => StreamKind::JPX,
            Filter::EOF => StreamKind::Weird,
        }
    }
}

/// Returned when the filter chain contains an image codec that this crate
/// does not implement (CCITTFax, DCT, JBIG2, JPX).  pdfTeX only decodes
/// streams for page content arrays and embedded Type 1 font files, where
/// these never legitimately occur.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedFilter(pub StreamKind);

impl std::fmt::Display for UnsupportedFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unsupported stream filter {:?}", self.0)
    }
}

impl std::error::Error for UnsupportedFilter {}

/// A PDF stream object (xpdf: the `Stream*` held by an `objStream`
/// Object).
pub struct Stream {
    dict: Rc<Dict>,
    data: Rc<[u8]>,
    start: u64,
    length: u64,
    decrypt: Option<(FileKey, i32, i32)>,
    filters: Vec<Filter>,
}

impl Stream {
    /// `baseStr->makeSubStream(start, gTrue, length, dict)`.
    pub(crate) fn new_sub_stream(data: Rc<[u8]>, start: u64, length: u64, dict: Rc<Dict>) -> Stream {
        Stream { dict, data, start, length, decrypt: None, filters: Vec::new() }
    }

    /// `new DecryptStream(str, fileKey, encAlgorithm, keyLength, objNum, objGen)`.
    pub(crate) fn set_decrypt(&mut self, key: &FileKey, obj_num: i32, obj_gen: i32) {
        self.decrypt = Some((key.clone(), obj_num, obj_gen));
    }

    /// `Stream::addFilters(Object *dict, int recursion)`.
    pub(crate) fn add_filters(&mut self, recursion: i32) {
        let dict = self.dict.clone();
        let mut obj = dict.lookup_rec("Filter", recursion);
        if obj.is_null() {
            obj = dict.lookup_rec("F", recursion);
        }
        let mut params = dict.lookup_rec("DecodeParms", recursion);
        if params.is_null() {
            params = dict.lookup_rec("DP", recursion);
        }
        let mut ok = true;
        let pos = self.start as i64;
        if obj.is_name() {
            let f = make_filter(obj.get_name(), &params, recursion, &mut ok, pos);
            if let Some(f) = f {
                self.filters.push(f);
            }
        } else if obj.is_array() {
            let n = obj.array_get_length();
            let mut i = 0;
            while ok && i < n {
                let obj2 = obj.array_get_rec(i, recursion);
                let params2 = if params.is_array() && i < params.array_get_length() {
                    params.array_get_rec(i, recursion)
                } else {
                    Object::Null
                };
                if obj2.is_name() {
                    let f = make_filter(obj2.get_name(), &params2, recursion, &mut ok, pos);
                    if let Some(f) = f {
                        self.filters.push(f);
                    }
                } else {
                    error(ErrorCategory::SyntaxError, pos, "Bad filter name");
                    self.filters.push(Filter::EOF);
                    ok = false;
                }
                i += 1;
            }
        } else if !obj.is_null() {
            error(ErrorCategory::SyntaxError, pos, "Bad 'Filter' attribute in stream");
        }
    }

    /// `getDict()`.
    pub fn get_dict(&self) -> &Rc<Dict> {
        &self.dict
    }

    /// `getKind()` of the outermost stream.
    pub fn get_kind(&self) -> StreamKind {
        match self.filters.last() {
            Some(f) => f.kind(),
            None => {
                if self.decrypt.is_some() {
                    StreamKind::Weird
                } else {
                    StreamKind::File
                }
            }
        }
    }

    /// The filter chain, innermost first.
    pub fn get_filters(&self) -> &[Filter] {
        &self.filters
    }

    /// Is this stream decrypted on read?
    pub fn is_encrypted(&self) -> bool {
        self.decrypt.is_some()
    }

    /// `getBaseStream()->getStart()`: file offset of the stream data.
    pub fn get_start(&self) -> u64 {
        self.start
    }

    /// The length used for the sub-stream (from /Length, from the
    /// repaired xref's `endstream` table, or found by scanning; may be
    /// larger than the file).
    pub fn get_length(&self) -> u64 {
        self.length
    }

    /// The raw bytes of the sub-stream as stored in the file (before
    /// decryption).
    pub fn get_raw_data(&self) -> &[u8] {
        let len = self.data.len() as u64;
        let a = self.start.min(len);
        let b = self.start.saturating_add(self.length).min(len);
        &self.data[a as usize..b as usize]
    }

    /// `getUndecodedStream()` contents: raw bytes, decrypted if the file
    /// is encrypted.
    pub fn get_undecoded_data(&self) -> Vec<u8> {
        let raw = self.get_raw_data();
        match &self.decrypt {
            Some((k, n, g)) => decrypt_bytes(raw, k, *n, *g),
            None => raw.to_vec(),
        }
    }

    /// All bytes `reset()` + `getChar()` deliver on the decoded stream.
    pub fn get_decoded_data(&self) -> Result<Vec<u8>, UnsupportedFilter> {
        let mut data = self.get_undecoded_data();
        for f in &self.filters {
            data = apply_filter(f, &data)?;
        }
        Ok(data)
    }

    /// An xpdf-style reader over the undecoded stream
    /// (`getUndecodedStream()` followed by `reset()`).
    pub fn get_undecoded_stream(&self) -> StreamReader {
        StreamReader::new(self.get_undecoded_data())
    }

    /// An xpdf-style reader over the decoded stream (`reset()` on the
    /// stream itself).
    pub fn reader(&self) -> Result<StreamReader, UnsupportedFilter> {
        Ok(StreamReader::new(self.get_decoded_data()?))
    }
}

fn apply_filter(f: &Filter, data: &[u8]) -> Result<Vec<u8>, UnsupportedFilter> {
    Ok(match f {
        Filter::ASCIIHex => filters::ascii_hex_decode(data),
        Filter::ASCII85 => filters::ascii85_decode(data),
        Filter::LZW { pred, early } => {
            let raw = filters::lzw_decode(data, *early);
            match pred {
                Some(p) => filters::apply_predictor(&raw, p),
                None => raw,
            }
        }
        Filter::RunLength => filters::run_length_decode(data),
        Filter::Flate { pred } => {
            let raw = filters::flate_decode(data);
            match pred {
                Some(p) => filters::apply_predictor(&raw, p),
                None => raw,
            }
        }
        Filter::EOF => Vec::new(),
        other => return Err(UnsupportedFilter(other.kind())),
    })
}

fn pred_params(pred: i32, columns: i32, colors: i32, bits: i32) -> Option<PredictorParams> {
    if pred == 1 {
        return None;
    }
    let p = PredictorParams { predictor: pred, width: columns, n_comps: colors, n_bits: bits };
    if p.is_ok() {
        Some(p)
    } else {
        None
    }
}

/// `Stream::makeFilter(char *name, Stream *str, Object *params, ...)`.
/// Returns `None` for the "Crypt" filter (handled in makeStream).
fn make_filter(name: &[u8], params: &Object, recursion: i32, ok: &mut bool, pos: i64) -> Option<Filter> {
    let int_param = |key: &str, dflt: i32| -> i32 {
        if params.is_dict() {
            let o = params.dict_lookup_rec(key, recursion);
            if let Object::Int(i) = o {
                return i;
            }
        }
        dflt
    };
    let bool_param = |key: &str, dflt: bool| -> bool {
        if params.is_dict() {
            let o = params.dict_lookup_rec(key, recursion);
            if let Object::Bool(b) = o {
                return b;
            }
        }
        dflt
    };
    match name {
        b"ASCIIHexDecode" | b"AHx" => Some(Filter::ASCIIHex),
        b"ASCII85Decode" | b"A85" => Some(Filter::ASCII85),
        b"LZWDecode" | b"LZW" => {
            let pred = int_param("Predictor", 1);
            let columns = int_param("Columns", 1);
            let colors = int_param("Colors", 1);
            let bits = int_param("BitsPerComponent", 8);
            let early = int_param("EarlyChange", 1);
            Some(Filter::LZW { pred: pred_params(pred, columns, colors, bits), early })
        }
        b"RunLengthDecode" | b"RL" => Some(Filter::RunLength),
        b"CCITTFaxDecode" | b"CCF" => Some(Filter::CCITTFax {
            encoding: int_param("K", 0),
            end_of_line: bool_param("EndOfLine", false),
            byte_align: bool_param("EncodedByteAlign", false),
            columns: int_param("Columns", 1728),
            rows: int_param("Rows", 0),
            end_of_block: bool_param("EndOfBlock", true),
            black: bool_param("BlackIs1", false),
        }),
        b"DCTDecode" | b"DCT" => Some(Filter::DCT { color_xform: int_param("ColorTransform", -1) }),
        b"FlateDecode" | b"Fl" => {
            let pred = int_param("Predictor", 1);
            let columns = int_param("Columns", 1);
            let colors = int_param("Colors", 1);
            let bits = int_param("BitsPerComponent", 8);
            Some(Filter::Flate { pred: pred_params(pred, columns, colors, bits) })
        }
        b"JBIG2Decode" => {
            let globals = if params.is_dict() { params.dict_lookup_rec("JBIG2Globals", recursion) } else { Object::None };
            Some(Filter::JBIG2 { globals })
        }
        b"JPXDecode" => Some(Filter::JPX),
        b"Crypt" => None,
        _ => {
            error(
                ErrorCategory::SyntaxError,
                pos,
                &format!("Unknown filter '{}'", String::from_utf8_lossy(name)),
            );
            *ok = false;
            Some(Filter::EOF)
        }
    }
}

/// A `reset()`-ed stream: `getChar()`, `lookChar()`, `getBlock()`,
/// `getPos()`, `discardChars()` over a byte buffer.
#[derive(Clone)]
pub struct StreamReader {
    data: Vec<u8>,
    pos: usize,
}

impl StreamReader {
    pub fn new(data: Vec<u8>) -> StreamReader {
        StreamReader { data, pos: 0 }
    }
    /// `reset()`
    pub fn reset(&mut self) {
        self.pos = 0;
    }
    /// `getChar()`: next byte or `EOF` (-1).
    #[inline]
    pub fn get_char(&mut self) -> i32 {
        if self.pos < self.data.len() {
            let c = self.data[self.pos];
            self.pos += 1;
            c as i32
        } else {
            EOF
        }
    }
    /// `lookChar()`
    #[inline]
    pub fn look_char(&self) -> i32 {
        if self.pos < self.data.len() {
            self.data[self.pos] as i32
        } else {
            EOF
        }
    }
    /// `getBlock(char *blk, int size)`
    pub fn get_block(&mut self, blk: &mut [u8]) -> usize {
        let n = blk.len().min(self.data.len() - self.pos);
        blk[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        n
    }
    /// `discardChars(Guint n)`
    pub fn discard_chars(&mut self, n: usize) -> usize {
        let k = n.min(self.data.len() - self.pos);
        self.pos += k;
        k
    }
    /// Number of bytes read so far.
    pub fn get_pos(&self) -> usize {
        self.pos
    }
    /// The whole buffer.
    pub fn data(&self) -> &[u8] {
        &self.data
    }
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }
}
