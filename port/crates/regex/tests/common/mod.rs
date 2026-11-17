#![allow(dead_code)]

use std::path::PathBuf;

pub fn data_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("data");
    p.push(name);
    p
}

pub fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0, "bad hex {s:?}");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex digit"))
        .collect()
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn show(b: &[u8]) -> String {
    let mut s = String::new();
    for &c in b {
        match c {
            b'\n' => s.push_str("\\n"),
            b'\t' => s.push_str("\\t"),
            b'\\' => s.push_str("\\\\"),
            0x20..=0x7e => s.push(c as char),
            _ => s.push_str(&format!("\\x{c:02x}")),
        }
    }
    s
}

pub fn read_lines(name: &str) -> Vec<String> {
    let text = std::fs::read_to_string(data_path(name)).unwrap_or_else(|e| panic!("read {name}: {e}"));
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.to_string())
        .collect()
}

pub fn err_name(code: i32) -> &'static str {
    [
        "NOERROR", "NOMATCH", "BADPAT", "ECOLLATE", "ECTYPE", "EESCAPE", "ESUBREG", "EBRACK", "EPAREN", "EBRACE",
        "BADBR", "ERANGE", "ESPACE", "BADRPT", "EEND", "ESIZE", "ERPAREN",
    ][code as usize]
}

/// Run a stack-hungry closure on a thread with a large stack (the C engine
/// is recursive; so is the port).
pub fn with_big_stack<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::Builder::new().stack_size(256 << 20).spawn(f).unwrap().join().unwrap();
}
