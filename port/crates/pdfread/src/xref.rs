//! Port of xpdf `XRef.h` / `XRef.cc`: classic xref tables, xref streams
//! (with `/Index`), hybrid-reference files (`/XRefStm`), `/Prev` chains
//! with loop detection, object streams (with xpdf's 128-entry object
//! stream cache), the 16-entry object cache, and reconstruction of
//! damaged files (`constructXRef`).

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use crate::decrypt::{CryptAlgorithm, FileKey};
use crate::error::{codes, error, ErrorCategory};
use crate::lexer::{is_space, Lexer, EOF};
use crate::object::{Dict, Object};
use crate::parser::{Parser, OBJECT_RECURSION_LIMIT};
use crate::stream::{Stream, StreamReader};

/// `xrefSearchSize`: read this many bytes at end of file to look for
/// 'startxref'.
const XREF_SEARCH_SIZE: usize = 1024;

const PERM_PRINT: i32 = 1 << 2;
const PERM_CHANGE: i32 = 1 << 3;
const PERM_COPY: i32 = 1 << 4;
const PERM_NOTES: i32 = 1 << 5;
const DEF_PERM_FLAGS: i32 = 0xfffc;

/// `xrefCacheSize`
const XREF_CACHE_SIZE: usize = 16;
/// `objStrCacheSize`
const OBJ_STR_CACHE_SIZE: usize = 128;
/// `objStrCacheTimeout`
const OBJ_STR_CACHE_TIMEOUT: u32 = 1000;

/// Entries beyond this are treated as an allocation failure (xpdf would
/// abort with "Out of memory" in `greallocn` long before reaching sizes
/// that matter here).
const MAX_ENTRIES: i64 = 1 << 27;

const GFILEOFFSET_MAX: i64 = i64::MAX;

/// Mirrors `enum XRefEntryType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XRefEntryType {
    Free,
    Uncompressed,
    Compressed,
}

/// Mirrors `struct XRefEntry`.  For compressed entries `offset` is the
/// object stream number and `gen` the index inside the object stream.
#[derive(Clone, Copy, Debug)]
pub struct XRefEntry {
    pub offset: i64,
    pub gen: i32,
    pub type_: XRefEntryType,
}

impl XRefEntry {
    const UNSET: XRefEntry = XRefEntry { offset: -1, gen: 0, type_: XRefEntryType::Free };
}

// ------------------------------------------------------------------------
// XRefPosSet
// ------------------------------------------------------------------------

/// Mirrors `class XRefPosSet` (sorted set of xref section offsets).
struct XRefPosSet {
    tab: Vec<i64>,
}

impl XRefPosSet {
    fn new() -> Self {
        XRefPosSet { tab: Vec::new() }
    }
    fn find(&self, pos: i64) -> usize {
        let mut a: isize = -1;
        let mut b: isize = self.tab.len() as isize;
        // invariant: tab[a] < pos < tab[b]
        while b - a > 1 {
            let m = (a + b) / 2;
            if self.tab[m as usize] < pos {
                a = m;
            } else if self.tab[m as usize] > pos {
                b = m;
            } else {
                return m as usize;
            }
        }
        b as usize
    }
    fn add(&mut self, pos: i64) {
        let i = self.find(pos);
        if i < self.tab.len() && self.tab[i] == pos {
            return;
        }
        self.tab.insert(i, pos);
    }
    fn check(&self, pos: i64) -> bool {
        let i = self.find(pos);
        i < self.tab.len() && self.tab[i] == pos
    }
}

// ------------------------------------------------------------------------
// ObjectStream
// ------------------------------------------------------------------------

/// Mirrors `class ObjectStream`.
struct ObjectStream {
    obj_str_num: i32,
    objs: Vec<Object>,
    obj_nums: Vec<i32>,
}

impl ObjectStream {
    /// `ObjectStream::ObjectStream(XRef *xref, int objStrNumA, int
    /// recursion)`; returns `None` when `!isOk()`.
    fn new(xref: &XRef, obj_str_num: i32, recursion: i32) -> Option<ObjectStream> {
        let obj_str = xref.fetch_rec(obj_str_num, 0, recursion);
        let stream = obj_str.get_stream()?;
        let dict = stream.get_dict();
        let n_objects = match dict.lookup_rec("N", recursion) {
            Object::Int(i) => i,
            _ => return None,
        };
        if n_objects <= 0 {
            return None;
        }
        let first = match dict.lookup_rec("First", recursion) {
            Object::Int(i) => i,
            _ => return None,
        };
        if first < 0 {
            return None;
        }
        // this is an arbitrary limit to avoid integer overflow problems in
        // the 'new Object[nObjects]' call (Acrobat apparently limits
        // object streams to 100-200 objects)
        if n_objects > 1_000_000 {
            error(ErrorCategory::SyntaxError, -1, "Too many objects in an object stream");
            return None;
        }
        let n = n_objects as usize;
        let data: Rc<[u8]> = Rc::from(stream.get_decoded_data().unwrap_or_default());
        let len = data.len() as u64;

        // parse the header: object numbers and offsets (EmbedStream
        // limited to 'first' bytes)
        let mut obj_nums = vec![0i32; n];
        let mut offsets = vec![0i32; n];
        {
            let lexer = Lexer::new(data.clone(), 0, Some(first as u64));
            let mut parser = Parser::new(Some(xref), lexer, false);
            for i in 0..n {
                let obj1 = parser.get_obj_simple(true);
                let obj2 = parser.get_obj_simple(true);
                if !obj1.is_int() || !obj2.is_int() {
                    return None;
                }
                obj_nums[i] = obj1.get_int();
                offsets[i] = obj2.get_int();
                if obj_nums[i] < 0 || offsets[i] < 0 || (i > 0 && offsets[i] < offsets[i - 1]) {
                    return None;
                }
            }
        }
        // after lexer->skipToEOF() the object stream has been read up to
        // min(first, len); then discardChars(offsets[0])
        let base = (first as u64).min(len);

        // parse the objects
        let mut objs = Vec::with_capacity(n);
        for i in 0..n {
            let s = (base + offsets[i] as u64).min(len);
            let e = if i == n - 1 { len } else { (base + offsets[i + 1] as u64).min(len) };
            let lexer = Lexer::new(data.clone(), s, Some(e));
            let mut parser = Parser::new(Some(xref), lexer, false);
            objs.push(parser.get_obj());
        }
        Some(ObjectStream { obj_str_num, objs, obj_nums })
    }

    /// `ObjectStream::getObject(int objIdx, int objNum, Object *obj)`.
    fn get_object(&self, obj_idx: i32, obj_num: i32) -> Object {
        if obj_idx < 0 || obj_idx as usize >= self.objs.len() || obj_num != self.obj_nums[obj_idx as usize] {
            Object::Null
        } else {
            self.objs[obj_idx as usize].clone()
        }
    }
}

// ------------------------------------------------------------------------
// XRef
// ------------------------------------------------------------------------

struct XRefState {
    entries: Vec<XRefEntry>,
    last: i32,
    root_num: i32,
    root_gen: i32,
    ok: bool,
    err_code: i32,
    repaired: bool,
    trailer_dict: Object,
    last_xref_pos: i64,
    last_startxref_pos: i64,
    xref_table_pos: Vec<i64>,
    stream_ends: Vec<u64>,
}

struct EncState {
    encrypted: bool,
    perm_flags: i32,
    owner_password_ok: bool,
    file_key: [u8; 32],
    key_length: usize,
    enc_version: i32,
    enc_algorithm: CryptAlgorithm,
}

struct CacheEntry {
    num: i32,
    gen: i32,
    obj: Object,
}

struct ObjStrCache {
    objs: Vec<Option<Rc<ObjectStream>>>, // objStrs[objStrCacheSize]
    last_use: Vec<u32>,                  // objStrLastUse[]
    length: usize,                       // objStrCacheLength
    time: u32,                           // objStrTime
}

/// Mirrors `class XRef`.  Always lives in an `Rc` (created by
/// [`XRef::new`]); dictionaries and arrays parsed through it keep a weak
/// pointer back to it to resolve indirect references.
pub struct XRef {
    weak_self: Weak<XRef>,
    data: Rc<[u8]>,
    start: u64,
    st: RefCell<XRefState>,
    enc: RefCell<EncState>,
    cache: RefCell<Vec<CacheEntry>>,
    obj_strs: RefCell<ObjStrCache>,
}

/// Sequential reader over the file (`FileStream::getChar` etc.).
struct FileCursor<'a> {
    data: &'a [u8],
    pos: u64,
}

impl<'a> FileCursor<'a> {
    fn get_char(&mut self) -> i32 {
        if self.pos < self.data.len() as u64 {
            let c = self.data[self.pos as usize];
            self.pos += 1;
            c as i32
        } else {
            EOF
        }
    }
    fn get_block(&mut self, buf: &mut [u8]) -> usize {
        let mut n = 0;
        while n < buf.len() && self.pos < self.data.len() as u64 {
            buf[n] = self.data[self.pos as usize];
            self.pos += 1;
            n += 1;
        }
        n
    }
}

/// File position `start + off`, mapping negative values past EOF.
fn file_pos(start: u64, off: i64) -> u64 {
    let p = start as i128 + off as i128;
    if p < 0 {
        u64::MAX
    } else {
        p.min(u64::MAX as i128) as u64
    }
}

impl XRef {
    /// `XRef::XRef(BaseStream *strA, GBool repair)`.  `data` is the whole
    /// file, `start` the offset of the `%PDF-` header (`str->getStart()`).
    pub fn new(data: Rc<[u8]>, start: u64, repair: bool) -> Rc<XRef> {
        let x = Rc::new_cyclic(|w| XRef {
            weak_self: w.clone(),
            data,
            start,
            st: RefCell::new(XRefState {
                entries: Vec::new(),
                last: -1,
                root_num: 0,
                root_gen: 0,
                ok: true,
                err_code: codes::ERR_NONE,
                repaired: false,
                trailer_dict: Object::None,
                last_xref_pos: 0,
                last_startxref_pos: 0,
                xref_table_pos: Vec::new(),
                stream_ends: Vec::new(),
            }),
            enc: RefCell::new(EncState {
                encrypted: false,
                perm_flags: DEF_PERM_FLAGS,
                owner_password_ok: false,
                file_key: [0; 32],
                key_length: 0,
                enc_version: 0,
                enc_algorithm: CryptAlgorithm::RC4,
            }),
            cache: RefCell::new(
                (0..XREF_CACHE_SIZE).map(|_| CacheEntry { num: -1, gen: 0, obj: Object::None }).collect(),
            ),
            obj_strs: RefCell::new(ObjStrCache {
                objs: vec![None; OBJ_STR_CACHE_SIZE],
                last_use: vec![0; OBJ_STR_CACHE_SIZE],
                length: 0,
                time: 0,
            }),
        });
        x.init(repair);
        x
    }

    fn init(&self, repair: bool) {
        // if the 'repair' flag is set, try to reconstruct the xref table
        if repair {
            let ok = self.construct_xref();
            let mut st = self.st.borrow_mut();
            st.ok = ok;
            if !ok {
                st.err_code = codes::ERR_DAMAGED;
                return;
            }
            st.repaired = true;

        // if the 'repair' flag is not set, read the xref table
        } else {
            // read the trailer
            let mut pos = self.get_start_xref();
            if pos == 0 {
                let mut st = self.st.borrow_mut();
                st.err_code = codes::ERR_DAMAGED;
                st.ok = false;
                return;
            }

            // read the xref table
            let mut pos_set = XRefPosSet::new();
            while self.read_xref(&mut pos, &mut pos_set, false) {}
            let mut st = self.st.borrow_mut();
            st.xref_table_pos = pos_set.tab;
            if !st.ok {
                st.err_code = codes::ERR_DAMAGED;
                return;
            }
        }

        // get the root dictionary (catalog) object
        let root = self.st.borrow().trailer_dict.dict_lookup_nf("Root");
        if let Object::Ref(r) = root {
            let mut st = self.st.borrow_mut();
            st.root_num = r.num;
            st.root_gen = r.gen;
        } else {
            let ok = self.construct_xref();
            let mut st = self.st.borrow_mut();
            st.ok = ok;
            if !ok {
                st.err_code = codes::ERR_DAMAGED;
                return;
            }
        }

        // now set the trailer dictionary's xref pointer so we can fetch
        // indirect objects from it
        let td = self.st.borrow().trailer_dict.clone();
        if let Object::Dict(d) = td {
            d.set_xref(Some(self));
        }
    }

    pub(crate) fn weak(&self) -> Weak<XRef> {
        self.weak_self.clone()
    }

    /// The whole file contents.
    pub fn get_data(&self) -> &Rc<[u8]> {
        &self.data
    }

    /// `str->getStart()` (offset of the `%PDF-` header).
    pub fn get_start(&self) -> u64 {
        self.start
    }

    /// `isOk()`
    pub fn is_ok(&self) -> bool {
        self.st.borrow().ok
    }
    /// `getErrorCode()`
    pub fn get_error_code(&self) -> i32 {
        self.st.borrow().err_code
    }
    /// `isRepaired()`
    pub fn is_repaired(&self) -> bool {
        self.st.borrow().repaired
    }

    // ----- encryption -----

    /// `setEncryption(...)`
    pub fn set_encryption(
        &self,
        perm_flags: i32,
        owner_password_ok: bool,
        file_key: &[u8],
        key_length: usize,
        enc_version: i32,
        enc_algorithm: CryptAlgorithm,
    ) {
        let mut e = self.enc.borrow_mut();
        e.encrypted = true;
        e.perm_flags = perm_flags;
        e.owner_password_ok = owner_password_ok;
        e.key_length = key_length.min(32);
        let kl = e.key_length;
        e.file_key[..kl].copy_from_slice(&file_key[..kl]);
        e.enc_version = enc_version;
        e.enc_algorithm = enc_algorithm;
    }
    /// `isEncrypted()`
    pub fn is_encrypted(&self) -> bool {
        self.enc.borrow().encrypted
    }
    /// `getEncryption(...)`: (permFlags, ownerPasswordOk, keyLength,
    /// encVersion, encAlgorithm).
    pub fn get_encryption(&self) -> Option<(i32, bool, usize, i32, CryptAlgorithm)> {
        let e = self.enc.borrow();
        if !e.encrypted {
            return None;
        }
        Some((e.perm_flags, e.owner_password_ok, e.key_length, e.enc_version, e.enc_algorithm))
    }
    fn file_key(&self) -> Option<FileKey> {
        let e = self.enc.borrow();
        if e.encrypted {
            Some(FileKey { key: e.file_key, key_length: e.key_length, algo: e.enc_algorithm })
        } else {
            None
        }
    }
    /// `okToPrint(GBool ignoreOwnerPW = gFalse)`
    pub fn ok_to_print(&self, ignore_owner_pw: bool) -> bool {
        let e = self.enc.borrow();
        (!ignore_owner_pw && e.owner_password_ok) || (e.perm_flags & PERM_PRINT) != 0
    }
    pub fn ok_to_change(&self, ignore_owner_pw: bool) -> bool {
        let e = self.enc.borrow();
        (!ignore_owner_pw && e.owner_password_ok) || (e.perm_flags & PERM_CHANGE) != 0
    }
    pub fn ok_to_copy(&self, ignore_owner_pw: bool) -> bool {
        let e = self.enc.borrow();
        (!ignore_owner_pw && e.owner_password_ok) || (e.perm_flags & PERM_COPY) != 0
    }
    pub fn ok_to_add_notes(&self, ignore_owner_pw: bool) -> bool {
        let e = self.enc.borrow();
        (!ignore_owner_pw && e.owner_password_ok) || (e.perm_flags & PERM_NOTES) != 0
    }
    /// `getPermFlags()`
    pub fn get_perm_flags(&self) -> i32 {
        self.enc.borrow().perm_flags
    }

    // ----- accessors -----

    /// `getCatalog(Object *obj)`: fetch the root object.
    pub fn get_catalog(&self) -> Object {
        let (n, g) = {
            let st = self.st.borrow();
            (st.root_num, st.root_gen)
        };
        self.fetch(n, g)
    }
    /// `getDocInfo(Object *obj)`
    pub fn get_doc_info(&self) -> Object {
        let td = self.st.borrow().trailer_dict.clone();
        td.dict_lookup("Info")
    }
    /// `getDocInfoNF(Object *obj)` (added for pdfTeX)
    pub fn get_doc_info_nf(&self) -> Object {
        self.st.borrow().trailer_dict.dict_lookup_nf("Info")
    }
    /// `getNumObjects()` = last + 1
    pub fn get_num_objects(&self) -> i32 {
        self.st.borrow().last + 1
    }
    /// `getLastXRefPos()`
    pub fn get_last_xref_pos(&self) -> i64 {
        self.st.borrow().last_xref_pos
    }
    /// `getLastStartxrefPos()`
    pub fn get_last_startxref_pos(&self) -> i64 {
        self.st.borrow().last_startxref_pos
    }
    pub fn get_root_num(&self) -> i32 {
        self.st.borrow().root_num
    }
    pub fn get_root_gen(&self) -> i32 {
        self.st.borrow().root_gen
    }
    /// `getNumXRefTables()`
    pub fn get_num_xref_tables(&self) -> i32 {
        self.st.borrow().xref_table_pos.len() as i32
    }
    /// `getXRefTablePos(int idx)`
    pub fn get_xref_table_pos(&self, idx: i32) -> i64 {
        self.st.borrow().xref_table_pos[idx as usize]
    }
    /// `getSize()`
    pub fn get_size(&self) -> i32 {
        self.st.borrow().entries.len() as i32
    }
    /// `getEntry(int i)`
    pub fn get_entry(&self, i: i32) -> XRefEntry {
        self.st.borrow().entries[i as usize]
    }
    /// `getTrailerDict()`
    pub fn get_trailer_dict(&self) -> Object {
        self.st.borrow().trailer_dict.clone()
    }

    /// `getStreamEnd(GFileOffset streamStart, GFileOffset *streamEnd)`.
    pub fn get_stream_end(&self, stream_start: u64) -> Option<u64> {
        let st = self.st.borrow();
        let ends = &st.stream_ends;
        if ends.is_empty() || stream_start > ends[ends.len() - 1] {
            return None;
        }
        let mut a: isize = -1;
        let mut b: isize = ends.len() as isize - 1;
        // invariant: streamEnds[a] < streamStart <= streamEnds[b]
        while b - a > 1 {
            let m = (a + b) / 2;
            if stream_start <= ends[m as usize] {
                b = m;
            } else {
                a = m;
            }
        }
        Some(ends[b as usize])
    }

    // ----- reading the xref -----

    /// `getStartXref()`: read the 'startxref' position.
    fn get_start_xref(&self) -> i64 {
        let size = self.data.len();
        // str->setPos(xrefSearchSize, -1)
        let pos0 = size.saturating_sub(XREF_SEARCH_SIZE);
        let n = (size - pos0).min(XREF_SEARCH_SIZE);
        let buf = &self.data[pos0..pos0 + n];

        // find startxref
        if n < 9 {
            return 0;
        }
        let mut found = None;
        let mut i = n as isize - 9;
        while i >= 0 {
            if &buf[i as usize..i as usize + 9] == b"startxref" {
                found = Some(i as usize);
                break;
            }
            i -= 1;
        }
        let i = match found {
            Some(i) => i,
            None => return 0,
        };
        let mut p = i + 9;
        while p < n && c_isspace(buf[p]) {
            p += 1;
        }
        let v = str_to_file_offset(&buf[p..]);
        let mut st = self.st.borrow_mut();
        st.last_xref_pos = v;
        st.last_startxref_pos = (pos0 + i) as i64;
        v
    }

    /// `readXRef(GFileOffset *pos, XRefPosSet *posSet, GBool hybrid)`.
    fn read_xref(&self, pos: &mut i64, pos_set: &mut XRefPosSet, hybrid: bool) -> bool {
        // check for a loop in the xref tables
        if pos_set.check(*pos) {
            error(ErrorCategory::SyntaxWarning, -1, "Infinite loop in xref table");
            return false;
        }
        pos_set.add(*pos);

        // the xref data should either be "xref ..." (for an xref table) or
        // "nn gg obj << ... >> stream ..." (for an xref stream); possibly
        // preceded by whitespace
        let abs = file_pos(self.start, *pos);
        let mut cur = FileCursor { data: &self.data, pos: abs };
        let mut buf = [0u8; 100];
        let n = cur.get_block(&mut buf);
        let mut i = 0;
        while i < n && is_space(buf[i] as i32) {
            i += 1;
        }

        // parse an old-style xref table
        if !hybrid
            && i + 4 < n
            && buf[i] == b'x'
            && buf[i + 1] == b'r'
            && buf[i + 2] == b'e'
            && buf[i + 3] == b'f'
            && is_space(buf[i + 4] as i32)
        {
            self.read_xref_table(pos, (i + 5) as i64, pos_set)

        // parse an xref stream
        } else {
            let lexer = Lexer::new(self.data.clone(), abs, None);
            let mut parser = Parser::new(None, lexer, true);
            let mut ok = parser.get_obj_simple(true).is_int();
            if ok {
                ok = parser.get_obj_simple(true).is_int();
            }
            if ok {
                ok = parser.get_obj_simple(true).is_cmd_of("obj");
            }
            let mut obj = Object::Null;
            if ok {
                obj = parser.get_obj();
                ok = obj.is_stream();
            }
            if !ok {
                if hybrid {
                    error(ErrorCategory::SyntaxError, -1, "Invalid XRefStm link in trailer");
                } else {
                    self.st.borrow_mut().ok = false;
                }
                return false;
            }
            let s = obj.get_stream().unwrap().clone();
            drop(parser);
            self.read_xref_stream(&s, pos)
        }
    }

    /// Grow `entries` to `new_size`, initializing new entries as unset.
    fn grow_entries(st: &mut XRefState, new_size: i64) -> bool {
        if new_size > MAX_ENTRIES {
            error(ErrorCategory::Internal, -1, "Out of memory (xref table too large)");
            return false;
        }
        let cur = st.entries.len();
        if (new_size as usize) > cur {
            if st.entries.try_reserve(new_size as usize - cur).is_err() {
                error(ErrorCategory::Internal, -1, "Out of memory");
                return false;
            }
            st.entries.resize(new_size as usize, XRefEntry::UNSET);
        }
        true
    }

    /// `readXRefTable(GFileOffset *pos, int offset, XRefPosSet *posSet)`.
    fn read_xref_table(&self, pos: &mut i64, offset: i64, pos_set: &mut XRefPosSet) -> bool {
        let start_pos = file_pos(self.start, pos.wrapping_add(offset));
        let mut cur = FileCursor { data: &self.data, pos: start_pos };
        let digit_ok = |c: i32| (b'0' as i32..=b'9' as i32).contains(&c);

        let res: Result<(), ()> = (|| {
            loop {
                let mut c;
                loop {
                    c = cur.get_char();
                    if !is_space(c) {
                        break;
                    }
                }
                if c == b't' as i32 {
                    let mut b6 = [0u8; 6];
                    if cur.get_block(&mut b6) != 6 || &b6 != b"railer" {
                        return Err(());
                    }
                    break;
                }
                if !digit_ok(c) {
                    return Err(());
                }
                let mut first: i32 = 0;
                loop {
                    let digit = c - b'0' as i32;
                    if first > i32::MAX.wrapping_sub(digit) / 10 {
                        return Err(());
                    }
                    first = first.wrapping_mul(10).wrapping_add(digit);
                    c = cur.get_char();
                    if !digit_ok(c) {
                        break;
                    }
                }
                if !is_space(c) {
                    return Err(());
                }
                loop {
                    c = cur.get_char();
                    if !is_space(c) {
                        break;
                    }
                }
                let mut n: i32 = 0;
                loop {
                    let digit = c - b'0' as i32;
                    if n > i32::MAX.wrapping_sub(digit) / 10 {
                        return Err(());
                    }
                    n = n.wrapping_mul(10).wrapping_add(digit);
                    c = cur.get_char();
                    if !digit_ok(c) {
                        break;
                    }
                }
                if !is_space(c) {
                    return Err(());
                }
                if first > i32::MAX - n {
                    return Err(());
                }
                {
                    let mut st = self.st.borrow_mut();
                    let size = st.entries.len() as i32;
                    if first + n > size {
                        let mut new_size: i32 = if size != 0 { size } else { 512 };
                        loop {
                            if new_size > i32::MAX / 2 {
                                return Err(());
                            }
                            new_size <<= 1;
                            if first + n <= new_size {
                                break;
                            }
                        }
                        if !Self::grow_entries(&mut st, new_size as i64) {
                            return Err(());
                        }
                    }
                }
                let mut i = first;
                while i < first + n {
                    loop {
                        c = cur.get_char();
                        if !is_space(c) {
                            break;
                        }
                    }
                    let mut off: i64 = 0;
                    loop {
                        off = off.wrapping_mul(10).wrapping_add((c - b'0' as i32) as i64);
                        c = cur.get_char();
                        if !digit_ok(c) {
                            break;
                        }
                    }
                    if !is_space(c) {
                        return Err(());
                    }
                    loop {
                        c = cur.get_char();
                        if !is_space(c) {
                            break;
                        }
                    }
                    let mut gen: i32 = 0;
                    loop {
                        gen = gen.wrapping_mul(10).wrapping_add(c - b'0' as i32);
                        c = cur.get_char();
                        if !digit_ok(c) {
                            break;
                        }
                    }
                    if !is_space(c) {
                        return Err(());
                    }
                    loop {
                        c = cur.get_char();
                        if !is_space(c) {
                            break;
                        }
                    }
                    let type_ = if c == b'n' as i32 {
                        XRefEntryType::Uncompressed
                    } else if c == b'f' as i32 {
                        XRefEntryType::Free
                    } else {
                        return Err(());
                    };
                    c = cur.get_char();
                    if !is_space(c) {
                        return Err(());
                    }
                    let entry = XRefEntry { offset: off, gen, type_ };
                    let mut st = self.st.borrow_mut();
                    let iu = i as usize;
                    if st.entries[iu].offset == -1 {
                        st.entries[iu] = entry;
                        // PDF files of patents from the IBM Intellectual
                        // Property Network have a bug: the xref table claims
                        // to start at 1 instead of 0.
                        if i == 1
                            && first == 1
                            && st.entries[1].offset == 0
                            && st.entries[1].gen == 65535
                            && st.entries[1].type_ == XRefEntryType::Free
                        {
                            i = 0;
                            first = 0;
                            st.entries[0] = st.entries[1];
                            st.entries[1].offset = -1;
                        }
                        if i > st.last {
                            st.last = i;
                        }
                    }
                    i += 1;
                }
            }
            Ok(())
        })();
        if res.is_err() {
            self.st.borrow_mut().ok = false;
            return false;
        }

        // read the trailer dictionary
        let lexer = Lexer::new(self.data.clone(), cur.pos, None);
        let mut parser = Parser::new(None, lexer, true);
        let obj = parser.get_obj();
        drop(parser);
        let dict = match &obj {
            Object::Dict(d) => d.clone(),
            _ => {
                self.st.borrow_mut().ok = false;
                return false;
            }
        };

        // get the 'Prev' pointer
        let more;
        match dict.lookup_nf("Prev") {
            Object::Int(i) => {
                *pos = i as u32 as i64;
                more = true;
            }
            Object::Ref(r) => {
                // certain buggy PDF generators generate "/Prev NNN 0 R"
                // instead of "/Prev NNN"
                *pos = r.num as u32 as i64;
                more = true;
            }
            _ => more = false,
        }

        // save the first trailer dictionary
        {
            let mut st = self.st.borrow_mut();
            if st.trailer_dict.is_none() {
                st.trailer_dict = obj.clone();
            }
        }

        // check for an 'XRefStm' key
        if let Object::Int(i) = dict.lookup("XRefStm") {
            let mut pos2 = i as u32 as i64;
            self.read_xref(&mut pos2, pos_set, true);
            if !self.st.borrow().ok {
                self.st.borrow_mut().ok = false;
                return false;
            }
        }
        more
    }

    /// `readXRefStream(Stream *xrefStr, GFileOffset *pos)`.
    fn read_xref_stream(&self, xref_str: &Rc<Stream>, pos: &mut i64) -> bool {
        let dict = xref_str.get_dict().clone();
        let fail = |_: ()| {
            self.st.borrow_mut().ok = false;
            false
        };

        let new_size = match dict.lookup_nf("Size") {
            Object::Int(i) => i,
            _ => return fail(()),
        };
        if new_size < 0 {
            return fail(());
        }
        {
            let mut st = self.st.borrow_mut();
            if new_size as usize > st.entries.len() && !Self::grow_entries(&mut st, new_size as i64) {
                st.ok = false;
                return false;
            }
        }

        let w_obj = dict.lookup_nf("W");
        if !w_obj.is_array() || w_obj.array_get_length() < 3 {
            return fail(());
        }
        let mut w = [0i32; 3];
        for (i, wi) in w.iter_mut().enumerate() {
            match w_obj.array_get(i as i32) {
                Object::Int(v) => *wi = v,
                _ => return fail(()),
            }
        }
        if w.iter().any(|&x| !(0..=8).contains(&x)) {
            return fail(());
        }

        let mut reader = StreamReader::new(xref_str.get_decoded_data().unwrap_or_default());
        let idx = dict.lookup_nf("Index");
        if idx.is_array() {
            let mut i = 0;
            while i + 1 < idx.array_get_length() {
                let first = match idx.array_get(i) {
                    Object::Int(v) => v,
                    _ => return fail(()),
                };
                let n = match idx.array_get(i + 1) {
                    Object::Int(v) => v,
                    _ => return fail(()),
                };
                if first < 0 || n < 0 || !self.read_xref_stream_section(&mut reader, &w, first, n) {
                    return fail(());
                }
                i += 2;
            }
        } else if !self.read_xref_stream_section(&mut reader, &w, 0, new_size) {
            return fail(());
        }

        let more = match dict.lookup_nf("Prev") {
            Object::Int(i) => {
                *pos = i as u32 as i64;
                true
            }
            _ => false,
        };
        let mut st = self.st.borrow_mut();
        if st.trailer_dict.is_none() {
            st.trailer_dict = Object::Dict(dict);
        }
        more
    }

    /// `readXRefStreamSection(Stream *xrefStr, int *w, int first, int n)`.
    fn read_xref_stream_section(&self, reader: &mut StreamReader, w: &[i32; 3], first: i32, n: i32) -> bool {
        let end = first.wrapping_add(n);
        if end < 0 {
            return false;
        }
        let mut st = self.st.borrow_mut();
        let size = st.entries.len() as i32;
        if end > size {
            let mut new_size: i32 = if size != 0 { size.wrapping_mul(2) } else { 1024 };
            while end > new_size && new_size > 0 {
                new_size = new_size.wrapping_shl(1);
            }
            if new_size < 0 {
                return false;
            }
            if !Self::grow_entries(&mut st, new_size as i64) {
                return false;
            }
        }
        for i in first..end {
            let mut type_: i64;
            if w[0] == 0 {
                type_ = 1;
            } else {
                type_ = 0;
                for _ in 0..w[0] {
                    let c = reader.get_char();
                    if c == EOF {
                        return false;
                    }
                    type_ = type_.wrapping_shl(8).wrapping_add(c as i64);
                }
            }
            let mut offset: i64 = 0;
            for _ in 0..w[1] {
                let c = reader.get_char();
                if c == EOF {
                    return false;
                }
                offset = offset.wrapping_shl(8).wrapping_add(c as i64);
            }
            if offset < 0 || offset > GFILEOFFSET_MAX {
                return false;
            }
            let mut gen: i64 = 0;
            for _ in 0..w[2] {
                let c = reader.get_char();
                if c == EOF {
                    return false;
                }
                gen = gen.wrapping_shl(8).wrapping_add(c as i64);
            }
            // some PDF generators include a free entry with gen=0xffffffff
            if (gen < 0 || gen > i32::MAX as i64) && type_ != 0 {
                return false;
            }
            let iu = i as usize;
            if st.entries[iu].offset == -1 {
                let t = match type_ {
                    0 => XRefEntryType::Free,
                    1 => XRefEntryType::Uncompressed,
                    2 => XRefEntryType::Compressed,
                    _ => return false,
                };
                st.entries[iu] = XRefEntry { offset, gen: gen as i32, type_: t };
                if i > st.last {
                    st.last = i;
                }
            }
        }
        true
    }

    // ----- reconstruction -----

    /// `constructXRef()`: attempt to construct an xref table for a damaged
    /// file.  The scanning loop is ported with its 4 KB buffer, because
    /// its pattern matching is limited by the buffer refill logic.
    fn construct_xref(&self) -> bool {
        let mut stream_obj_nums: Vec<i32> = Vec::new();
        let mut last_obj_num: i32 = -1;
        {
            let mut st = self.st.borrow_mut();
            st.root_num = -1;
            st.stream_ends.clear();
        }
        let data = self.data.clone();
        let mut read_pos = self.start; // str->reset()
        let mut buf = vec![0u8; 4096 + 1];
        let mut buf_pos: u64 = self.start;
        let mut p: usize = 0;
        let mut end: usize = 0;
        let mut start_of_line = true;
        let mut space = true;
        let mut eof = false;
        loop {
            if end - p < 256 && !eof {
                buf.copy_within(p..end, 0);
                buf_pos += p as u64;
                let rem = end - p;
                p = rem;
                let n = 4096 - p;
                let avail = (data.len() as u64).saturating_sub(read_pos) as usize;
                let m = n.min(avail);
                if m > 0 {
                    let rp = read_pos as usize;
                    buf[p..p + m].copy_from_slice(&data[rp..rp + m]);
                }
                read_pos += m as u64;
                end = p + m;
                buf[end] = 0;
                p = 0;
                eof = m < n;
            }
            if p == end && eof {
                break;
            }
            if start_of_line && strncmp(&buf, p, b"trailer") {
                self.construct_trailer_dict(buf_pos + (p + 7) as u64);
                p += 7;
                start_of_line = false;
                space = false;
            } else if start_of_line && strncmp(&buf, p, b"endstream") {
                self.st.borrow_mut().stream_ends.push(buf_pos + p as u64);
                p += 9;
                start_of_line = false;
                space = false;
            } else if space && buf[p].is_ascii_digit() {
                p = self.construct_object_entry(&buf, p, buf_pos + p as u64, &mut last_obj_num);
                start_of_line = false;
                space = false;
            } else if buf[p] == b'>' && buf[p + 1] == b'>' {
                p += 2;
                start_of_line = false;
                space = false;
                // skip any PDF whitespace except for '\0'
                while matches!(buf[p], b'\t' | b'\n' | 0x0c | b'\r' | b' ') {
                    if buf[p] == b'\n' || buf[p] == b'\r' {
                        start_of_line = true;
                    }
                    space = true;
                    p += 1;
                }
                if strncmp(&buf, p, b"stream") {
                    if last_obj_num >= 0 {
                        stream_obj_nums.push(last_obj_num);
                    }
                    p += 6;
                    start_of_line = false;
                    space = false;
                }
            } else {
                if buf[p] == b'\n' || buf[p] == b'\r' {
                    start_of_line = true;
                    space = true;
                } else if is_space(buf[p] as i32) {
                    space = true;
                } else {
                    start_of_line = false;
                    space = false;
                }
                p += 1;
            }
        }

        // read each stream object, check for xref or object stream
        for &num in &stream_obj_nums {
            let gen = self.st.borrow().entries[num as usize].gen;
            let obj = self.fetch(num, gen);
            if let Object::Stream(s) = &obj {
                let dict = s.get_dict().clone();
                let type_ = dict.lookup("Type");
                if type_.is_name_of("XRef") {
                    self.save_trailer_dict(&dict, true);
                } else if type_.is_name_of("ObjStm") {
                    self.construct_object_stream_entries(s, num);
                }
            }
        }

        // if the file is encrypted, then any objects fetched here will be
        // incorrect (because decryption is not yet enabled), so clear the
        // cache to avoid that problem
        for e in self.cache.borrow_mut().iter_mut() {
            if e.num >= 0 {
                e.obj = Object::None;
                e.num = -1;
            }
        }

        if self.st.borrow().root_num < 0 {
            error(ErrorCategory::SyntaxError, -1, "Couldn't find trailer dictionary");
            return false;
        }
        true
    }

    /// `constructTrailerDict(GFileOffset pos)`.
    fn construct_trailer_dict(&self, pos: u64) {
        let lexer = Lexer::new(self.data.clone(), pos, None);
        let mut parser = Parser::new(None, lexer, false);
        let obj = parser.get_obj();
        if let Object::Dict(d) = obj {
            self.save_trailer_dict(&d, false);
        }
    }

    /// `saveTrailerDict(Dict *dict, GBool isXRefStream)`.
    fn save_trailer_dict(&self, dict: &Rc<Dict>, is_xref_stream: bool) {
        if let Object::Ref(r) = dict.lookup_nf("Root") {
            let mut st = self.st.borrow_mut();
            // the xref stream scanning code runs after all objects are
            // found, so we can check for a valid root object number at
            // that point
            if !is_xref_stream || r.num <= st.last {
                st.root_num = r.num;
                st.root_gen = r.gen;
                st.trailer_dict = Object::Dict(dict.clone());
            }
        }
    }

    /// `constructObjectEntry(char *p, GFileOffset pos, int *objNum)`:
    /// look for an object header ("nnn ggg obj") at `buf[p]`.
    fn construct_object_entry(&self, buf: &[u8], mut p: usize, pos: u64, obj_num: &mut i32) -> usize {
        let mut num: i32 = 0;
        loop {
            num = num.wrapping_mul(10).wrapping_add((buf[p] - b'0') as i32);
            p += 1;
            if !(buf[p].is_ascii_digit() && num < 100_000_000) {
                break;
            }
        }
        if !matches!(buf[p], b'\t' | 0x0c | b' ') {
            return p;
        }
        loop {
            p += 1;
            if !matches!(buf[p], b'\t' | 0x0c | b' ') {
                break;
            }
        }
        if !buf[p].is_ascii_digit() {
            return p;
        }
        let mut gen: i32 = 0;
        loop {
            gen = gen.wrapping_mul(10).wrapping_add((buf[p] - b'0') as i32);
            p += 1;
            if !(buf[p].is_ascii_digit() && gen < 100_000_000) {
                break;
            }
        }
        if !matches!(buf[p], b'\t' | 0x0c | b' ') {
            return p;
        }
        loop {
            p += 1;
            if !matches!(buf[p], b'\t' | 0x0c | b' ') {
                break;
            }
        }
        if !strncmp(buf, p, b"obj") {
            return p;
        }
        if self.construct_xref_entry(num, gen, pos as i64 - self.start as i64, XRefEntryType::Uncompressed) {
            *obj_num = num;
        }
        p
    }

    /// `constructObjectStreamEntries(Object *objStr, int objStrObjNum)`.
    fn construct_object_stream_entries(&self, obj_str: &Rc<Stream>, obj_str_obj_num: i32) {
        // get the object count
        let n_objects = match obj_str.get_dict().lookup("N") {
            Object::Int(i) => i,
            _ => return,
        };
        if n_objects <= 0 || n_objects > 1_000_000 {
            return;
        }
        // parse the header: object numbers and offsets (the lexer reads
        // the decoded stream from its start, not limited to /First)
        let data: Rc<[u8]> = Rc::from(obj_str.get_decoded_data().unwrap_or_default());
        let lexer = Lexer::new(data, 0, None);
        let mut parser = Parser::new(None, lexer, false);
        for i in 0..n_objects {
            let obj1 = parser.get_obj_simple(true);
            let obj2 = parser.get_obj_simple(true);
            if obj1.is_int() && obj2.is_int() {
                let num = obj1.get_int();
                if (0..1_000_000).contains(&num) {
                    self.construct_xref_entry(num, i, obj_str_obj_num as i64, XRefEntryType::Compressed);
                }
            }
        }
    }

    /// `constructXRefEntry(int num, int gen, GFileOffset pos, XRefEntryType
    /// type)`.
    fn construct_xref_entry(&self, num: i32, gen: i32, pos: i64, type_: XRefEntryType) -> bool {
        let mut st = self.st.borrow_mut();
        if num as usize >= st.entries.len() {
            let new_size = (num as i64 + 1 + 255) & !255;
            if new_size > i32::MAX as i64 {
                return false;
            }
            if !Self::grow_entries(&mut st, new_size) {
                return false;
            }
        }
        let e = &mut st.entries[num as usize];
        if e.type_ == XRefEntryType::Free || gen >= e.gen {
            e.offset = pos;
            e.gen = gen;
            e.type_ = type_;
            if num > st.last {
                st.last = num;
            }
        }
        true
    }

    // ----- fetching -----

    /// `fetch(int num, int gen, Object *obj, int recursion = 0)`.
    pub fn fetch(&self, num: i32, gen: i32) -> Object {
        self.fetch_rec(num, gen, 0)
    }

    pub fn fetch_rec(&self, num: i32, gen: i32, recursion: i32) -> Object {
        // check for bogus ref - this can happen in corrupted PDF files
        let size = self.st.borrow().entries.len();
        if num < 0 || num as usize >= size {
            return Object::Null;
        }

        // check the cache
        {
            let mut cache = self.cache.borrow_mut();
            if cache[0].num == num && cache[0].gen == gen {
                return cache[0].obj.clone();
            }
            for i in 1..XREF_CACHE_SIZE {
                if cache[i].num == num && cache[i].gen == gen {
                    let tmp = cache.remove(i);
                    let o = tmp.obj.clone();
                    cache.insert(0, tmp);
                    return o;
                }
            }
        }

        let e = self.st.borrow().entries[num as usize];
        let obj = match e.type_ {
            XRefEntryType::Uncompressed => {
                if e.gen != gen {
                    return Object::Null;
                }
                let lexer = Lexer::new(self.data.clone(), file_pos(self.start, e.offset), None);
                let mut parser = Parser::new(Some(self), lexer, true);
                let obj1 = parser.get_obj_simple(true);
                let obj2 = parser.get_obj_simple(true);
                let obj3 = parser.get_obj_simple(true);
                if !obj1.is_int()
                    || obj1.get_int() != num
                    || !obj2.is_int()
                    || obj2.get_int() != gen
                    || !obj3.is_cmd_of("obj")
                {
                    return Object::Null;
                }
                let fk = self.file_key();
                parser.get_obj_full(false, fk.as_ref(), num, gen, recursion)
            }
            XRefEntryType::Compressed => {
                let bad = {
                    let st = self.st.borrow();
                    e.offset >= st.entries.len() as i64
                        || e.offset < 0
                        || st.entries[e.offset as usize].type_ != XRefEntryType::Uncompressed
                };
                if bad {
                    error(ErrorCategory::SyntaxError, -1, "Invalid object stream");
                    return Object::Null;
                }
                match self.get_object_stream_object(e.offset as i32, e.gen, num, recursion) {
                    Some(o) => o,
                    None => return Object::Null,
                }
            }
            XRefEntryType::Free => return Object::Null,
        };

        // put the new object in the cache, throwing away the oldest object
        // currently in the cache
        {
            let mut cache = self.cache.borrow_mut();
            cache.pop();
            cache.insert(0, CacheEntry { num, gen, obj: obj.clone() });
        }
        obj
    }

    /// `getObjectStreamObject(int objStrNum, int objIdx, int objNum, Object
    /// *obj, int recursion)`.
    fn get_object_stream_object(&self, obj_str_num: i32, obj_idx: i32, obj_num: i32, recursion: i32) -> Option<Object> {
        if recursion >= OBJECT_RECURSION_LIMIT {
            return None;
        }

        // check for a cached ObjectStream
        let cached = self.get_object_stream_from_cache(obj_str_num);
        if let Some(os) = cached {
            let o = os.get_object(obj_idx, obj_num);
            self.clean_object_stream_cache();
            return Some(o);
        }

        // load a new ObjectStream
        let os = Rc::new(ObjectStream::new(self, obj_str_num, recursion + 1)?);
        let o = os.get_object(obj_idx, obj_num);
        self.add_object_stream_to_cache(os);
        self.clean_object_stream_cache();
        Some(o)
    }

    /// `getObjectStreamFromCache(int objStrNum)`.
    fn get_object_stream_from_cache(&self, obj_str_num: i32) -> Option<Rc<ObjectStream>> {
        let mut c = self.obj_strs.borrow_mut();
        // check the MRU entry in the cache
        if let Some(os) = &c.objs[0] {
            if os.obj_str_num == obj_str_num {
                let os = os.clone();
                c.last_use[0] = c.time;
                c.time = c.time.wrapping_add(1);
                return Some(os);
            }
        }
        // check the rest of the cache
        for i in 1..c.length {
            let hit = matches!(&c.objs[i], Some(os) if os.obj_str_num == obj_str_num);
            if hit {
                let os = c.objs[i].clone();
                for j in (1..=i).rev() {
                    c.objs[j] = c.objs[j - 1].clone();
                    c.last_use[j] = c.last_use[j - 1];
                }
                c.objs[0] = os.clone();
                c.last_use[0] = c.time;
                c.time = c.time.wrapping_add(1);
                return os;
            }
        }
        None
    }

    /// `addObjectStreamToCache(ObjectStream *objStr)`.
    fn add_object_stream_to_cache(&self, os: Rc<ObjectStream>) {
        let mut c = self.obj_strs.borrow_mut();
        if c.length == OBJ_STR_CACHE_SIZE {
            c.objs[OBJ_STR_CACHE_SIZE - 1] = None;
            c.length -= 1;
        }
        for j in (1..=c.length).rev() {
            c.objs[j] = c.objs[j - 1].clone();
            c.last_use[j] = c.last_use[j - 1];
        }
        c.length += 1;
        c.objs[0] = Some(os);
        c.last_use[0] = c.time;
        c.time = c.time.wrapping_add(1);
    }

    /// `cleanObjectStreamCache()`.
    fn clean_object_stream_cache(&self) {
        let mut c = self.obj_strs.borrow_mut();
        let len = c.length;
        if len > 1 && c.time.wrapping_sub(c.last_use[len - 1]) > OBJ_STR_CACHE_TIMEOUT {
            c.objs[len - 1] = None;
            c.length -= 1;
        }
    }
}

/// `strncmp(&buf[p], s, len(s)) == 0` on a NUL-terminated buffer.
fn strncmp(buf: &[u8], p: usize, s: &[u8]) -> bool {
    for (k, &ch) in s.iter().enumerate() {
        match buf.get(p + k) {
            Some(&b) if b == ch => {}
            _ => return false,
        }
    }
    true
}

/// C `isspace()` in the "C" locale.
fn c_isspace(c: u8) -> bool {
    c == b' ' || (0x09..=0x0d).contains(&c)
}

/// `XRef::strToFileOffset(char *s)`.
fn str_to_file_offset(s: &[u8]) -> i64 {
    let mut x: i64 = 0;
    for &c in s {
        if !c.is_ascii_digit() {
            break;
        }
        let d = (c - b'0') as i64;
        if x > (GFILEOFFSET_MAX - d) / 10 {
            break;
        }
        x = 10 * x + d;
    }
    x
}
