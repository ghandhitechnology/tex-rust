use texrs_posix_regex::Regex;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let cflags: i32 = a[1].parse().unwrap();
    let re = Regex::compile(a[2].as_bytes(), cflags).unwrap();
    println!("{}", re.debug_dump());
    if a.len() > 3 {
        let r = re.try_exec(a[3].as_bytes(), 10, 0);
        println!("{:?}", r.map(|o| o.map(|v| v[..re.nsub() + 1].to_vec())));
        println!("{}", re.debug_dump());
    }
}
