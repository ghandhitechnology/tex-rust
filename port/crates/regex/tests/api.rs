//! API-level tests: error texts, flags, C-string semantics, the pdfTeX
//! helper, and behaviours confirmed with pdfTeX that are worth pinning down
//! by name.

use texrs_posix_regex::pdftex::{MatchOutcome, MatchState};
use texrs_posix_regex::*;

fn ere(p: &str) -> Regex {
    Regex::compile(p.as_bytes(), REG_EXTENDED).unwrap()
}

#[test]
fn regerror_table() {
    let expect = [
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
    for (i, m) in expect.iter().enumerate() {
        assert_eq!(regerror(i as i32), Some(*m));
    }
    assert_eq!(regerror(17), None);
    assert_eq!(regerror(-1), None);
}

#[test]
fn compile_errors() {
    let cases: &[(&str, i32, RegError)] = &[
        ("(", REG_EXTENDED, RegError::EParen),
        ("a\\", REG_EXTENDED, RegError::EEscape),
        ("[", REG_EXTENDED, RegError::BadPat),
        ("[a", REG_EXTENDED, RegError::EBrack),
        ("[[:foo:]]", REG_EXTENDED, RegError::ECtype),
        ("[[.ab.]]", REG_EXTENDED, RegError::ECollate),
        ("[b-a]", REG_EXTENDED, RegError::ERange),
        ("a{1", REG_EXTENDED, RegError::EBrace),
        ("a{2,1}", REG_EXTENDED, RegError::BadBr),
        ("a{32768}", REG_EXTENDED, RegError::BadBr),
        ("*a", REG_EXTENDED, RegError::BadRpt),
        ("(a)\\2", REG_EXTENDED, RegError::ESubreg),
        ("\\(a", 0, RegError::EParen),
        ("a\\)", 0, RegError::EParen),
        ("a\\{1", 0, RegError::EBrace),
        ("\\{1\\}", 0, RegError::BadRpt),
    ];
    for &(p, f, e) in cases {
        assert_eq!(Regex::compile(p.as_bytes(), f).unwrap_err(), e, "pattern {p:?}");
    }
    assert_eq!(RegError::EParen.message(), "Unmatched ( or \\(");
    assert_eq!(RegError::EParen.code(), 8);
}

#[test]
fn submatches() {
    let re = ere("(a+)(b*)");
    assert_eq!(re.nsub(), 2);
    assert_eq!(re.exec(b"xaabbc", 3, 0), Some(vec![(1, 5), (1, 3), (3, 5)]));
    // Extra registers are unset.
    assert_eq!(re.exec(b"xaabbc", 5, 0).unwrap()[3..], [(-1, -1), (-1, -1)]);
    // nmatch smaller than nsub+1.
    assert_eq!(re.exec(b"xaabbc", 1, 0), Some(vec![(1, 5)]));
    assert_eq!(re.exec(b"xyz", 3, 0), None);
    // POSIX leftmost-longest with alternation.
    assert_eq!(ere("(a|ab)(c|bcd)(d*)").exec(b"abcd", 4, 0), Some(vec![(0, 4), (0, 1), (1, 4), (4, 4)]));
}

#[test]
fn c_string_semantics() {
    // regcomp/regexec see C strings: everything after a NUL is ignored.
    let re = Regex::compile(b"ab\0zzz", REG_EXTENDED).unwrap();
    assert_eq!(re.exec(b"xab\0cd", 1, 0), Some(vec![(1, 3)]));
    assert_eq!(ere("d").exec(b"abc\0d", 1, 0), None);
}

#[test]
fn nosub_and_regexec() {
    let re = Regex::compile(b"(b)", REG_EXTENDED | REG_NOSUB).unwrap();
    let mut pm = [(7isize, 7isize); 2];
    assert_eq!(re.regexec(b"abc", &mut pm, 0), 0);
    assert_eq!(pm, [(7, 7), (7, 7)]); // untouched, as in C
    let re = ere("(b)");
    assert_eq!(re.regexec(b"abc", &mut pm, 0), 0);
    assert_eq!(pm, [(1, 2), (1, 2)]);
    pm = [(9, 9), (9, 9)];
    assert_eq!(re.regexec(b"xyz", &mut pm, 0), REG_NOMATCH);
    assert_eq!(pm, [(9, 9), (9, 9)]); // untouched on failure
    // Unknown eflags: regexec returns REG_BADPAT (non-zero).
    assert_eq!(re.regexec(b"abc", &mut pm, 8), 1);
    // REG_STARTEND searches pmatch[0], offsets relative to the string.
    let mut pm = [(2isize, 5isize)];
    assert_eq!(ere("b").regexec(b"abcabc", &mut pm, REG_STARTEND), 0);
    assert_eq!(pm, [(4, 5)]);
}

#[test]
fn eflags_and_newline() {
    let re = ere("^a");
    assert!(re.exec(b"ab", 1, 0).is_some());
    assert!(re.exec(b"ab", 1, REG_NOTBOL).is_none());
    let re = ere("a$");
    assert!(re.exec(b"ba", 1, REG_NOTEOL).is_none());
    // REG_NEWLINE: anchors at newlines, '.' excludes newline.
    let re = Regex::compile(b"^b$", REG_EXTENDED | REG_NEWLINE).unwrap();
    assert_eq!(re.exec(b"a\nb\nc", 1, 0), Some(vec![(2, 3)]));
    assert!(ere("^b$").exec(b"a\nb\nc", 1, 0).is_none());
    let re = Regex::compile(b"a.b", REG_EXTENDED | REG_NEWLINE).unwrap();
    assert!(re.exec(b"a\nb", 1, 0).is_none());
    // glibc 2.5 quirk kept: under REG_NEWLINE, [^...] still matches '\n'
    // (the bracket parser excludes '\0' instead of '\n').
    let re = Regex::compile(b"a[^x]b", REG_EXTENDED | REG_NEWLINE).unwrap();
    assert!(re.exec(b"a\nb", 1, 0).is_some());
}

#[test]
fn quirks_confirmed_with_pdftex() {
    // Escaped letters never match under REG_ICASE (pattern keeps raw case,
    // the subject is upper-cased).
    let re = Regex::compile(b"\\a", REG_EXTENDED | REG_ICASE).unwrap();
    assert!(re.exec(b"a", 1, 0).is_none());
    assert!(re.exec(b"A", 1, 0).is_none());
    // Without REG_NEWLINE a '^' after an embedded newline still matches.
    assert_eq!(ere("a.^b").exec(b"a\nb", 1, 0), Some(vec![(0, 3)]));
    // C-locale classes: bytes >= 0x80 are in no class.
    assert!(ere("[[:alpha:]]").exec(b"\xe9", 1, 0).is_none());
    // Ranges are upper-cased under REG_ICASE: [Z-a] becomes [Z-A].
    assert_eq!(Regex::compile(b"[Z-a]", REG_EXTENDED | REG_ICASE).unwrap_err(), RegError::ERange);
    assert!(Regex::compile(b"[Z-a]", REG_EXTENDED).is_ok());
}

#[test]
fn engine_failures() {
    // The C engine dereferences a NULL state here; pdfTeX crashes.
    let re = ere("o$($|.)");
    assert!(matches!(re.try_exec(b"o\n", 10, 0), Err(ExecAbort::NullState { .. })));
    // ...but not with nmatch <= 1 (no state log).
    assert_eq!(re.try_exec(b"o\n", 1, 0), Ok(Some(vec![(0, 2)])));
    // The C engine back-tracks forever here (pdfTeX hangs).
    let re = ere("(a*)*(x)(\\1)");
    assert_eq!(re.try_exec_limited(b"ax", 10, 0, 100_000), Err(ExecAbort::StepLimit));
}

#[test]
fn deep_nesting() {
    // pdfTeX copes with 10000 nested groups; the port must too.
    let d = 10000;
    let p = format!("{}a{}", "(".repeat(d), ")".repeat(d));
    let re = ere(&p);
    assert_eq!(re.nsub(), d);
    assert_eq!(re.exec(b"xa", 2, 0), Some(vec![(1, 2), (1, 2)]));
    let p = "()".repeat(20000);
    assert!(ere(&p).exec(b"a", 1, 0).is_some());
}

#[test]
fn pdftex_helper() {
    let mut st = MatchState::new();
    assert_eq!(st.get_match(0), b"-1->");
    assert_eq!(st.match_strings(b"(a+)(b*)", b"xaabbc", -1, false), Ok(MatchOutcome::Match));
    assert_eq!(st.get_match(0), b"1->aabb");
    assert_eq!(st.get_match(1), b"1->aa");
    assert_eq!(st.get_match(2), b"3->bb");
    assert_eq!(st.get_match(3), b"-1->");
    assert_eq!(st.get_match(10), b"-1->");
    // A compile error leaves the previous match state alone.
    let r = st.match_strings(b"(", b"x", -1, false).unwrap();
    assert_eq!(r.as_bytes(), b"-1");
    assert_eq!(r.warning().unwrap(), "\\pdfmatch: Unmatched ( or \\(");
    assert_eq!(st.get_match(1), b"1->aa");
    // subcount limits the registers that \pdflastmatch can see.
    assert_eq!(st.match_strings(b"(B)", b"abc", 1, true), Ok(MatchOutcome::Match));
    assert_eq!(st.get_match(0), b"1->b");
    assert_eq!(st.get_match(1), b"-1->");
    assert_eq!(st.match_strings(b"z", b"abc", -1, false), Ok(MatchOutcome::NoMatch));
    assert_eq!(st.get_match(0), b"-1->");
}
