use texrs_posix_regex::Regex;
fn unhex(s: &str) -> Vec<u8> { (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cflags: i32 = a[1].parse().unwrap();
    let p = unhex(&a[2]);
    let s = unhex(&a[3]);
    let nmatch: usize = a[4].parse().unwrap();
    let t = std::time::Instant::now();
    let re = Regex::compile(&p, cflags);
    eprintln!("compile {:?} ok={}", t.elapsed(), re.is_ok());
    if let Ok(re) = re {
        eprintln!("{}", re.debug_dump().lines().filter(|l| !l.starts_with("state")).count());
        let t = std::time::Instant::now();
        let r = re.try_exec_limited(&s, nmatch, 0, 300_000);
        eprintln!("exec {:?} {:?}", t.elapsed(), r);
    }
}
