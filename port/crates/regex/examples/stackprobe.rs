fn main() {
    let a: Vec<String> = std::env::args().collect();
    let p = &a[1];
    let r = texrs_posix_regex::__compile_inline(p.as_bytes(), 1, 1000);
    println!("len {} -> {:?}", p.len(), r);
}
