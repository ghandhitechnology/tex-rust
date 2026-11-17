//! The pdfTeX side of `\pdfmatch` / `\pdflastmatch`: a port of
//! `matchstrings` and `getmatch` from `texk/web2c/pdftexdir/utils.c`
//! (TeX Live 2026).  The string-pool plumbing stays with the caller; this
//! module reproduces the regex calls and the static state kept between the
//! two primitives.

use crate::{ExecAbort, RegError, Regex, REG_EXTENDED, REG_ICASE};

/// `DEFAULT_SUB_MATCH_COUNT` in utils.c.
pub const DEFAULT_SUB_MATCH_COUNT: i32 = 10;

/// What `\pdfmatch` expands to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchOutcome {
    /// regexec succeeded: `1`.
    Match,
    /// regexec failed: `0`.
    NoMatch,
    /// regcomp failed: `-1`.  pdfTeX also issues the warning
    /// `\pdfmatch: <message>` (see [`MatchOutcome::warning`]); the
    /// `\pdflastmatch` state is left untouched.
    CompileError(RegError),
}

impl MatchOutcome {
    /// The characters pdfTeX appends to the string pool.
    pub fn as_bytes(&self) -> &'static [u8] {
        match self {
            MatchOutcome::Match => b"1",
            MatchOutcome::NoMatch => b"0",
            MatchOutcome::CompileError(_) => b"-1",
        }
    }

    /// The text pdfTeX passes to `pdftex_warn`, if any.
    pub fn warning(&self) -> Option<String> {
        match self {
            MatchOutcome::CompileError(e) => Some(format!("\\pdfmatch: {}", e.message())),
            _ => None,
        }
    }
}

/// The static variables of utils.c used by `\pdfmatch` and `\pdflastmatch`
/// (`sub_match_count`, `pmatch`, `match_string`, `last_match_succeeded`).
#[derive(Clone, Debug, Default)]
pub struct MatchState {
    sub_match_count: i32,
    pmatch: Vec<(isize, isize)>,
    match_string: Option<Vec<u8>>,
    last_match_succeeded: bool,
}

impl MatchState {
    pub fn new() -> MatchState {
        MatchState::default()
    }

    /// `matchstrings(s, t, subcount, icase)`: `pattern` and `subject` are
    /// the string-pool contents (pdfTeX hands them to the C library as C
    /// strings, so they end at the first NUL).  `subcount < 0` means no
    /// `subcount` keyword was given.
    ///
    /// Returns `Err` where the C engine aborts or crashes pdfTeX.
    pub fn match_strings(&mut self, pattern: &[u8], subject: &[u8], subcount: i32, icase: bool) -> Result<MatchOutcome, ExecAbort> {
        let cflags = REG_EXTENDED | if icase { REG_ICASE } else { 0 };
        let re = match Regex::compile(pattern, cflags) {
            Err(e) => return Ok(MatchOutcome::CompileError(e)),
            Ok(r) => r,
        };
        let subject: &[u8] = match subject.iter().position(|&b| b == 0) {
            Some(n) => &subject[..n],
            None => subject,
        };
        self.sub_match_count = if subcount < 0 { DEFAULT_SUB_MATCH_COUNT } else { subcount };
        let nmatch = self.sub_match_count.max(0) as usize;
        // pdfTeX allocates pmatch uninitialised; it is only read after a
        // successful regexec, which writes all nmatch entries.
        self.pmatch = vec![(-1, -1); nmatch];
        let ret = re.try_regexec(subject, &mut self.pmatch, 0)?;
        self.match_string = Some(subject.to_vec());
        self.last_match_succeeded = ret == 0;
        Ok(if ret == 0 { MatchOutcome::Match } else { MatchOutcome::NoMatch })
    }

    /// `getmatch(i)`: the expansion of `\pdflastmatch i`, i.e.
    /// `<rm_so>-><matched text>` or `-1->`.
    pub fn get_match(&self, i: i32) -> Vec<u8> {
        let found = i >= 0
            && i < self.sub_match_count
            && self.match_string.is_some()
            && self.last_match_succeeded
            && self.pmatch[i as usize].0 >= 0
            && self.pmatch[i as usize].1 >= self.pmatch[i as usize].0;
        if !found {
            return b"-1->".to_vec();
        }
        let (so, eo) = self.pmatch[i as usize];
        let s = self.match_string.as_ref().unwrap();
        let mut out = format!("{so}->").into_bytes();
        out.extend_from_slice(&s[so as usize..eo as usize]);
        out
    }
}
