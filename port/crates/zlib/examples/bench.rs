//! Throughput benchmark: `cargo run --release --example bench -- <file> [reps]`
//!
//! Compresses the file with deflate at levels 1, 6 and 9 (zlib wrapper,
//! windowBits 15, memLevel 8, one call with a buffer from compressBound) and
//! inflates the result, reporting the best of `reps` runs. Compare with
//! tools/bench_python.py, which does the same with CPython's zlib (C zlib
//! 1.3.2).

use std::time::Instant;
use texrs_zlib::{compress_bound, crc32, Deflate, Flush, Inflate, Status};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: bench <file> [reps]");
    let reps: usize = args.get(2).map_or(5, |s| s.parse().unwrap());
    let data = std::fs::read(path).unwrap();
    println!("input {} bytes, crc {:08x}", data.len(), crc32(0, &data));
    for level in [1, 6, 9] {
        let mut out = vec![0u8; compress_bound(data.len() as u64) as usize];
        let mut best = f64::MAX;
        let mut clen = 0;
        for _ in 0..reps {
            let t = Instant::now();
            let mut d = Deflate::new(level).unwrap();
            let r = d.deflate(&data, &mut out, Flush::Finish);
            assert_eq!(r.status, Status::StreamEnd);
            clen = r.produced;
            best = best.min(t.elapsed().as_secs_f64());
        }
        let comp = out[..clen].to_vec();
        let mut back = vec![0u8; data.len()];
        let mut ibest = f64::MAX;
        for _ in 0..reps {
            let t = Instant::now();
            let mut inf = Inflate::new().unwrap();
            let r = inf.inflate(&comp, &mut back, Flush::Finish);
            assert_eq!(r.status, Status::StreamEnd);
            ibest = ibest.min(t.elapsed().as_secs_f64());
        }
        assert!(back == data);
        println!(
            "level {}: {} bytes (crc {:08x})  deflate {:.1} ms ({:.1} MB/s)  inflate {:.1} ms ({:.1} MB/s)",
            level,
            clen,
            crc32(0, &comp),
            best * 1e3,
            data.len() as f64 / best / 1e6,
            ibest * 1e3,
            data.len() as f64 / ibest / 1e6
        );
    }
}
