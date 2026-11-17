//! Pure-Rust port of the POSIX regular-expression engine bundled with pdfTeX
//! in TeX Live 2026 (`texk/web2c/pdftexdir/regex/`, glibc 2.5's engine as
//! packaged by libgnurx).  pdfTeX uses it for `\pdfmatch`.
//!
//! The port reproduces the configuration of TeX Live's Windows build: no
//! multibyte/I18N support, the C locale's character classes (ASCII only),
//! 32-bit `unsigned long` bitset words, and the glibc 2.5 algorithms
//! (including their quirks).  See README.md.
//!
//! ```
//! use texrs_posix_regex::{Regex, REG_EXTENDED};
//! let re = Regex::compile(b"(a+)(b*)", REG_EXTENDED).unwrap();
//! let m = re.exec(b"xaabbc", 3, 0).unwrap();
//! assert_eq!(m, vec![(1, 5), (1, 3), (3, 5)]);
//! ```

mod compile;
mod ctype;
mod exec;
mod internal;
pub mod pdftex;

use std::cell::RefCell;

pub use exec::ExecAbort;

/// regcomp cflags: use POSIX extended syntax.
pub const REG_EXTENDED: i32 = 1;
/// regcomp cflags: ignore case.
pub const REG_ICASE: i32 = REG_EXTENDED << 1;
/// regcomp cflags: `.` and `[^...]` do not match newline; anchors match at
/// newlines.
pub const REG_NEWLINE: i32 = REG_ICASE << 1;
/// regcomp cflags: report only success/failure.
pub const REG_NOSUB: i32 = REG_NEWLINE << 1;

/// regexec eflags: `^` does not match at the beginning of the string.
pub const REG_NOTBOL: i32 = 1;
/// regexec eflags: `$` does not match at the end of the string.
pub const REG_NOTEOL: i32 = 1 << 1;
/// regexec eflags: use `pmatch[0]` to delimit the string (only meaningful
/// for [`Regex::regexec`]).
pub const REG_STARTEND: i32 = 1 << 2;

/// regexec/regcomp return value for "no match".
pub const REG_NOMATCH: i32 = 1;

/// `__re_error_msgid`: the message for each `reg_errcode_t`, exactly as in
/// regcomp.c (gettext is the identity in TeX Live's build).
const ERROR_MESSAGES: [&str; 17] = [
    "Success",
    "No match",
    "Invalid regular expression",
    "Invalid collation character",
    "Invalid character class name",
    "Trailing backslash",
    "Invalid back reference",
    "Unmatched [ or [^",
    "Unmatched ( or \\(",
    "Unmatched \\{",
    "Invalid content of \\{\\}",
    "Invalid range end",
    "Memory exhausted",
    "Invalid preceding regular expression",
    "Premature end of regular expression",
    "Regular expression too big",
    "Unmatched ) or \\)",
];

/// `regerror`: the message text for an error code (0..=16).  Returns `None`
/// for codes outside the table (the C function calls `abort()`).
pub fn regerror(errcode: i32) -> Option<&'static str> {
    if (0..ERROR_MESSAGES.len() as i32).contains(&errcode) {
        Some(ERROR_MESSAGES[errcode as usize])
    } else {
        None
    }
}

/// A compilation error (`reg_errcode_t` returned by `regcomp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum RegError {
    /// REG_BADPAT: "Invalid regular expression"
    BadPat = 2,
    /// REG_ECOLLATE: "Invalid collation character"
    ECollate = 3,
    /// REG_ECTYPE: "Invalid character class name"
    ECtype = 4,
    /// REG_EESCAPE: "Trailing backslash"
    EEscape = 5,
    /// REG_ESUBREG: "Invalid back reference"
    ESubreg = 6,
    /// REG_EBRACK: "Unmatched [ or [^"
    EBrack = 7,
    /// REG_EPAREN: "Unmatched ( or \\("
    EParen = 8,
    /// REG_EBRACE: "Unmatched \\{"
    EBrace = 9,
    /// REG_BADBR: "Invalid content of \\{\\}"
    BadBr = 10,
    /// REG_ERANGE: "Invalid range end"
    ERange = 11,
    /// REG_ESPACE: "Memory exhausted"
    ESpace = 12,
    /// REG_BADRPT: "Invalid preceding regular expression"
    BadRpt = 13,
    /// REG_EEND: "Premature end of regular expression" (never produced)
    EEnd = 14,
    /// REG_ESIZE: "Regular expression too big" (never produced)
    ESize = 15,
    /// REG_ERPAREN: "Unmatched ) or \\)" (regcomp maps it to REG_EPAREN)
    ERParen = 16,
}

impl RegError {
    fn from_code(code: i32) -> RegError {
        match code {
            2 => RegError::BadPat,
            3 => RegError::ECollate,
            4 => RegError::ECtype,
            5 => RegError::EEscape,
            6 => RegError::ESubreg,
            7 => RegError::EBrack,
            8 => RegError::EParen,
            9 => RegError::EBrace,
            10 => RegError::BadBr,
            11 => RegError::ERange,
            12 => RegError::ESpace,
            13 => RegError::BadRpt,
            14 => RegError::EEnd,
            15 => RegError::ESize,
            16 => RegError::ERParen,
            _ => RegError::BadPat,
        }
    }

    /// The numeric `reg_errcode_t` value.
    pub fn code(self) -> i32 {
        self as i32
    }

    /// The `regerror` message text.
    pub fn message(self) -> &'static str {
        ERROR_MESSAGES[self as usize]
    }
}

impl std::fmt::Display for RegError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for RegError {}

/// A compiled regular expression (`regex_t`).
///
/// As in C, executing a pattern extends the DFA state cache stored in the
/// compiled pattern; the cache lives in a `RefCell`, so a `Regex` is not
/// `Sync`.
pub struct Regex {
    comp: compile::Compiled,
    cache: RefCell<internal::StateCache>,
}

impl std::fmt::Debug for Regex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Regex").field("re_nsub", &self.comp.re_nsub).finish()
    }
}

/// Recursion budget for compiling on the caller's stack (sized for 1 MB).
const INLINE_DEPTH: u32 = 1000;

/// Stack reserved for compiling/matching patterns that may recurse deeply.
/// Only address space is reserved; pages are committed on use.
const BIG_STACK: usize = 1 << 30;

fn on_big_stack<R: Send, F: FnOnce() -> R + Send>(f: F) -> R {
    std::thread::scope(|s| {
        match std::thread::Builder::new().stack_size(BIG_STACK).spawn_scoped(s, f) {
            Ok(h) => h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)),
            Err(e) => panic!("cannot start regex worker thread: {e}"),
        }
    })
}

/// `strlen` semantics: the C API sees the bytes up to the first NUL.
fn c_str(s: &[u8]) -> &[u8] {
    match s.iter().position(|&b| b == 0) {
        Some(n) => &s[..n],
        None => s,
    }
}

impl Regex {
    /// `regcomp(preg, pattern, cflags)`.
    ///
    /// The pattern is truncated at its first NUL byte, as C's `strlen` would.
    pub fn compile(pattern: &[u8], cflags: i32) -> Result<Regex, RegError> {
        let pattern = c_str(pattern);
        let mut syntax = if cflags & REG_EXTENDED != 0 {
            internal::RE_SYNTAX_POSIX_EXTENDED
        } else {
            internal::RE_SYNTAX_POSIX_BASIC
        };
        if cflags & REG_ICASE != 0 {
            syntax |= internal::RE_ICASE;
        }
        let newline_anchor;
        if cflags & REG_NEWLINE != 0 {
            // REG_NEWLINE implies neither . nor [^...] match newline.
            syntax &= !internal::RE_DOT_NEWLINE;
            syntax |= internal::RE_HAT_LISTS_NOT_NEWLINE;
            // It also changes the matching behavior.
            newline_anchor = true;
        } else {
            newline_anchor = false;
        }
        let no_sub = cflags & REG_NOSUB != 0;

        // The C parser and closure computation are recursive (pdfTeX's
        // 5 MB main-thread stack copes with 10000 nested groups).  Compile in
        // place with a recursion budget that fits a 1 MB stack; if the
        // pattern needs more, compile again (from scratch, so the result is
        // identical) on a thread with a large stack.
        let mut result = compile::re_compile_internal(pattern, syntax, no_sub, newline_anchor, INLINE_DEPTH);
        if result.as_ref().err() == Some(&compile::REG_NEED_STACK) {
            result = on_big_stack(|| compile::re_compile_internal(pattern, syntax, no_sub, newline_anchor, u32::MAX));
        }
        match result {
            Ok(mut comp) => {
                let cache = std::mem::replace(&mut comp.cache, internal::StateCache::new(0));
                Ok(Regex { comp, cache: RefCell::new(cache) })
            }
            Err(code) => {
                // POSIX doesn't distinguish between an unmatched open-group
                // and an unmatched close-group: both are REG_EPAREN.
                let code = if code == internal::REG_ERPAREN { internal::REG_EPAREN } else { code };
                Err(RegError::from_code(code))
            }
        }
    }

    /// `re_nsub`: the number of parenthesized subexpressions.
    pub fn nsub(&self) -> usize {
        self.comp.re_nsub
    }

    /// `regexec(preg, subject, nmatch, pmatch, eflags)`.
    ///
    /// Returns `None` when regexec returns non-zero (no match, or an internal
    /// error), otherwise `nmatch` `(rm_so, rm_eo)` pairs with `-1` for unset
    /// registers.  The subject is truncated at its first NUL byte.  With
    /// `REG_NOSUB` (where C leaves `pmatch` untouched) all pairs are
    /// `(-1, -1)`.
    ///
    /// # Panics
    ///
    /// Panics where the C engine does not return normally: a failed
    /// `assert()` (pdfTeX aborts) or a NULL-pointer dereference (pdfTeX
    /// crashes).  See [`Regex::try_exec`].
    pub fn exec(&self, subject: &[u8], nmatch: usize, eflags: i32) -> Option<Vec<(isize, isize)>> {
        match self.try_exec(subject, nmatch, eflags) {
            Ok(r) => r,
            Err(a) => panic!("{a}"),
        }
    }

    /// Like [`Regex::exec`], but reports the places where the C engine aborts
    /// or crashes as an [`ExecAbort`] instead of panicking.
    pub fn try_exec(&self, subject: &[u8], nmatch: usize, eflags: i32) -> Result<Option<Vec<(isize, isize)>>, ExecAbort> {
        self.try_exec_limited(subject, nmatch, eflags, u64::MAX)
    }

    /// Like [`Regex::try_exec`] with a budget on the number of
    /// back-tracking steps of the submatch phase (`set_regs`).  The C engine
    /// has no such limit and loops forever on some back-reference patterns
    /// (e.g. ERE `(a*)*(x)(\1)` against `ax`, confirmed with pdfTeX); the
    /// budget makes that observable as [`ExecAbort::StepLimit`].
    pub fn try_exec_limited(
        &self,
        subject: &[u8],
        nmatch: usize,
        eflags: i32,
        step_limit: u64,
    ) -> Result<Option<Vec<(isize, isize)>>, ExecAbort> {
        let mut pmatch = vec![(-1isize, -1isize); nmatch];
        let ret = self.regexec_impl(subject, &mut pmatch, eflags & !REG_STARTEND, step_limit)?;
        Ok(if ret == 0 { Some(pmatch) } else { None })
    }

    /// C-style `regexec`: fills `pmatch` (whose length is `nmatch`) and
    /// returns 0 on a match, `REG_NOMATCH` (1) otherwise.  As in C, `pmatch`
    /// is only written on success and when `REG_NOSUB` was not given; with
    /// `REG_STARTEND` the search range is `pmatch[0]`.
    ///
    /// # Panics
    ///
    /// Panics where the C engine aborts or crashes (see [`ExecAbort`]).
    pub fn regexec(&self, subject: &[u8], pmatch: &mut [(isize, isize)], eflags: i32) -> i32 {
        match self.try_regexec(subject, pmatch, eflags) {
            Ok(r) => r,
            Err(a) => panic!("{a}"),
        }
    }

    /// Non-panicking form of [`Regex::regexec`].
    pub fn try_regexec(&self, subject: &[u8], pmatch: &mut [(isize, isize)], eflags: i32) -> Result<i32, ExecAbort> {
        self.regexec_impl(subject, pmatch, eflags, u64::MAX)
    }

    fn regexec_impl(&self, subject: &[u8], pmatch: &mut [(isize, isize)], eflags: i32, step_limit: u64) -> Result<i32, ExecAbort> {
        use internal::*;
        if eflags & !(internal::REG_NOTBOL | internal::REG_NOTEOL | internal::REG_STARTEND) != 0 {
            return Ok(1); // regexec returns REG_BADPAT, i.e. non-zero
        }
        let (string, start, length): (&[u8], i32, i32) = if eflags & internal::REG_STARTEND != 0 {
            let (so, eo) = pmatch.first().copied().unwrap_or((0, 0));
            let eo = (eo.max(0) as usize).min(subject.len());
            (&subject[..eo], so as i32, eo as i32)
        } else {
            let s = c_str(subject);
            (s, 0, s.len() as i32)
        };
        let nmatch = if self.comp.no_sub { 0 } else { pmatch.len() };
        let mut regs = vec![exec::RegMatch { rm_so: -1, rm_eo: -1 }; nmatch];
        let mut cache = self.cache.borrow_mut();
        let cache: &mut internal::StateCache = &mut cache;
        let comp = &self.comp;
        let regs_ref = &mut regs;
        let mut run = move || {
            exec::re_search_internal(comp, cache, string, length, start, length - start, length, nmatch, regs_ref, eflags, step_limit)
        };
        // Without back-references the matcher does not recurse at all; with
        // them it may recurse deeply (see exec::SIFT_DEPTH_LIMIT), so those
        // patterns run on a big stack.
        let simple = comp.dfa.nbackref == 0;
        let err = if simple { run() } else { on_big_stack(run) }?;
        if err != REG_NOERROR {
            return Ok(1);
        }
        for (dst, r) in pmatch.iter_mut().zip(regs.iter()) {
            *dst = (r.rm_so as isize, r.rm_eo as isize);
        }
        Ok(0)
    }

    /// Convenience: does the pattern match anywhere in `subject`?
    pub fn is_match(&self, subject: &[u8]) -> bool {
        self.exec(subject, 0, 0).is_some()
    }
}

#[doc(hidden)]
impl Regex {
    /// Debug dump of the NFA (not part of the stable API).
    pub fn debug_dump(&self) -> String {
        let d = &self.comp.dfa;
        let mut s = String::new();
        for (i, n) in d.nodes.iter().enumerate() {
            s.push_str(&format!(
                "{i}: ty={} c={:?} idx={} ctx={:#x} cons={:#x} dup={} opt={} next={} edests={:?} ecl={:?}\n",
                n.ty,
                n.c as char,
                n.idx,
                n.ctx_type,
                n.constraint,
                n.duplicated,
                n.opt_subexp,
                d.nexts[i],
                d.edests[i].as_slice(),
                d.eclosures[i].as_slice()
            ));
        }
        let c = self.cache.borrow();
        for (i, st) in c.states.iter().enumerate() {
            s.push_str(&format!("state {i}: ctx={} nodes={:?} halt={} cons={}\n", st.context, st.nodes.as_slice(), st.halt, st.has_constraint));
        }
        s
    }
}

#[doc(hidden)]
/// Test hook: compile on the current stack with the given recursion budget;
/// returns Err(-100) when the budget is exceeded.
pub fn __compile_inline(pattern: &[u8], cflags: i32, depth_limit: u32) -> Result<usize, i32> {
    let syntax = if cflags & REG_EXTENDED != 0 { internal::RE_SYNTAX_POSIX_EXTENDED } else { internal::RE_SYNTAX_POSIX_BASIC };
    compile::re_compile_internal(c_str(pattern), syntax, false, false, depth_limit).map(|c| c.dfa.nodes.len())
}

#[doc(hidden)]
impl Regex {
    /// Highest register index read by a back-reference (`opr.idx + 1`
    /// after subexpression renumbering), 0 if none.  When it is >= nmatch,
    /// the C set_regs reads past the caller's pmatch array (undefined
    /// behaviour; pdfTeX hits this with a small `subcount`).
    pub fn __max_backref_register(&self) -> usize {
        self.comp
            .dfa
            .nodes
            .iter()
            .filter(|n| n.ty == internal::OP_BACK_REF)
            .map(|n| n.idx as usize + 1)
            .max()
            .unwrap_or(0)
    }
}
