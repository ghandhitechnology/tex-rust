//! Port of xpdf `Object.h/.cc`, `Array.h/.cc` and `Dict.h/.cc`.
//!
//! xpdf objects are a tagged union with manual reference counting for
//! arrays and dictionaries (`Object::copy` shares them, deep-copies
//! strings and names).  Here every compound value is reference counted
//! with `Rc`, so `Object::clone()` is the analogue of `Object::copy()`.
//! Objects returned by the parser are never mutated afterwards, so the
//! sharing is unobservable, exactly as in xpdf.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::{Rc, Weak};

use crate::stream::Stream;
use crate::xref::XRef;

/// Mirrors `struct Ref`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Ref {
    pub num: i32,
    pub gen: i32,
}

impl Ref {
    pub fn new(num: i32, gen: i32) -> Ref {
        Ref { num, gen }
    }
}

/// Mirrors `enum ObjType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjType {
    Bool,
    Int,
    Real,
    String,
    Name,
    Null,
    Array,
    Dict,
    Stream,
    Ref,
    Cmd,
    Error,
    EOF,
    None,
}

impl ObjType {
    /// Mirrors `objTypeNames[]` (used by pdftoepdf's error messages).
    pub fn name(self) -> &'static str {
        match self {
            ObjType::Bool => "boolean",
            ObjType::Int => "integer",
            ObjType::Real => "real",
            ObjType::String => "string",
            ObjType::Name => "name",
            ObjType::Null => "null",
            ObjType::Array => "array",
            ObjType::Dict => "dictionary",
            ObjType::Stream => "stream",
            ObjType::Ref => "ref",
            ObjType::Cmd => "cmd",
            ObjType::Error => "error",
            ObjType::EOF => "eof",
            ObjType::None => "none",
        }
    }
}

/// Mirrors `class Object`.
///
/// * `String` holds the raw bytes of a PDF string (xpdf `GString`; may
///   contain NUL bytes).
/// * `Name` and `Cmd` hold the bytes of xpdf's C string, i.e. they never
///   contain a NUL byte (the lexer truncates at the first NUL exactly as
///   `copyString()` does).
#[derive(Clone, Default)]
pub enum Object {
    Bool(bool),
    Int(i32),
    Real(f64),
    String(Rc<[u8]>),
    Name(Rc<[u8]>),
    Null,
    Array(Rc<Array>),
    Dict(Rc<Dict>),
    Stream(Rc<Stream>),
    Ref(Ref),
    Cmd(Rc<[u8]>),
    Error,
    EOF,
    #[default]
    None,
}

impl Object {
    // ----- constructors (initBool, initInt, ...) -----

    pub fn new_string(s: &[u8]) -> Object {
        Object::String(Rc::from(s))
    }
    /// `initName(const char*)`: the name is a C string, so it stops at
    /// the first NUL byte.
    pub fn new_name(s: &[u8]) -> Object {
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        Object::Name(Rc::from(&s[..end]))
    }
    pub fn new_cmd(s: &[u8]) -> Object {
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        Object::Cmd(Rc::from(&s[..end]))
    }
    pub fn new_ref(num: i32, gen: i32) -> Object {
        Object::Ref(Ref { num, gen })
    }

    // ----- type checking -----

    pub fn get_type(&self) -> ObjType {
        match self {
            Object::Bool(_) => ObjType::Bool,
            Object::Int(_) => ObjType::Int,
            Object::Real(_) => ObjType::Real,
            Object::String(_) => ObjType::String,
            Object::Name(_) => ObjType::Name,
            Object::Null => ObjType::Null,
            Object::Array(_) => ObjType::Array,
            Object::Dict(_) => ObjType::Dict,
            Object::Stream(_) => ObjType::Stream,
            Object::Ref(_) => ObjType::Ref,
            Object::Cmd(_) => ObjType::Cmd,
            Object::Error => ObjType::Error,
            Object::EOF => ObjType::EOF,
            Object::None => ObjType::None,
        }
    }
    pub fn is_bool(&self) -> bool {
        matches!(self, Object::Bool(_))
    }
    pub fn is_int(&self) -> bool {
        matches!(self, Object::Int(_))
    }
    pub fn is_real(&self) -> bool {
        matches!(self, Object::Real(_))
    }
    pub fn is_num(&self) -> bool {
        matches!(self, Object::Int(_) | Object::Real(_))
    }
    pub fn is_string(&self) -> bool {
        matches!(self, Object::String(_))
    }
    pub fn is_name(&self) -> bool {
        matches!(self, Object::Name(_))
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Object::Null)
    }
    pub fn is_array(&self) -> bool {
        matches!(self, Object::Array(_))
    }
    pub fn is_dict(&self) -> bool {
        matches!(self, Object::Dict(_))
    }
    pub fn is_stream(&self) -> bool {
        matches!(self, Object::Stream(_))
    }
    pub fn is_ref(&self) -> bool {
        matches!(self, Object::Ref(_))
    }
    pub fn is_cmd(&self) -> bool {
        matches!(self, Object::Cmd(_))
    }
    pub fn is_error(&self) -> bool {
        matches!(self, Object::Error)
    }
    pub fn is_eof(&self) -> bool {
        matches!(self, Object::EOF)
    }
    pub fn is_none(&self) -> bool {
        matches!(self, Object::None)
    }

    /// `isName(const char *nameA)`.
    pub fn is_name_of(&self, name: impl AsRef<[u8]>) -> bool {
        matches!(self, Object::Name(n) if &n[..] == name.as_ref())
    }
    /// `isDict(const char *dictType)`.
    pub fn is_dict_of(&self, dict_type: impl AsRef<[u8]>) -> bool {
        matches!(self, Object::Dict(d) if d.is(dict_type))
    }
    /// `isStream(char *dictType)`.
    pub fn is_stream_of(&self, dict_type: impl AsRef<[u8]>) -> bool {
        matches!(self, Object::Stream(s) if s.get_dict().is(dict_type))
    }
    /// `isCmd(const char *cmdA)`.
    pub fn is_cmd_of(&self, cmd: impl AsRef<[u8]>) -> bool {
        matches!(self, Object::Cmd(c) if &c[..] == cmd.as_ref())
    }

    // ----- accessors -----
    //
    // xpdf's accessors assume the object has the right type (anything
    // else is undefined behaviour in C++).  These return a neutral
    // value instead of panicking.

    pub fn get_bool(&self) -> bool {
        matches!(self, Object::Bool(true))
    }
    pub fn get_int(&self) -> i32 {
        match self {
            Object::Int(i) => *i,
            _ => 0,
        }
    }
    pub fn get_real(&self) -> f64 {
        match self {
            Object::Real(r) => *r,
            _ => 0.0,
        }
    }
    /// `getNum()`: `type == objInt ? (double)intg : real`.
    pub fn get_num(&self) -> f64 {
        match self {
            Object::Int(i) => *i as f64,
            Object::Real(r) => *r,
            _ => 0.0,
        }
    }
    /// String bytes (`getString()->getCString()` with its length).
    pub fn get_string(&self) -> &[u8] {
        match self {
            Object::String(s) => s,
            _ => &[],
        }
    }
    /// Name bytes (`getName()`), without the leading '/'.
    pub fn get_name(&self) -> &[u8] {
        match self {
            Object::Name(s) => s,
            _ => &[],
        }
    }
    /// Name as UTF-8 text (lossy), for convenience.
    pub fn get_name_str(&self) -> String {
        String::from_utf8_lossy(self.get_name()).into_owned()
    }
    pub fn get_array(&self) -> Option<&Rc<Array>> {
        match self {
            Object::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn get_dict(&self) -> Option<&Rc<Dict>> {
        match self {
            Object::Dict(d) => Some(d),
            _ => None,
        }
    }
    pub fn get_stream(&self) -> Option<&Rc<Stream>> {
        match self {
            Object::Stream(s) => Some(s),
            _ => None,
        }
    }
    pub fn get_ref(&self) -> Ref {
        match self {
            Object::Ref(r) => *r,
            _ => Ref::default(),
        }
    }
    pub fn get_ref_num(&self) -> i32 {
        self.get_ref().num
    }
    pub fn get_ref_gen(&self) -> i32 {
        self.get_ref().gen
    }
    pub fn get_cmd(&self) -> &[u8] {
        match self {
            Object::Cmd(s) => s,
            _ => &[],
        }
    }

    /// `getTypeName()`.
    pub fn get_type_name(&self) -> &'static str {
        self.get_type().name()
    }

    // ----- fetch -----

    /// `Object::fetch(XRef *xref, Object *obj, int recursion = 0)`: if
    /// this is a Ref and `xref` is given, fetch the referenced object,
    /// otherwise return a copy.
    pub fn fetch(&self, xref: Option<&XRef>) -> Object {
        self.fetch_rec(xref, 0)
    }
    pub fn fetch_rec(&self, xref: Option<&XRef>, recursion: i32) -> Object {
        match (self, xref) {
            (Object::Ref(r), Some(x)) => x.fetch_rec(r.num, r.gen, recursion),
            _ => self.clone(),
        }
    }
    pub(crate) fn fetch_weak(&self, xref: &Weak<XRef>, recursion: i32) -> Object {
        match self {
            Object::Ref(r) => match xref.upgrade() {
                Some(x) => x.fetch_rec(r.num, r.gen, recursion),
                None => self.clone(),
            },
            _ => self.clone(),
        }
    }

    // ----- array accessors -----

    pub fn array_get_length(&self) -> i32 {
        match self {
            Object::Array(a) => a.get_length(),
            _ => 0,
        }
    }
    pub fn array_get(&self, i: i32) -> Object {
        match self {
            Object::Array(a) => a.get(i),
            _ => Object::Null,
        }
    }
    pub fn array_get_rec(&self, i: i32, recursion: i32) -> Object {
        match self {
            Object::Array(a) => a.get_rec(i, recursion),
            _ => Object::Null,
        }
    }
    pub fn array_get_nf(&self, i: i32) -> Object {
        match self {
            Object::Array(a) => a.get_nf(i),
            _ => Object::Null,
        }
    }

    // ----- dict accessors -----

    pub fn dict_get_length(&self) -> i32 {
        match self {
            Object::Dict(d) => d.get_length(),
            _ => 0,
        }
    }
    /// `dictAdd(char *key, Object *val)`.  Only valid on a dictionary that
    /// is not shared yet (a freshly created one); a shared dictionary is
    /// copied first (copy-on-write), which xpdf never needs.
    pub fn dict_add(&mut self, key: &[u8], val: Object) {
        if let Object::Dict(d) = self {
            Rc::make_mut(d).add(key, val);
        }
    }
    pub fn dict_is(&self, dict_type: impl AsRef<[u8]>) -> bool {
        match self {
            Object::Dict(d) => d.is(dict_type),
            _ => false,
        }
    }
    pub fn dict_lookup(&self, key: impl AsRef<[u8]>) -> Object {
        match self {
            Object::Dict(d) => d.lookup(key),
            _ => Object::Null,
        }
    }
    pub fn dict_lookup_rec(&self, key: impl AsRef<[u8]>, recursion: i32) -> Object {
        match self {
            Object::Dict(d) => d.lookup_rec(key, recursion),
            _ => Object::Null,
        }
    }
    pub fn dict_lookup_nf(&self, key: impl AsRef<[u8]>) -> Object {
        match self {
            Object::Dict(d) => d.lookup_nf(key),
            _ => Object::Null,
        }
    }
    pub fn dict_get_key(&self, i: i32) -> Rc<[u8]> {
        match self {
            Object::Dict(d) => d.get_key(i),
            _ => Rc::from(&b""[..]),
        }
    }
    pub fn dict_get_val(&self, i: i32) -> Object {
        match self {
            Object::Dict(d) => d.get_val(i),
            _ => Object::Null,
        }
    }
    pub fn dict_get_val_nf(&self, i: i32) -> Object {
        match self {
            Object::Dict(d) => d.get_val_nf(i),
            _ => Object::Null,
        }
    }

    // ----- stream accessors -----

    /// `streamGetDict()`.
    pub fn stream_get_dict(&self) -> Option<&Rc<Dict>> {
        match self {
            Object::Stream(s) => Some(s.get_dict()),
            _ => None,
        }
    }
    /// `streamIs(char *dictType)`.
    pub fn stream_is(&self, dict_type: impl AsRef<[u8]>) -> bool {
        self.is_stream_of(dict_type)
    }
}

impl fmt::Debug for Object {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Object::Bool(b) => write!(f, "{}", b),
            Object::Int(i) => write!(f, "{}", i),
            Object::Real(r) => write!(f, "{:?}", r),
            Object::String(s) => write!(f, "({})", String::from_utf8_lossy(s).escape_debug()),
            Object::Name(n) => write!(f, "/{}", String::from_utf8_lossy(n)),
            Object::Null => write!(f, "null"),
            Object::Array(a) => {
                write!(f, "[")?;
                for (i, e) in a.elems.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{:?}", e)?;
                }
                write!(f, "]")
            }
            Object::Dict(d) => write!(f, "{:?}", d),
            Object::Stream(s) => write!(f, "{:?} stream", s.get_dict()),
            Object::Ref(r) => write!(f, "{} {} R", r.num, r.gen),
            Object::Cmd(c) => write!(f, "{}", String::from_utf8_lossy(c)),
            Object::Error => write!(f, "<error>"),
            Object::EOF => write!(f, "<EOF>"),
            Object::None => write!(f, "<none>"),
        }
    }
}

// ------------------------------------------------------------------------
// Array
// ------------------------------------------------------------------------

/// Mirrors `class Array`.  The `xref` pointer is used to resolve
/// indirect references in [`Array::get`] (it is "null" for arrays parsed
/// by a parser that had no XRef, e.g. trailer dictionaries).
#[derive(Clone, Default)]
pub struct Array {
    xref: Weak<XRef>,
    elems: Vec<Object>,
}

impl Array {
    pub fn new(xref: Option<&XRef>) -> Array {
        Array { xref: xref.map(|x| x.weak()).unwrap_or_default(), elems: Vec::new() }
    }
    pub(crate) fn new_weak(xref: Weak<XRef>) -> Array {
        Array { xref, elems: Vec::new() }
    }
    pub fn get_length(&self) -> i32 {
        self.elems.len() as i32
    }
    pub fn add(&mut self, elem: Object) {
        self.elems.push(elem);
    }
    /// `get(int i, Object *obj, int recursion = 0)` — resolves refs.
    pub fn get(&self, i: i32) -> Object {
        self.get_rec(i, 0)
    }
    pub fn get_rec(&self, i: i32, recursion: i32) -> Object {
        if i < 0 || i as usize >= self.elems.len() {
            return Object::Null;
        }
        self.elems[i as usize].fetch_weak(&self.xref, recursion)
    }
    /// `getNF(int i, Object *obj)` — does not resolve refs.
    pub fn get_nf(&self, i: i32) -> Object {
        if i < 0 || i as usize >= self.elems.len() {
            return Object::Null;
        }
        self.elems[i as usize].clone()
    }
    /// Direct access to the (unresolved) elements.
    pub fn elems(&self) -> &[Object] {
        &self.elems
    }
}

// ------------------------------------------------------------------------
// Dict
// ------------------------------------------------------------------------

/// Mirrors `class Dict`.
///
/// Entries keep insertion order (the order of first occurrence in the
/// file).  Adding a key that already exists replaces the value *in
/// place* (xpdf `Dict::add`), so for `<< /A 1 /B 2 /A 3 >>` the entries
/// are `A 3`, `B 2`.
pub struct Dict {
    xref: RefCell<Weak<XRef>>,
    entries: Vec<(Rc<[u8]>, Object)>,
    // hash index, built once the dictionary gets big
    index: Option<HashMap<Rc<[u8]>, usize>>,
}

const DICT_INDEX_THRESHOLD: usize = 16;

impl Clone for Dict {
    fn clone(&self) -> Dict {
        Dict {
            xref: RefCell::new(self.xref.borrow().clone()),
            entries: self.entries.clone(),
            index: self.index.clone(),
        }
    }
}

impl Default for Dict {
    fn default() -> Dict {
        Dict { xref: RefCell::new(Weak::new()), entries: Vec::new(), index: None }
    }
}

impl Dict {
    pub fn new(xref: Option<&XRef>) -> Dict {
        Dict {
            xref: RefCell::new(xref.map(|x| x.weak()).unwrap_or_default()),
            entries: Vec::new(),
            index: None,
        }
    }
    pub(crate) fn new_weak(xref: Weak<XRef>) -> Dict {
        Dict { xref: RefCell::new(xref), entries: Vec::new(), index: None }
    }
    /// `getLength()`.
    pub fn get_length(&self) -> i32 {
        self.entries.len() as i32
    }

    fn find(&self, key: &[u8]) -> Option<usize> {
        match &self.index {
            Some(ix) => ix.get(key).copied(),
            None => self.entries.iter().position(|(k, _)| &k[..] == key),
        }
    }

    /// `add(char *key, Object *val)`.
    pub fn add(&mut self, key: &[u8], val: Object) {
        if let Some(i) = self.find(key) {
            self.entries[i].1 = val;
            return;
        }
        let k: Rc<[u8]> = Rc::from(key);
        if let Some(ix) = &mut self.index {
            ix.insert(k.clone(), self.entries.len());
        }
        self.entries.push((k, val));
        if self.index.is_none() && self.entries.len() > DICT_INDEX_THRESHOLD {
            let mut ix = HashMap::with_capacity(self.entries.len() * 2);
            for (i, (k, _)) in self.entries.iter().enumerate() {
                ix.insert(k.clone(), i);
            }
            self.index = Some(ix);
        }
    }

    /// `is(const char *type)`: `/Type` entry is the given name.
    pub fn is(&self, dict_type: impl AsRef<[u8]>) -> bool {
        match self.find(b"Type") {
            Some(i) => self.entries[i].1.is_name_of(dict_type),
            None => false,
        }
    }

    /// `lookup(const char *key, Object *obj, int recursion = 0)`.
    pub fn lookup(&self, key: impl AsRef<[u8]>) -> Object {
        self.lookup_rec(key, 0)
    }
    pub fn lookup_rec(&self, key: impl AsRef<[u8]>, recursion: i32) -> Object {
        match self.find(key.as_ref()) {
            Some(i) => {
                let x = self.xref.borrow().clone();
                self.entries[i].1.fetch_weak(&x, recursion)
            }
            None => Object::Null,
        }
    }
    /// `lookupNF(const char *key, Object *obj)`.
    pub fn lookup_nf(&self, key: impl AsRef<[u8]>) -> Object {
        match self.find(key.as_ref()) {
            Some(i) => self.entries[i].1.clone(),
            None => Object::Null,
        }
    }
    /// `getKey(int i)`.
    pub fn get_key(&self, i: i32) -> Rc<[u8]> {
        self.entries[i as usize].0.clone()
    }
    /// `getVal(int i, Object *obj)` — resolves refs.
    pub fn get_val(&self, i: i32) -> Object {
        let x = self.xref.borrow().clone();
        self.entries[i as usize].1.fetch_weak(&x, 0)
    }
    /// `getValNF(int i, Object *obj)`.
    pub fn get_val_nf(&self, i: i32) -> Object {
        self.entries[i as usize].1.clone()
    }
    /// `setXRef(XRef *xrefA)` (used for the trailer dictionary).
    pub fn set_xref(&self, xref: Option<&XRef>) {
        *self.xref.borrow_mut() = xref.map(|x| x.weak()).unwrap_or_default();
    }
    /// Iterate over (key, unresolved value) in dictionary order.
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &Object)> {
        self.entries.iter().map(|(k, v)| (&k[..], v))
    }
}

impl fmt::Debug for Dict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<<")?;
        for (k, v) in &self.entries {
            write!(f, " /{} {:?}", String::from_utf8_lossy(k), v)?;
        }
        write!(f, " >>")
    }
}
