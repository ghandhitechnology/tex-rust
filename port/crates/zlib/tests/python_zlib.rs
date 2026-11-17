//! Byte-identity against CPython's own `zlib` module (zlib 1.3.2):
//! `zlib.compressobj(level, method, wbits, memLevel, strategy)`,
//! `.compress()`, `.flush(mode)`, `zlib.compress()`, `zlib.decompress()`,
//! `zlib.crc32()`, `zlib.adler32()`.
//!
//! CPython calls deflate() with its own output buffer sizes (blocks of 32K,
//! 64K, 256K, 1M, ... per call, see pycore_blocks_output_buffer.h). That
//! matters for level 0 (stored blocks depend on avail_out), so the Rust side
//! reproduces CPython's exact call sequence, making even level 0 comparable.

mod common;
use common::*;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use texrs_zlib::{adler32, crc32, Deflate, Inflate, Flush, Status};

const KB: usize = 1024;
const MB: usize = 1024 * 1024;
static BLOCKS: [usize; 17] = [
    32 * KB,
    64 * KB,
    256 * KB,
    MB,
    4 * MB,
    8 * MB,
    16 * MB,
    16 * MB,
    32 * MB,
    32 * MB,
    32 * MB,
    32 * MB,
    64 * MB,
    64 * MB,
    128 * MB,
    128 * MB,
    256 * MB,
];

/// CPython's _BlocksOutputBuffer: returns the output of one
/// compress()/flush() call, calling deflate exactly as CPython does.
fn py_call(d: &mut Deflate, data: &[u8], flush: Flush) -> Vec<u8> {
    let mut out = Vec::new();
    let mut blocks = 1usize;
    let mut avail_out = BLOCKS[0];
    let mut pos = 0usize;
    loop {
        if avail_out == 0 {
            avail_out = BLOCKS[blocks.min(BLOCKS.len() - 1)];
            blocks += 1;
        }
        let mut buf = vec![0u8; avail_out];
        let r = d.deflate(&data[pos..], &mut buf, flush);
        assert!(r.status != Status::StreamError);
        out.extend_from_slice(&buf[..r.produced]);
        pos += r.consumed;
        avail_out -= r.produced;
        if avail_out != 0 {
            break;
        }
    }
    assert_eq!(pos, data.len());
    out
}

enum Op {
    Compress(usize, usize),
    Flush(i32),
}

struct Spec {
    data: String,
    level: i32,
    wbits: i32,
    mem: i32,
    strategy: i32,
    ops: Vec<Op>,
    oneshot: bool,
}

fn rust_side(spec: &Spec, data: &[u8]) -> Vec<u8> {
    let mut d = Deflate::with_params(spec.level, 8, spec.wbits, spec.mem, spec.strategy).unwrap();
    if spec.oneshot {
        // zlib.compress(): same loop with Z_FINISH once all input is given
        return py_call(&mut d, data, Flush::Finish);
    }
    let mut out = Vec::new();
    for op in &spec.ops {
        match *op {
            Op::Compress(a, b) => out.extend(py_call(&mut d, &data[a..b], Flush::NoFlush)),
            Op::Flush(0) => {}
            Op::Flush(m) => {
                let f = match m {
                    1 => Flush::PartialFlush,
                    2 => Flush::SyncFlush,
                    3 => Flush::FullFlush,
                    4 => Flush::Finish,
                    5 => Flush::Block,
                    _ => unreachable!(),
                };
                out.extend(py_call(&mut d, &[], f));
            }
        }
    }
    out
}

const PY: &str = r#"
import sys, zlib
spec = sys.argv[1]
assert zlib.ZLIB_RUNTIME_VERSION == "1.3.2", zlib.ZLIB_RUNTIME_VERSION
data = {}
for line in open(spec):
    t = line.split()
    if t[0] == "data":
        data[t[1]] = open(t[2], "rb").read()
        continue
    name, did, level, wbits, mem, strat, oneshot = t[:7]
    d = data[did]
    level, wbits, mem, strat = int(level), int(wbits), int(mem), int(strat)
    if oneshot == "1":
        out = zlib.compress(d, level, wbits)
    else:
        c = zlib.compressobj(level, zlib.DEFLATED, wbits, mem, strat)
        parts = []
        for op in t[7:]:
            k, a, *b = op.split(":")
            if k == "c":
                parts.append(c.compress(d[int(a):int(b[0])]))
            else:
                parts.append(c.flush(int(a)))
        out = b"".join(parts)
    # the stream must decompress to the input
    dw = 47 if wbits > 15 else (wbits + 32 if wbits > 0 else wbits)
    assert zlib.decompress(out, dw) == d, name
    open(sys.argv[2] + "/" + name + ".py.bin", "wb").write(out)
print("checks", zlib.crc32(data["text"]), zlib.adler32(data["text"]), zlib.crc32(data["random"], 12345), zlib.adler32(data["random"], 0x12345678))
"#;

#[test]
fn cpython_zlib_module() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cpython-zlib");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let inputs = standard_inputs(true);
    let mut spec_txt = String::new();
    for (id, d) in &inputs {
        let p = dir.join(format!("{}.bin", id));
        fs::write(&p, d).unwrap();
        let _ = writeln!(spec_txt, "data {} {}", id, p.to_str().unwrap().replace('\\', "/"));
    }
    let get = |id: &str| -> &Vec<u8> { &inputs.iter().find(|(i, _)| *i == id).unwrap().1 };
    let mut specs: Vec<(String, Spec)> = Vec::new();
    let mut rng = Rng::new(97);

    // all levels x strategies x inputs: compress(all) + flush(Z_FINISH)
    for (id, d) in &inputs {
        for level in 0..=9 {
            for strategy in 0..=4 {
                specs.push((
                    format!("ls-{}-{}-{}", id, level, strategy),
                    Spec {
                        data: id.to_string(),
                        level,
                        wbits: 15,
                        mem: 8,
                        strategy,
                        ops: vec![Op::Compress(0, d.len()), Op::Flush(4)],
                        oneshot: false,
                    },
                ));
            }
        }
    }
    // windowBits 9..15 raw/zlib/gzip x memLevel 1..9
    for id in ["text", "far", "random", "pdf"] {
        let n = get(id).len();
        for wbits in 9..=15 {
            for w in [-wbits, wbits, wbits + 16] {
                for mem in 1..=9 {
                    for &level in &[0, 1, 6, 9] {
                        specs.push((
                            format!("wm-{}-{}-{}-{}", id, w, mem, level),
                            Spec {
                                data: id.to_string(),
                                level,
                                wbits: w,
                                mem,
                                strategy: 0,
                                ops: vec![Op::Compress(0, n), Op::Flush(4)],
                                oneshot: false,
                            },
                        ));
                    }
                }
            }
        }
    }
    // chunked compress() calls and mixed flush modes
    for k in 0..400 {
        let (id, d) = rng.pick(&inputs).clone();
        let n = d.len();
        let mut ops = Vec::new();
        let mut pos = 0;
        let mixed = k % 2 == 1;
        while pos < n {
            let step = match rng.below(3) {
                0 => rng.range(1, 100),
                1 => rng.range(1, 10_000),
                _ => rng.range(1, 400_000),
            } as usize;
            let e = (pos + step).min(n);
            ops.push(Op::Compress(pos, e));
            pos = e;
            if mixed && rng.below(2) == 0 {
                ops.push(Op::Flush(*rng.pick(&[1, 2, 3, 5, 2])));
            }
        }
        ops.push(Op::Flush(4));
        specs.push((
            format!("ck-{}", k),
            Spec {
                data: id.to_string(),
                level: rng.below(11) as i32 - 1,
                wbits: *rng.pick(&[15, 15, -15, 31, 12]),
                mem: *rng.pick(&[8, 8, 1, 9]),
                strategy: if rng.below(3) == 0 { rng.below(5) as i32 } else { 0 },
                ops,
                oneshot: false,
            },
        ));
    }
    // zlib.compress() one-shot
    for (id, _) in &inputs {
        for level in -1..=9 {
            for &w in &[15, -15, 31, 9] {
                specs.push((
                    format!("os-{}-{}-{}", id, level, w),
                    Spec {
                        data: id.to_string(),
                        level,
                        wbits: w,
                        mem: 8,
                        strategy: 0,
                        ops: vec![],
                        oneshot: true,
                    },
                ));
            }
        }
    }

    for (name, s) in &specs {
        let _ = write!(
            spec_txt,
            "{} {} {} {} {} {} {}",
            name, s.data, s.level, s.wbits, s.mem, s.strategy, s.oneshot as i32
        );
        for op in &s.ops {
            match op {
                Op::Compress(a, b) => {
                    let _ = write!(spec_txt, " c:{}:{}", a, b);
                }
                Op::Flush(m) => {
                    let _ = write!(spec_txt, " f:{}", m);
                }
            }
        }
        spec_txt.push('\n');
    }
    let spec_path = dir.join("spec.txt");
    fs::write(&spec_path, &spec_txt).unwrap();
    let out = Command::new("python")
        .arg("-I")
        .arg("-c")
        .arg(PY)
        .arg(&spec_path)
        .arg(&dir)
        .output()
        .expect("python");
    assert!(
        out.status.success(),
        "python failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // checksums
    let line = String::from_utf8_lossy(&out.stdout);
    let v: Vec<u64> = line.split_whitespace().skip(1).map(|x| x.parse().unwrap()).collect();
    assert_eq!(v[0], crc32(0, get("text")) as u64);
    assert_eq!(v[1], adler32(1, get("text")) as u64);
    assert_eq!(v[2], crc32(12345, get("random")) as u64);
    assert_eq!(v[3], adler32(0x1234_5678, get("random")) as u64);

    let mut bytes = 0usize;
    for (name, s) in &specs {
        let py = fs::read(dir.join(format!("{}.py.bin", name))).unwrap();
        let mine = rust_side(s, get(&s.data));
        assert!(
            py == mine,
            "{}: CPython {} bytes, texrs {} bytes, first diff {:?}",
            name,
            py.len(),
            mine.len(),
            py.iter().zip(mine.iter()).position(|(a, b)| a != b)
        );
        // and texrs inflates it back
        let iw = if s.wbits > 15 { 47 } else if s.wbits > 0 { s.wbits + 32 } else { s.wbits };
        let mut inf = Inflate::with_window_bits(iw).unwrap();
        let mut back = vec![0u8; get(&s.data).len() + 1];
        let r = inf.inflate(&mine, &mut back, Flush::Finish);
        assert_eq!(r.status, Status::StreamEnd, "{}", name);
        assert!(back[..r.produced] == get(&s.data)[..], "{}", name);
        bytes += py.len();
        let _ = fs::remove_file(dir.join(format!("{}.py.bin", name)));
    }
    eprintln!(
        "[cpython_zlib_module] {} cases ({} compressed bytes) byte-identical to CPython zlib {}",
        specs.len(),
        bytes,
        "1.3.2"
    );
}
