//! Minimal compressor for smoke tests: zpipe <level> [wbits] [memlevel] [strategy] < in > out
use std::io::{Read, Write};
fn main() {
    let a: Vec<i32> = std::env::args().skip(1).map(|s| s.parse().unwrap()).collect();
    let level = *a.first().unwrap_or(&6);
    let wbits = *a.get(1).unwrap_or(&15);
    let mem = *a.get(2).unwrap_or(&8);
    let strat = *a.get(3).unwrap_or(&0);
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input).unwrap();
    let mut d = texrs_zlib::Deflate::with_params(level, 8, wbits, mem, strat).unwrap();
    let mut out = Vec::new();
    let mut buf = vec![0u8; 32768];
    let mut pos = 0;
    loop {
        let r = d.deflate(&input[pos..], &mut buf, texrs_zlib::Flush::Finish);
        pos += r.consumed;
        out.extend_from_slice(&buf[..r.produced]);
        if r.status == texrs_zlib::Status::StreamEnd { break; }
        assert!(!r.status.is_error(), "{:?}", r.status);
    }
    std::io::stdout().write_all(&out).unwrap();
}
