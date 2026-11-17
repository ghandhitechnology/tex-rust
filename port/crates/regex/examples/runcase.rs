//! Debug helper: reads cases "cflags\teflags\tnmatch\tpattern_hex\tsubject_hex"
//! from stdin, prints one result line per case (flushed).
use std::io::{BufRead, Write};
use texrs_posix_regex::Regex;

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let cflags: i32 = f[0].parse().unwrap();
        let eflags: i32 = f[1].parse().unwrap();
        let nmatch: usize = f[2].parse().unwrap();
        let p = unhex(f[3]);
        let s = unhex(f[4]);
        let out = match Regex::compile(&p, cflags) {
            Err(e) => format!("E:{}", e.message()),
            Ok(re) => match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| re.try_exec_limited(&s, nmatch, eflags, 300_000))) {
                Err(_) => "PANIC".to_string(),
                Ok(Err(a)) => format!("ABORT:{a:?}"),
                Ok(Ok(None)) => "0".to_string(),
                Ok(Ok(Some(v))) => {
                    let n = v.len();
                    format!("1:{}", v[..n].iter().map(|(a, b)| format!("{a},{b}")).collect::<Vec<_>>().join(";"))
                }
            },
        };
        let mut o = stdout.lock();
        writeln!(o, "{out}").unwrap();
        o.flush().unwrap();
    }
}
