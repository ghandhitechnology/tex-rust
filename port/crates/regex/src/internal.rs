//! Port of `regex_internal.h` and `regex_internal.c`.
//!
//! Configuration reproduced (see README): `RE_ENABLE_I18N` undefined, `_LIBC`
//! undefined, `MB_CUR_MAX == 1` (C locale), `bitset_word_t` = 32-bit
//! `unsigned long` (Windows LLP64), `translate == NULL` (regcomp never sets a
//! translation table).

use crate::ctype;

// ---------------------------------------------------------------------------
// Error codes (regex.h `reg_errcode_t`).

pub(crate) type RegErrcode = i32;
pub(crate) const REG_NOERROR: RegErrcode = 0;
pub(crate) const REG_NOMATCH: RegErrcode = 1;
pub(crate) const REG_BADPAT: RegErrcode = 2;
pub(crate) const REG_ECOLLATE: RegErrcode = 3;
pub(crate) const REG_ECTYPE: RegErrcode = 4;
pub(crate) const REG_EESCAPE: RegErrcode = 5;
pub(crate) const REG_ESUBREG: RegErrcode = 6;
pub(crate) const REG_EBRACK: RegErrcode = 7;
pub(crate) const REG_EPAREN: RegErrcode = 8;
pub(crate) const REG_EBRACE: RegErrcode = 9;
pub(crate) const REG_BADBR: RegErrcode = 10;
pub(crate) const REG_ERANGE: RegErrcode = 11;
pub(crate) const REG_ESPACE: RegErrcode = 12;
pub(crate) const REG_BADRPT: RegErrcode = 13;
#[allow(dead_code)]
pub(crate) const REG_EEND: RegErrcode = 14;
#[allow(dead_code)]
pub(crate) const REG_ESIZE: RegErrcode = 15;
pub(crate) const REG_ERPAREN: RegErrcode = 16;

// ---------------------------------------------------------------------------
// Syntax bits (regex.h).  `reg_syntax_t` is `unsigned long` (32 bits on Windows).

pub(crate) type RegSyntax = u32;
pub(crate) const RE_BACKSLASH_ESCAPE_IN_LISTS: RegSyntax = 1;
pub(crate) const RE_BK_PLUS_QM: RegSyntax = RE_BACKSLASH_ESCAPE_IN_LISTS << 1;
pub(crate) const RE_CHAR_CLASSES: RegSyntax = RE_BK_PLUS_QM << 1;
pub(crate) const RE_CONTEXT_INDEP_ANCHORS: RegSyntax = RE_CHAR_CLASSES << 1;
pub(crate) const RE_CONTEXT_INDEP_OPS: RegSyntax = RE_CONTEXT_INDEP_ANCHORS << 1;
pub(crate) const RE_CONTEXT_INVALID_OPS: RegSyntax = RE_CONTEXT_INDEP_OPS << 1;
pub(crate) const RE_DOT_NEWLINE: RegSyntax = RE_CONTEXT_INVALID_OPS << 1;
pub(crate) const RE_DOT_NOT_NULL: RegSyntax = RE_DOT_NEWLINE << 1;
pub(crate) const RE_HAT_LISTS_NOT_NEWLINE: RegSyntax = RE_DOT_NOT_NULL << 1;
pub(crate) const RE_INTERVALS: RegSyntax = RE_HAT_LISTS_NOT_NEWLINE << 1;
pub(crate) const RE_LIMITED_OPS: RegSyntax = RE_INTERVALS << 1;
pub(crate) const RE_NEWLINE_ALT: RegSyntax = RE_LIMITED_OPS << 1;
pub(crate) const RE_NO_BK_BRACES: RegSyntax = RE_NEWLINE_ALT << 1;
pub(crate) const RE_NO_BK_PARENS: RegSyntax = RE_NO_BK_BRACES << 1;
pub(crate) const RE_NO_BK_REFS: RegSyntax = RE_NO_BK_PARENS << 1;
pub(crate) const RE_NO_BK_VBAR: RegSyntax = RE_NO_BK_REFS << 1;
pub(crate) const RE_NO_EMPTY_RANGES: RegSyntax = RE_NO_BK_VBAR << 1;
pub(crate) const RE_UNMATCHED_RIGHT_PAREN_ORD: RegSyntax = RE_NO_EMPTY_RANGES << 1;
pub(crate) const RE_NO_POSIX_BACKTRACKING: RegSyntax = RE_UNMATCHED_RIGHT_PAREN_ORD << 1;
pub(crate) const RE_NO_GNU_OPS: RegSyntax = RE_NO_POSIX_BACKTRACKING << 1;
pub(crate) const RE_DEBUG: RegSyntax = RE_NO_GNU_OPS << 1;
pub(crate) const RE_INVALID_INTERVAL_ORD: RegSyntax = RE_DEBUG << 1;
pub(crate) const RE_ICASE: RegSyntax = RE_INVALID_INTERVAL_ORD << 1;
pub(crate) const RE_CARET_ANCHORS_HERE: RegSyntax = RE_ICASE << 1;
pub(crate) const RE_CONTEXT_INVALID_DUP: RegSyntax = RE_CARET_ANCHORS_HERE << 1;
#[allow(dead_code)]
pub(crate) const RE_NO_SUB: RegSyntax = RE_CONTEXT_INVALID_DUP << 1;

pub(crate) const RE_SYNTAX_POSIX_COMMON: RegSyntax =
    RE_CHAR_CLASSES | RE_DOT_NEWLINE | RE_DOT_NOT_NULL | RE_INTERVALS | RE_NO_EMPTY_RANGES;
pub(crate) const RE_SYNTAX_POSIX_BASIC: RegSyntax =
    RE_SYNTAX_POSIX_COMMON | RE_BK_PLUS_QM | RE_CONTEXT_INVALID_DUP;
pub(crate) const RE_SYNTAX_POSIX_EXTENDED: RegSyntax = RE_SYNTAX_POSIX_COMMON
    | RE_CONTEXT_INDEP_ANCHORS
    | RE_CONTEXT_INDEP_OPS
    | RE_NO_BK_BRACES
    | RE_NO_BK_PARENS
    | RE_NO_BK_VBAR
    | RE_CONTEXT_INVALID_OPS
    | RE_UNMATCHED_RIGHT_PAREN_ORD;

pub(crate) const RE_DUP_MAX: i32 = 0x7fff;

// regexec eflags (regex.h).
pub(crate) const REG_NOTBOL: i32 = 1;
pub(crate) const REG_NOTEOL: i32 = 1 << 1;
pub(crate) const REG_STARTEND: i32 = 1 << 2;

// ---------------------------------------------------------------------------
// Bitsets.  `bitset_word_t` is `unsigned long`, i.e. 32 bits with MinGW.

pub(crate) const SBC_MAX: usize = 256;
pub(crate) const BITSET_WORD_BITS: i32 = 32;
pub(crate) const BITSET_WORDS: usize = SBC_MAX / BITSET_WORD_BITS as usize;
pub(crate) type Bitset = [u32; BITSET_WORDS];

#[inline]
pub(crate) fn bitset_set(set: &mut Bitset, i: usize) {
    set[i / 32] |= 1u32 << (i % 32);
}
#[inline]
pub(crate) fn bitset_clear(set: &mut Bitset, i: usize) {
    set[i / 32] &= !(1u32 << (i % 32));
}
#[inline]
pub(crate) fn bitset_contain(set: &Bitset, i: usize) -> bool {
    set[i / 32] & (1u32 << (i % 32)) != 0
}
#[inline]
pub(crate) fn bitset_not(set: &mut Bitset) {
    for w in set.iter_mut() {
        *w = !*w;
    }
}
#[inline]
pub(crate) fn bitset_merge(dest: &mut Bitset, src: &Bitset) {
    for i in 0..BITSET_WORDS {
        dest[i] |= src[i];
    }
}

// ---------------------------------------------------------------------------
// Context / constraint constants.

pub(crate) const PREV_WORD_CONSTRAINT: u32 = 0x0001;
pub(crate) const PREV_NOTWORD_CONSTRAINT: u32 = 0x0002;
pub(crate) const NEXT_WORD_CONSTRAINT: u32 = 0x0004;
pub(crate) const NEXT_NOTWORD_CONSTRAINT: u32 = 0x0008;
pub(crate) const PREV_NEWLINE_CONSTRAINT: u32 = 0x0010;
pub(crate) const NEXT_NEWLINE_CONSTRAINT: u32 = 0x0020;
pub(crate) const PREV_BEGBUF_CONSTRAINT: u32 = 0x0040;
pub(crate) const NEXT_ENDBUF_CONSTRAINT: u32 = 0x0080;
pub(crate) const WORD_DELIM_CONSTRAINT: u32 = 0x0100;
pub(crate) const NOT_WORD_DELIM_CONSTRAINT: u32 = 0x0200;

pub(crate) const INSIDE_WORD: u32 = PREV_WORD_CONSTRAINT | NEXT_WORD_CONSTRAINT;
pub(crate) const WORD_FIRST: u32 = PREV_NOTWORD_CONSTRAINT | NEXT_WORD_CONSTRAINT;
pub(crate) const WORD_LAST: u32 = PREV_WORD_CONSTRAINT | NEXT_NOTWORD_CONSTRAINT;
pub(crate) const INSIDE_NOTWORD: u32 = PREV_NOTWORD_CONSTRAINT | NEXT_NOTWORD_CONSTRAINT;
pub(crate) const LINE_FIRST: u32 = PREV_NEWLINE_CONSTRAINT;
pub(crate) const LINE_LAST: u32 = NEXT_NEWLINE_CONSTRAINT;
pub(crate) const BUF_FIRST: u32 = PREV_BEGBUF_CONSTRAINT;
pub(crate) const BUF_LAST: u32 = NEXT_ENDBUF_CONSTRAINT;
pub(crate) const WORD_DELIM: u32 = WORD_DELIM_CONSTRAINT;
pub(crate) const NOT_WORD_DELIM: u32 = NOT_WORD_DELIM_CONSTRAINT;

pub(crate) const CONTEXT_WORD: u32 = 1;
pub(crate) const CONTEXT_NEWLINE: u32 = CONTEXT_WORD << 1;
pub(crate) const CONTEXT_BEGBUF: u32 = CONTEXT_NEWLINE << 1;
pub(crate) const CONTEXT_ENDBUF: u32 = CONTEXT_BEGBUF << 1;

#[inline]
pub(crate) fn is_word_context(c: u32) -> bool {
    c & CONTEXT_WORD != 0
}
#[inline]
pub(crate) fn is_newline_context(c: u32) -> bool {
    c & CONTEXT_NEWLINE != 0
}
#[inline]
pub(crate) fn is_begbuf_context(c: u32) -> bool {
    c & CONTEXT_BEGBUF != 0
}
#[inline]
pub(crate) fn is_endbuf_context(c: u32) -> bool {
    c & CONTEXT_ENDBUF != 0
}
#[inline]
pub(crate) fn is_ordinary_context(c: u32) -> bool {
    c == 0
}

#[inline]
pub(crate) fn is_word_char(ch: i32) -> bool {
    ctype::isalnum(ch) || ch == b'_' as i32
}

pub(crate) const NEWLINE_CHAR: u8 = b'\n';

#[inline]
pub(crate) fn not_satisfy_prev_constraint(constraint: u32, context: u32) -> bool {
    ((constraint & PREV_WORD_CONSTRAINT) != 0 && !is_word_context(context))
        || ((constraint & PREV_NOTWORD_CONSTRAINT) != 0 && is_word_context(context))
        || ((constraint & PREV_NEWLINE_CONSTRAINT) != 0 && !is_newline_context(context))
        || ((constraint & PREV_BEGBUF_CONSTRAINT) != 0 && !is_begbuf_context(context))
}

#[inline]
pub(crate) fn not_satisfy_next_constraint(constraint: u32, context: u32) -> bool {
    ((constraint & NEXT_WORD_CONSTRAINT) != 0 && !is_word_context(context))
        || ((constraint & NEXT_NOTWORD_CONSTRAINT) != 0 && is_word_context(context))
        || ((constraint & NEXT_NEWLINE_CONSTRAINT) != 0 && !is_newline_context(context))
        || ((constraint & NEXT_ENDBUF_CONSTRAINT) != 0 && !is_endbuf_context(context))
}

// ---------------------------------------------------------------------------
// Token types (re_token_type_t).

pub(crate) type TokenType = u8;
#[allow(dead_code)]
pub(crate) const NON_TYPE: TokenType = 0;
pub(crate) const CHARACTER: TokenType = 1;
pub(crate) const END_OF_RE: TokenType = 2;
pub(crate) const SIMPLE_BRACKET: TokenType = 3;
pub(crate) const OP_BACK_REF: TokenType = 4;
pub(crate) const OP_PERIOD: TokenType = 5;
pub(crate) const EPSILON_BIT: TokenType = 8;
pub(crate) const OP_OPEN_SUBEXP: TokenType = EPSILON_BIT;
pub(crate) const OP_CLOSE_SUBEXP: TokenType = EPSILON_BIT | 1;
pub(crate) const OP_ALT: TokenType = EPSILON_BIT | 2;
pub(crate) const OP_DUP_ASTERISK: TokenType = EPSILON_BIT | 3;
pub(crate) const ANCHOR: TokenType = EPSILON_BIT | 4;
pub(crate) const CONCAT: TokenType = 16;
pub(crate) const SUBEXP: TokenType = 17;
pub(crate) const OP_DUP_PLUS: TokenType = 18;
pub(crate) const OP_DUP_QUESTION: TokenType = 19;
pub(crate) const OP_OPEN_BRACKET: TokenType = 20;
pub(crate) const OP_CLOSE_BRACKET: TokenType = 21;
pub(crate) const OP_CHARSET_RANGE: TokenType = 22;
pub(crate) const OP_OPEN_DUP_NUM: TokenType = 23;
pub(crate) const OP_CLOSE_DUP_NUM: TokenType = 24;
pub(crate) const OP_NON_MATCH_LIST: TokenType = 25;
pub(crate) const OP_OPEN_COLL_ELEM: TokenType = 26;
#[allow(dead_code)]
pub(crate) const OP_CLOSE_COLL_ELEM: TokenType = 27;
pub(crate) const OP_OPEN_EQUIV_CLASS: TokenType = 28;
#[allow(dead_code)]
pub(crate) const OP_CLOSE_EQUIV_CLASS: TokenType = 29;
pub(crate) const OP_OPEN_CHAR_CLASS: TokenType = 30;
#[allow(dead_code)]
pub(crate) const OP_CLOSE_CHAR_CLASS: TokenType = 31;
pub(crate) const OP_WORD: TokenType = 32;
pub(crate) const OP_NOTWORD: TokenType = 33;
pub(crate) const OP_SPACE: TokenType = 34;
pub(crate) const OP_NOTSPACE: TokenType = 35;
pub(crate) const BACK_SLASH: TokenType = 36;

#[inline]
pub(crate) fn is_epsilon_node(t: TokenType) -> bool {
    t & EPSILON_BIT != 0
}

/// `re_token_t`.  The C `opr` union is represented by separate fields; the C
/// code never reads a member other than the one written for the token's type
/// in a way that changes behaviour (checked by hand, see README).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Token {
    /// opr.c
    pub c: u8,
    /// opr.sbcset (index into `Dfa::sbcsets`)
    pub sbcset: u32,
    /// opr.idx
    pub idx: i32,
    /// opr.ctx_type
    pub ctx_type: u32,
    pub ty: TokenType,
    /// 10-bit context constraint.
    pub constraint: u32,
    pub duplicated: bool,
    pub opt_subexp: bool,
    pub word_char: bool,
}

// ---------------------------------------------------------------------------
// re_node_set: a literal port of the C algorithms (sorted int arrays which may
// transiently contain duplicates exactly where the C code would produce them).

#[derive(Clone, Debug, Default)]
pub(crate) struct NodeSet {
    pub alloc: i32,
    pub nelem: i32,
    pub elems: Vec<i32>,
}

impl NodeSet {
    #[inline]
    pub fn new_empty() -> NodeSet {
        NodeSet { alloc: 0, nelem: 0, elems: Vec::new() }
    }

    #[inline]
    pub fn as_slice(&self) -> &[i32] {
        &self.elems[..self.nelem.max(0) as usize]
    }

    #[inline]
    fn set_alloc(&mut self, n: i32) {
        self.alloc = n;
        self.elems.resize(n.max(0) as usize, 0);
    }

    /// re_node_set_alloc
    pub fn alloc_with(size: i32) -> NodeSet {
        NodeSet { alloc: size, nelem: 0, elems: vec![0; size.max(0) as usize] }
    }

    /// re_node_set_init_1
    pub fn init_1(elem: i32) -> NodeSet {
        NodeSet { alloc: 1, nelem: 1, elems: vec![elem] }
    }

    /// re_node_set_init_2
    pub fn init_2(elem1: i32, elem2: i32) -> NodeSet {
        if elem1 == elem2 {
            NodeSet { alloc: 2, nelem: 1, elems: vec![elem1, 0] }
        } else if elem1 < elem2 {
            NodeSet { alloc: 2, nelem: 2, elems: vec![elem1, elem2] }
        } else {
            NodeSet { alloc: 2, nelem: 2, elems: vec![elem2, elem1] }
        }
    }

    /// re_node_set_init_copy
    pub fn init_copy(src: &NodeSet) -> NodeSet {
        if src.nelem > 0 {
            NodeSet { alloc: src.nelem, nelem: src.nelem, elems: src.as_slice().to_vec() }
        } else {
            NodeSet::new_empty()
        }
    }

    /// re_node_set_empty
    #[inline]
    pub fn empty(&mut self) {
        self.nelem = 0;
    }

    /// re_node_set_add_intersect: add (src1 ∩ src2) to self.
    pub fn add_intersect(&mut self, src1: &NodeSet, src2: &NodeSet) {
        if src1.nelem == 0 || src2.nelem == 0 {
            return;
        }
        if src1.nelem + src2.nelem + self.nelem > self.alloc {
            let new_alloc = src1.nelem + src2.nelem + self.alloc;
            self.set_alloc(new_alloc);
        }
        let d = &mut self.elems;
        let s1 = &src1.elems;
        let s2 = &src2.elems;
        let mut sbase = self.nelem + src1.nelem + src2.nelem;
        let mut i1 = src1.nelem - 1;
        let mut i2 = src2.nelem - 1;
        let mut id = self.nelem - 1;
        loop {
            if s1[i1 as usize] == s2[i2 as usize] {
                while id >= 0 && d[id as usize] > s1[i1 as usize] {
                    id -= 1;
                }
                if id < 0 || d[id as usize] != s1[i1 as usize] {
                    sbase -= 1;
                    d[sbase as usize] = s1[i1 as usize];
                }
                i1 -= 1;
                if i1 < 0 {
                    break;
                }
                i2 -= 1;
                if i2 < 0 {
                    break;
                }
            } else if s1[i1 as usize] < s2[i2 as usize] {
                i2 -= 1;
                if i2 < 0 {
                    break;
                }
            } else {
                i1 -= 1;
                if i1 < 0 {
                    break;
                }
            }
        }

        let mut id = self.nelem - 1;
        let mut is = self.nelem + src1.nelem + src2.nelem - 1;
        let mut delta = is - sbase + 1;

        self.nelem += delta;
        if delta > 0 && id >= 0 {
            loop {
                if d[is as usize] > d[id as usize] {
                    d[(id + delta) as usize] = d[is as usize];
                    delta -= 1;
                    is -= 1;
                    if delta == 0 {
                        break;
                    }
                } else {
                    d[(id + delta) as usize] = d[id as usize];
                    id -= 1;
                    if id < 0 {
                        break;
                    }
                }
            }
        }
        // Copy remaining SRC elements.
        let sb = sbase as usize;
        let n = delta.max(0) as usize;
        d.copy_within(sb..sb + n, 0);
    }

    /// re_node_set_init_union
    pub fn init_union(src1: Option<&NodeSet>, src2: Option<&NodeSet>) -> NodeSet {
        let s1n = src1.map_or(0, |s| s.nelem);
        let s2n = src2.map_or(0, |s| s.nelem);
        if !(s1n > 0 && s2n > 0) {
            if s1n > 0 {
                return NodeSet::init_copy(src1.unwrap());
            } else if s2n > 0 {
                return NodeSet::init_copy(src2.unwrap());
            } else {
                return NodeSet::new_empty();
            }
        }
        let src1 = src1.unwrap();
        let src2 = src2.unwrap();
        let mut dest = NodeSet::alloc_with(s1n + s2n);
        let (mut i1, mut i2, mut id) = (0usize, 0usize, 0usize);
        let (n1, n2) = (s1n as usize, s2n as usize);
        while i1 < n1 && i2 < n2 {
            if src1.elems[i1] > src2.elems[i2] {
                dest.elems[id] = src2.elems[i2];
                id += 1;
                i2 += 1;
                continue;
            }
            if src1.elems[i1] == src2.elems[i2] {
                i2 += 1;
            }
            dest.elems[id] = src1.elems[i1];
            id += 1;
            i1 += 1;
        }
        if i1 < n1 {
            dest.elems[id..id + (n1 - i1)].copy_from_slice(&src1.elems[i1..n1]);
            id += n1 - i1;
        } else if i2 < n2 {
            dest.elems[id..id + (n2 - i2)].copy_from_slice(&src2.elems[i2..n2]);
            id += n2 - i2;
        }
        dest.nelem = id as i32;
        dest
    }

    /// re_node_set_merge: self ∪= src.
    pub fn merge(&mut self, src: &NodeSet) {
        if src.nelem == 0 {
            return;
        }
        if self.alloc < 2 * src.nelem + self.nelem {
            let new_alloc = 2 * (src.nelem + self.alloc);
            self.set_alloc(new_alloc);
        }
        if self.nelem == 0 {
            self.nelem = src.nelem;
            let n = src.nelem as usize;
            self.elems[..n].copy_from_slice(&src.elems[..n]);
            return;
        }
        let d = &mut self.elems;
        let s = &src.elems;
        let mut sbase = self.nelem + 2 * src.nelem;
        let mut is = src.nelem - 1;
        let mut id = self.nelem - 1;
        while is >= 0 && id >= 0 {
            if d[id as usize] == s[is as usize] {
                is -= 1;
                id -= 1;
            } else if d[id as usize] < s[is as usize] {
                sbase -= 1;
                d[sbase as usize] = s[is as usize];
                is -= 1;
            } else {
                id -= 1;
            }
        }
        if is >= 0 {
            sbase -= is + 1;
            let n = (is + 1) as usize;
            d[sbase as usize..sbase as usize + n].copy_from_slice(&s[..n]);
        }

        let mut id = self.nelem - 1;
        let mut is = self.nelem + 2 * src.nelem - 1;
        let mut delta = is - sbase + 1;
        if delta == 0 {
            return;
        }
        self.nelem += delta;
        loop {
            if d[is as usize] > d[id as usize] {
                d[(id + delta) as usize] = d[is as usize];
                delta -= 1;
                is -= 1;
                if delta == 0 {
                    break;
                }
            } else {
                d[(id + delta) as usize] = d[id as usize];
                id -= 1;
                if id < 0 {
                    let sb = sbase as usize;
                    d.copy_within(sb..sb + delta as usize, 0);
                    break;
                }
            }
        }
    }

    /// re_node_set_insert.  Returns 1 (allocation never fails here).
    pub fn insert(&mut self, elem: i32) -> i32 {
        if self.alloc == 0 {
            *self = NodeSet::init_1(elem);
            return 1;
        }
        if self.nelem == 0 {
            self.elems[0] = elem;
            self.nelem += 1;
            return 1;
        }
        if self.alloc == self.nelem {
            let na = self.alloc * 2;
            self.set_alloc(na);
        }
        let e = &mut self.elems;
        let mut idx;
        if elem < e[0] {
            idx = self.nelem as usize;
            while idx > 0 {
                e[idx] = e[idx - 1];
                idx -= 1;
            }
        } else {
            idx = self.nelem as usize;
            while e[idx - 1] > elem {
                e[idx] = e[idx - 1];
                idx -= 1;
            }
        }
        e[idx] = elem;
        self.nelem += 1;
        1
    }

    /// re_node_set_insert_last
    pub fn insert_last(&mut self, elem: i32) -> i32 {
        if self.alloc == self.nelem {
            let na = (self.alloc + 1) * 2;
            self.set_alloc(na);
        }
        self.elems[self.nelem as usize] = elem;
        self.nelem += 1;
        1
    }

    /// re_node_set_compare
    pub fn compare(set1: &NodeSet, set2: &NodeSet) -> bool {
        if set1.nelem != set2.nelem {
            return false;
        }
        let mut i = set1.nelem;
        loop {
            i -= 1;
            if i < 0 {
                break;
            }
            if set1.elems[i as usize] != set2.elems[i as usize] {
                return false;
            }
        }
        true
    }

    /// re_node_set_contains: idx+1 if present, else 0.
    pub fn contains(&self, elem: i32) -> i32 {
        if self.nelem <= 0 {
            return 0;
        }
        let mut idx: u32 = 0;
        let mut right: u32 = (self.nelem - 1) as u32;
        while idx < right {
            let mid = (idx + right) / 2;
            if self.elems[mid as usize] < elem {
                idx = mid + 1;
            } else {
                right = mid;
            }
        }
        if self.elems[idx as usize] == elem {
            idx as i32 + 1
        } else {
            0
        }
    }

    /// re_node_set_remove_at
    pub fn remove_at(&mut self, idx: i32) {
        if idx < 0 || idx >= self.nelem {
            return;
        }
        self.nelem -= 1;
        let mut i = idx as usize;
        while i < self.nelem as usize {
            self.elems[i] = self.elems[i + 1];
            i += 1;
        }
    }

    /// re_node_set_remove
    pub fn remove(&mut self, id: i32) {
        let i = self.contains(id) - 1;
        self.remove_at(i);
    }
}

// ---------------------------------------------------------------------------
// DFA states.

pub(crate) type StateId = u32;
/// A `re_dfastate_t *` (NULL = None).
pub(crate) type StatePtr = Option<StateId>;

#[derive(Debug)]
pub(crate) struct State {
    pub hash: u32,
    pub nodes: NodeSet,
    pub non_eps_nodes: NodeSet,
    /// `inveclosure.alloc == 0` means "not computed yet".
    pub inveclosure: NodeSet,
    /// None: `entrance_nodes == &nodes`.
    pub entrance_nodes: Option<NodeSet>,
    pub trtable: Option<Box<[StatePtr; SBC_MAX]>>,
    pub context: u32,
    pub halt: bool,
    pub has_backref: bool,
    pub has_constraint: bool,
}

impl State {
    #[inline]
    pub fn entrance(&self) -> &NodeSet {
        self.entrance_nodes.as_ref().unwrap_or(&self.nodes)
    }
}

/// The DFA's state table (`dfa->state_table`) plus the state storage.  In C
/// this lives inside the compiled pattern and is extended by every regexec
/// call; we keep the same semantics.
#[derive(Debug)]
pub(crate) struct StateCache {
    pub states: Vec<State>,
    pub table: Vec<Vec<StateId>>,
    pub state_hash_mask: u32,
}

impl StateCache {
    pub fn new(pat_len: usize) -> StateCache {
        // table_size = 2 ^ ceil(log pat_len)
        let mut table_size: u32 = 1;
        loop {
            if table_size as usize > pat_len {
                break;
            }
            table_size <<= 1;
        }
        StateCache {
            states: Vec::new(),
            table: vec![Vec::new(); table_size as usize],
            state_hash_mask: table_size - 1,
        }
    }

    #[inline]
    pub fn st(&self, id: StateId) -> &State {
        &self.states[id as usize]
    }

    #[inline]
    pub fn st_mut(&mut self, id: StateId) -> &mut State {
        &mut self.states[id as usize]
    }

    /// re_acquire_state
    pub fn acquire_state(&mut self, dfa: &Dfa, nodes: &NodeSet) -> StatePtr {
        if nodes.nelem == 0 {
            return None;
        }
        let hash = calc_state_hash(nodes, 0);
        let spot = (hash & self.state_hash_mask) as usize;
        for &sid in &self.table[spot] {
            let state = &self.states[sid as usize];
            if hash != state.hash {
                continue;
            }
            if NodeSet::compare(&state.nodes, nodes) {
                return Some(sid);
            }
        }
        Some(self.create_ci_newstate(dfa, nodes, hash))
    }

    /// re_acquire_state_context
    pub fn acquire_state_context(&mut self, dfa: &Dfa, nodes: &NodeSet, context: u32) -> StatePtr {
        if nodes.nelem == 0 {
            return None;
        }
        let hash = calc_state_hash(nodes, context);
        let spot = (hash & self.state_hash_mask) as usize;
        for &sid in &self.table[spot] {
            let state = &self.states[sid as usize];
            if state.hash == hash && state.context == context && NodeSet::compare(state.entrance(), nodes) {
                return Some(sid);
            }
        }
        Some(self.create_cd_newstate(dfa, nodes, context, hash))
    }

    /// register_state
    fn register_state(&mut self, dfa: &Dfa, mut newstate: State, hash: u32) -> StateId {
        newstate.hash = hash;
        newstate.non_eps_nodes = NodeSet::alloc_with(newstate.nodes.nelem);
        for i in 0..newstate.nodes.nelem as usize {
            let elem = newstate.nodes.elems[i];
            if !is_epsilon_node(dfa.nodes[elem as usize].ty) {
                newstate.non_eps_nodes.insert_last(elem);
            }
        }
        let id = self.states.len() as StateId;
        self.states.push(newstate);
        let spot = (hash & self.state_hash_mask) as usize;
        self.table[spot].push(id);
        id
    }

    fn blank_state(nodes: &NodeSet) -> State {
        State {
            hash: 0,
            nodes: NodeSet::init_copy(nodes),
            non_eps_nodes: NodeSet::new_empty(),
            inveclosure: NodeSet::new_empty(),
            entrance_nodes: None,
            trtable: None,
            context: 0,
            halt: false,
            has_backref: false,
            has_constraint: false,
        }
    }

    /// create_ci_newstate
    fn create_ci_newstate(&mut self, dfa: &Dfa, nodes: &NodeSet, hash: u32) -> StateId {
        let mut newstate = StateCache::blank_state(nodes);
        for i in 0..nodes.nelem as usize {
            let node = &dfa.nodes[nodes.elems[i] as usize];
            let ty = node.ty;
            if ty == CHARACTER && node.constraint == 0 {
                continue;
            }
            if ty == END_OF_RE {
                newstate.halt = true;
            } else if ty == OP_BACK_REF {
                newstate.has_backref = true;
            } else if ty == ANCHOR || node.constraint != 0 {
                newstate.has_constraint = true;
            }
        }
        self.register_state(dfa, newstate, hash)
    }

    /// create_cd_newstate
    fn create_cd_newstate(&mut self, dfa: &Dfa, nodes: &NodeSet, context: u32, hash: u32) -> StateId {
        let mut newstate = StateCache::blank_state(nodes);
        newstate.context = context;
        let mut nctx_nodes: i32 = 0;
        for i in 0..nodes.nelem {
            let node = &dfa.nodes[nodes.elems[i as usize] as usize];
            let ty = node.ty;
            let mut constraint: u32 = 0;
            if node.constraint != 0 {
                constraint = node.constraint;
            }
            if ty == CHARACTER && constraint == 0 {
                continue;
            }
            if ty == END_OF_RE {
                newstate.halt = true;
            } else if ty == OP_BACK_REF {
                newstate.has_backref = true;
            } else if ty == ANCHOR {
                constraint = node.ctx_type;
            }

            if constraint != 0 {
                if newstate.entrance_nodes.is_none() {
                    newstate.entrance_nodes = Some(NodeSet::init_copy(nodes));
                    nctx_nodes = 0;
                    newstate.has_constraint = true;
                }
                if not_satisfy_prev_constraint(constraint, context) {
                    newstate.nodes.remove_at(i - nctx_nodes);
                    nctx_nodes += 1;
                }
            }
        }
        self.register_state(dfa, newstate, hash)
    }
}

/// calc_state_hash
#[inline]
pub(crate) fn calc_state_hash(nodes: &NodeSet, context: u32) -> u32 {
    let mut hash: u32 = (nodes.nelem as u32).wrapping_add(context);
    for i in 0..nodes.nelem as usize {
        hash = hash.wrapping_add(nodes.elems[i] as u32);
    }
    hash
}

// ---------------------------------------------------------------------------
// The compiled DFA (`re_dfa_t`), immutable once compilation is finished.

#[derive(Debug)]
pub(crate) struct Dfa {
    pub nodes: Vec<Token>,
    pub nexts: Vec<i32>,
    pub org_indices: Vec<i32>,
    pub edests: Vec<NodeSet>,
    pub eclosures: Vec<NodeSet>,
    /// Empty when not computed (C: `inveclosures == NULL`).
    pub inveclosures: Vec<NodeSet>,
    /// Storage for SIMPLE_BRACKET bitsets (C: `opr.sbcset` pointers).
    pub sbcsets: Vec<Bitset>,
    pub init_state: StatePtr,
    pub init_state_word: StatePtr,
    pub init_state_nl: StatePtr,
    pub init_state_begbuf: StatePtr,
    pub init_node: i32,
    pub nbackref: i32,
    pub used_bkref_map: u32,
    pub completed_bkref_map: u32,
    pub has_plural_match: bool,
    pub has_mb_node: bool,
    pub word_ops_used: bool,
    pub mb_cur_max: i32,
    pub word_char: Bitset,
    pub syntax: RegSyntax,
    pub subexp_map: Option<Vec<i32>>,
}

impl Dfa {
    pub fn new() -> Dfa {
        Dfa {
            nodes: Vec::new(),
            nexts: Vec::new(),
            org_indices: Vec::new(),
            edests: Vec::new(),
            eclosures: Vec::new(),
            inveclosures: Vec::new(),
            sbcsets: Vec::new(),
            init_state: None,
            init_state_word: None,
            init_state_nl: None,
            init_state_begbuf: None,
            init_node: 0,
            nbackref: 0,
            used_bkref_map: 0,
            completed_bkref_map: 0,
            has_plural_match: false,
            has_mb_node: false,
            word_ops_used: false,
            mb_cur_max: 1,
            word_char: [0; BITSET_WORDS],
            syntax: 0,
            subexp_map: None,
        }
    }

    /// re_dfa_add_node
    pub fn add_node(&mut self, token: Token) -> i32 {
        let idx = self.nodes.len() as i32;
        let mut t = token;
        t.constraint = 0;
        self.nodes.push(t);
        self.nexts.push(-1);
        // org_indices is uninitialised memory in C for non-duplicated nodes;
        // it is only read for duplicated nodes.
        self.org_indices.push(0);
        self.edests.push(NodeSet::new_empty());
        self.eclosures.push(NodeSet::new_empty());
        idx
    }
}

// ---------------------------------------------------------------------------
// re_string_t for the pattern (compile side).

pub(crate) struct PatString<'a> {
    raw: &'a [u8],
    /// Upper-cased copy under REG_ICASE, otherwise the raw pattern.
    mbs: Vec<u8>,
    pub len: i32,
    pub cur_idx: i32,
    icase: bool,
}

impl<'a> PatString<'a> {
    /// re_string_construct (trans == NULL)
    pub fn construct(pattern: &'a [u8], icase: bool) -> PatString<'a> {
        let mbs = if icase {
            pattern
                .iter()
                .map(|&b| {
                    let ch = b as i32;
                    if ctype::islower(ch) {
                        ctype::toupper(ch) as u8
                    } else {
                        b
                    }
                })
                .collect()
        } else {
            pattern.to_vec()
        };
        PatString { raw: pattern, mbs, len: pattern.len() as i32, cur_idx: 0, icase }
    }

    #[inline]
    fn byte(&self, i: i32) -> u8 {
        if i >= 0 {
            self.mbs.get(i as usize).copied().unwrap_or(0)
        } else {
            0
        }
    }

    /// re_string_peek_byte
    #[inline]
    pub fn peek_byte(&self, offset: i32) -> u8 {
        self.byte(self.cur_idx + offset)
    }
    /// re_string_fetch_byte
    #[inline]
    pub fn fetch_byte(&mut self) -> u8 {
        let b = self.byte(self.cur_idx);
        self.cur_idx += 1;
        b
    }
    /// re_string_peek_byte_case (non-I18N build: raw byte when mbs_allocated)
    #[inline]
    pub fn peek_byte_case(&self, idx: i32) -> u8 {
        if !self.icase {
            return self.peek_byte(idx);
        }
        let off = self.cur_idx + idx;
        self.raw.get(off as usize).copied().unwrap_or(0)
    }
    /// re_string_fetch_byte_case
    #[inline]
    pub fn fetch_byte_case(&mut self) -> u8 {
        if !self.icase {
            return self.fetch_byte();
        }
        let b = self.raw.get(self.cur_idx as usize).copied().unwrap_or(0);
        self.cur_idx += 1;
        b
    }
    /// re_string_eoi (stop == len for patterns)
    #[inline]
    pub fn eoi(&self) -> bool {
        self.len <= self.cur_idx
    }
    #[inline]
    pub fn skip_bytes(&mut self, n: i32) {
        self.cur_idx += n;
    }
}

// ---------------------------------------------------------------------------
// re_string_t for the subject string (regexec side).
//
// The C code converts (upper-cases, for REG_ICASE) the input lazily into a
// buffer that grows by doubling.  We keep the same bookkeeping (`bufs_len`,
// `valid_len`, ...) so that every control-flow decision is identical, but the
// converted bytes are computed once for the whole string.

pub(crate) struct InputString<'a> {
    pub raw: &'a [u8],
    /// Converted copy of the *whole* raw string (upper-cased if icase).
    conv: Option<Vec<u8>>,
    pub raw_mbs_idx: i32,
    pub valid_len: i32,
    pub valid_raw_len: i32,
    pub bufs_len: i32,
    pub cur_idx: i32,
    pub raw_len: i32,
    pub len: i32,
    pub raw_stop: i32,
    pub stop: i32,
    pub tip_context: u32,
    pub word_char: Bitset,
    pub icase: bool,
    pub mbs_allocated: bool,
    pub newline_anchor: bool,
}

impl<'a> InputString<'a> {
    /// re_string_allocate (trans == NULL, mb_cur_max == 1)
    pub fn allocate(raw: &'a [u8], len: i32, init_len: i32, icase: bool, dfa: &Dfa) -> InputString<'a> {
        let init_len = if init_len < dfa.mb_cur_max { dfa.mb_cur_max } else { init_len };
        let init_buf_len = if len + 1 < init_len { len + 1 } else { init_len };
        let mbs_allocated = icase;
        let conv = if icase {
            Some(
                raw[..len as usize]
                    .iter()
                    .map(|&b| {
                        let ch = b as i32;
                        if ctype::islower(ch) {
                            ctype::toupper(ch) as u8
                        } else {
                            b
                        }
                    })
                    .collect(),
            )
        } else {
            None
        };
        let valid_len = if mbs_allocated || dfa.mb_cur_max > 1 { 0 } else { len };
        InputString {
            raw,
            conv,
            raw_mbs_idx: 0,
            valid_len,
            valid_raw_len: valid_len,
            bufs_len: init_buf_len,
            cur_idx: 0,
            raw_len: len,
            len,
            raw_stop: len,
            stop: len,
            tip_context: 0,
            word_char: dfa.word_char,
            icase,
            mbs_allocated,
            newline_anchor: false,
        }
    }

    /// Absolute byte of the converted buffer (the C `mbs` pointer is
    /// `raw_mbs_idx` bytes into it).  Bytes past the end read as NUL, which
    /// is what the C code sees for the raw (NUL-terminated) string.
    #[inline]
    pub fn abs_byte(&self, abs: i32) -> u8 {
        if abs < 0 {
            return 0;
        }
        match &self.conv {
            Some(v) => v.get(abs as usize).copied().unwrap_or(0),
            None => self.raw.get(abs as usize).copied().unwrap_or(0),
        }
    }

    /// re_string_byte_at
    #[inline]
    pub fn byte_at(&self, idx: i32) -> u8 {
        self.abs_byte(self.raw_mbs_idx + idx)
    }

    /// re_string_fetch_byte
    #[inline]
    pub fn fetch_byte(&mut self) -> u8 {
        let b = self.byte_at(self.cur_idx);
        self.cur_idx += 1;
        b
    }

    /// memcmp (buf + a, buf + b, n) == 0 on the converted buffer, with
    /// buffer-relative offsets.
    pub fn bytes_equal(&self, a: i32, b: i32, n: i32) -> bool {
        for k in 0..n {
            if self.byte_at(a + k) != self.byte_at(b + k) {
                return false;
            }
        }
        true
    }

    #[inline]
    pub fn eoi(&self) -> bool {
        self.stop <= self.cur_idx
    }

    #[inline]
    pub fn skip_bytes(&mut self, n: i32) {
        self.cur_idx += n;
    }

    /// build_upper_buffer / re_string_translate_buffer bookkeeping.
    pub fn build_buffers(&mut self) {
        let end_idx = if self.bufs_len > self.len { self.len } else { self.bufs_len };
        let start = self.valid_len;
        let mut char_idx = start;
        if char_idx < end_idx {
            char_idx = end_idx;
        }
        self.valid_len = char_idx;
        self.valid_raw_len = char_idx;
    }

    /// re_string_context_at
    pub fn context_at(&self, idx: i32, eflags: i32) -> u32 {
        if idx < 0 {
            return self.tip_context;
        }
        if idx == self.len {
            return if eflags & REG_NOTEOL != 0 { CONTEXT_ENDBUF } else { CONTEXT_NEWLINE | CONTEXT_ENDBUF };
        }
        let c = self.byte_at(idx);
        if bitset_contain(&self.word_char, c as usize) {
            return CONTEXT_WORD;
        }
        if c == NEWLINE_CHAR && self.newline_anchor {
            CONTEXT_NEWLINE
        } else {
            0
        }
    }

    /// re_string_reconstruct
    pub fn reconstruct(&mut self, idx: i32, eflags: i32) {
        let mut offset = idx - self.raw_mbs_idx;
        if offset < 0 {
            // Reset buffer.
            self.len = self.raw_len;
            self.stop = self.raw_stop;
            self.valid_len = 0;
            self.raw_mbs_idx = 0;
            self.valid_raw_len = 0;
            self.tip_context = if eflags & REG_NOTBOL != 0 { CONTEXT_BEGBUF } else { CONTEXT_NEWLINE | CONTEXT_BEGBUF };
            offset = idx;
        }

        if offset != 0 {
            if offset < self.valid_raw_len {
                self.tip_context = self.context_at(offset - 1, eflags);
                self.valid_len -= offset;
                self.valid_raw_len -= offset;
            } else {
                self.valid_len = 0;
                let c = self.raw.get((self.raw_mbs_idx + offset - 1) as usize).copied().unwrap_or(0);
                self.valid_raw_len = 0;
                self.tip_context = if bitset_contain(&self.word_char, c as usize) {
                    CONTEXT_WORD
                } else if c == NEWLINE_CHAR && self.newline_anchor {
                    CONTEXT_NEWLINE
                } else {
                    0
                };
            }
        }
        self.raw_mbs_idx = idx;
        self.len -= offset;
        self.stop -= offset;

        if self.mbs_allocated {
            if self.icase {
                self.build_buffers();
            }
        } else {
            self.valid_len = self.len;
        }
        self.cur_idx = 0;
    }
}
