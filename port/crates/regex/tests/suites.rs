//! glibc's and AT&T's regex test suites, checked against the suites' own
//! expectations (tests/data/suite_cases.tsv, produced by tools/corpus.py).
//!
//! These expectations were written for *current* glibc (or, for the AT&T
//! files, for an idealised POSIX engine), not for the glibc 2.5 engine that
//! pdfTeX bundles.  Every case where the port disagrees must be listed in
//! tests/data/suite_known_diffs.tsv together with the reason; for ERE cases
//! the pdfTeX oracle (tests/oracle.rs) independently confirms that the C
//! engine behaves like the port.

mod common;
use common::*;
use std::collections::BTreeMap;
use texrs_posix_regex::Regex;

/// Step budget for the submatch phase; the C engine loops forever on a few
/// back-reference patterns (confirmed with pdfTeX), which the port reproduces.
const STEP_LIMIT: u64 = 300_000;

fn check_spec(spec: &str, reg: (isize, isize), subject: &[u8]) -> bool {
    let (so, eo) = reg;
    if spec == "?" {
        return true;
    }
    if spec == "-" {
        return so == -1 && eo == -1;
    }
    if let Some(t) = spec.strip_prefix("T=") {
        let t = unhex(t);
        return so != -1 && eo != -1 && eo >= so && (eo - so) as usize == t.len() && subject.get(so as usize..eo as usize) == Some(&t[..]);
    }
    if let Some(t) = spec.strip_prefix('@') {
        let t = unhex(t);
        if so == -1 || eo == -1 || so != eo {
            return false;
        }
        // strncmp (string + so, match + 1, strlen (match + 1) ?: 1)
        let mut rest: Vec<u8> = subject[so as usize..].to_vec();
        rest.push(0);
        let mut want = t.clone();
        want.push(0);
        let n = if t.is_empty() { 1 } else { t.len() };
        return rest.len() >= n && want.len() >= n && rest[..n] == want[..n];
    }
    let mut it = spec.split(',');
    let a: isize = it.next().unwrap().parse().unwrap();
    let b: isize = it.next().unwrap().parse().unwrap();
    so == a && eo == b
}

/// Returns None if the case passes, otherwise a description of the outcome.
pub fn run_case(cflags: i32, eflags: i32, nmatch: usize, pattern: &[u8], subject: &[u8], expect: &str) -> Option<String> {
    let subject_c: Vec<u8> = subject.iter().copied().take_while(|&b| b != 0).collect();
    let re = Regex::compile(pattern, cflags);
    if let Some(name) = expect.strip_prefix("ERR:") {
        return match re {
            Err(e) if name == "*" || err_name(e.code()) == name => None,
            Err(e) => Some(format!("compile error {}", err_name(e.code()))),
            Ok(_) => Some("compiled".into()),
        };
    }
    let re = match re {
        Err(e) => return Some(format!("compile error {}", err_name(e.code()))),
        Ok(r) => r,
    };
    let res = match re.try_exec_limited(subject, nmatch, eflags, STEP_LIMIT) {
        Ok(r) => r,
        Err(a) => return Some(format!("ABORT:{a:?}")),
    };
    let got = match &res {
        None => "NOMATCH".to_string(),
        Some(v) => {
            let mut s = String::from("MATCH:");
            let n = v.len().min(re.nsub() + 1);
            s.push_str(&v[..n].iter().map(|(a, b)| format!("{a},{b}")).collect::<Vec<_>>().join(";"));
            s
        }
    };
    if expect == "NOMATCH" {
        return if res.is_none() { None } else { Some(got) };
    }
    let v = match res {
        None => return Some(got),
        Some(v) => v,
    };
    if let Some(specs) = expect.strip_prefix("MATCH:") {
        for (i, spec) in specs.split(';').enumerate() {
            let reg = v.get(i).copied().unwrap_or((-1, -1));
            if !check_spec(spec, reg, &subject_c) {
                return Some(got);
            }
        }
    }
    None
}

#[test]
fn glibc_and_att_suites() {
    with_big_stack(|| {
        let lines = read_lines("suite_cases.tsv");
        let known: BTreeMap<String, String> = read_lines("suite_known_diffs.tsv")
            .into_iter()
            .map(|l| {
                let mut f = l.splitn(3, '\t');
                let id = f.next().unwrap().to_string();
                let got = f.next().unwrap_or("").to_string();
                (id, got)
            })
            .collect();
        let mut pass = 0;
        let mut known_hit = 0;
        let mut per_source: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        let mut failures = Vec::new();
        let mut stale = Vec::new();
        for line in &lines {
            let f: Vec<&str> = line.split('\t').collect();
            let (id, cflags, eflags, nmatch) = (f[0], f[1].parse().unwrap(), f[2].parse().unwrap(), f[3].parse().unwrap());
            let pattern = unhex(f[4]);
            let subject = unhex(f[5]);
            let expect = f[6];
            let src = id.split(':').next().unwrap().to_string();
            let e = per_source.entry(src).or_default();
            e.1 += 1;
            match run_case(cflags, eflags, nmatch, &pattern, &subject, expect) {
                None => {
                    pass += 1;
                    e.0 += 1;
                    if known.contains_key(id) {
                        stale.push(id.to_string());
                    }
                }
                Some(got) => {
                    if let Some(k) = known.get(id) {
                        if *k == got {
                            known_hit += 1;
                            continue;
                        }
                    }
                    failures.push(format!(
                        "{id}\t{got}\t# cflags={cflags} eflags={eflags} /{}/ on \"{}\" expected {expect}",
                        show(&pattern),
                        show(&subject)
                    ));
                }
            }
        }
        for (src, (p, t)) in &per_source {
            eprintln!("  {src:<16} {p:>5}/{t:<5} pass the suite's own expectation");
        }
        eprintln!(
            "suites: {} cases, {} pass, {} documented glibc-2.5 differences, {} unexpected",
            lines.len(),
            pass,
            known_hit,
            failures.len()
        );
        for fl in &failures {
            eprintln!("{fl}");
        }
        assert!(stale.is_empty(), "known-diff entries that now pass: {stale:?}");
        assert!(failures.is_empty(), "{} unexpected suite failures", failures.len());
    });
}
