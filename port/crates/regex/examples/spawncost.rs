use texrs_posix_regex::{Regex, REG_EXTENDED};
fn main() {
    let n = 2000;
    for pat in [&b"^(a+)(b*)$"[..], b"x{1}(a+)", b"[0-9]+"] {
        let t = std::time::Instant::now();
        for _ in 0..n {
            let re = Regex::compile(pat, REG_EXTENDED).unwrap();
            let _ = re.exec(b"aaabbb", 10, 0);
        }
        println!("{:?}: {:?} per compile+exec", std::str::from_utf8(pat).unwrap(), t.elapsed() / n);
    }
    let t = std::time::Instant::now();
    for _ in 0..n {
        std::thread::scope(|s| { std::thread::Builder::new().stack_size(1 << 30).spawn_scoped(s, || 1).unwrap().join().unwrap() });
    }
    println!("spawn: {:?}", t.elapsed() / n);
}
