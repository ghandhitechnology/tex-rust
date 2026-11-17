//! Differential test against the real C engine inside TeX Live 2026's
//! pdfTeX (pdftex.dll, pdfTeX 1.40.29).
//!
//! tests/data/oracle*.tsv were produced by tools/oracle.py, which ran every
//! case through `\pdfmatch` / `\pdflastmatch` / `\pdfescapehex` in an
//! ini-mode pdftex.exe.  For each case we run the port the way pdfTeX's
//! `matchstrings` does (regcomp with REG_EXTENDED [| REG_ICASE], regexec
//! with nmatch = subcount or 10, eflags 0) and render the result the way
//! `getmatch` reports it.  The rendering must be identical, including the
//! regerror text, pdfTeX crashes (NULL dereference / failed assert in the C
//! engine) and hangs (endless back-tracking in set_regs).

mod common;
use common::*;
use texrs_posix_regex::{ExecAbort, Regex, REG_EXTENDED, REG_ICASE};

/// Budget for the submatch back-tracking phase.  The C engine never
/// returns on the cases recorded as TIMEOUT; the port must exhaust this.
const STEP_LIMIT: u64 = 300_000;

pub fn pdftex_result(icase: bool, subcount: i32, pattern: &[u8], subject: &[u8]) -> String {
    let cflags = REG_EXTENDED | if icase { REG_ICASE } else { 0 };
    let re = match Regex::compile(pattern, cflags) {
        Err(e) => return format!("E:{}", e.message()),
        Ok(r) => r,
    };
    let nmatch = if subcount < 0 { 10 } else { subcount as usize };
    match re.try_exec_limited(subject, nmatch, 0, STEP_LIMIT) {
        // Never returns in pdfTeX either.
        Err(ExecAbort::StepLimit) => "TIMEOUT".into(),
        // pdftex.dll's runtime shows its "Microsoft Visual C++ Runtime
        // Library" assertion dialog and then aborts.
        Err(ExecAbort::Assertion { .. }) => "ASSERT".into(),
        // Access violation: pdfTeX dies.
        Err(ExecAbort::NullState { .. }) => "CRASH".into(),
        Ok(None) => "0".into(),
        Ok(Some(v)) => {
            // getmatch(): found iff i < sub_match_count && rm_so >= 0 &&
            // rm_eo >= rm_so.
            let regs: Vec<String> = v
                .iter()
                .map(|&(so, eo)| if so >= 0 && eo >= so { format!("{so},{eo}") } else { "-".into() })
                .collect();
            format!("1:{}", regs.join(";"))
        }
    }
}

/// True when regexec would read past pdfTeX's pmatch array (a
/// back-reference to a register >= nmatch): undefined behaviour in C, whose
/// outcome depends on heap contents (it differs between pdfTeX runs).
fn reads_past_pmatch(icase: bool, subcount: i32, pattern: &[u8]) -> bool {
    let cflags = REG_EXTENDED | if icase { REG_ICASE } else { 0 };
    let nmatch = if subcount < 0 { 10 } else { subcount as usize };
    match Regex::compile(pattern, cflags) {
        Ok(re) => nmatch > 1 && re.__max_backref_register() >= nmatch,
        Err(_) => false,
    }
}

fn run_file(name: &str) -> (usize, Vec<String>, std::collections::BTreeMap<String, usize>) {
    let lines = read_lines(name);
    let mut fails = Vec::new();
    let mut kinds = std::collections::BTreeMap::new();
    for line in &lines {
        let f: Vec<&str> = line.split('\t').collect();
        let (id, icase, sub) = (f[0], f[1] == "1", f[2].parse::<i32>().unwrap());
        let pattern = unhex(f[3]);
        let subject = unhex(f[4]);
        let want = f[5];
        let kind = if want.starts_with("E:") {
            "regcomp error"
        } else if want == "0" {
            "no match"
        } else if want.starts_with("1:") {
            "match"
        } else {
            want
        };
        if reads_past_pmatch(icase, sub, &pattern) {
            *kinds.entry("not compared (C reads past pmatch)".to_string()).or_insert(0) += 1;
            continue;
        }
        *kinds.entry(kind.to_string()).or_insert(0) += 1;
        let got = pdftex_result(icase, sub, &pattern, &subject);
        if got != want {
            fails.push(format!(
                "{id}: icase={} subcount={sub} /{}/ on \"{}\"\n    pdftex: {want}\n    port:   {got}",
                icase as i32,
                show(&pattern),
                show(&subject)
            ));
        }
    }
    (lines.len(), fails, kinds)
}

fn check(name: &str) {
    let name = name.to_string();
    with_big_stack(move || {
        let (n, fails, kinds) = run_file(&name);
        eprintln!("{name}: {n} cases {kinds:?}, {} mismatches", fails.len());
        for f in fails.iter().take(60) {
            eprintln!("{f}");
        }
        assert!(n > 0);
        assert!(fails.is_empty(), "{} of {n} cases differ from pdfTeX", fails.len());
    });
}

#[test]
fn oracle_main() {
    check("oracle_main.tsv");
}

#[test]
fn oracle_backref() {
    check("oracle_backref.tsv");
}

#[test]
fn oracle_regressions() {
    check("oracle_extra.tsv");
}
