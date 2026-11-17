//! Differential test harness.
//!
//! Test cases are sequences of explicit zlib calls (with explicit avail_in,
//! avail_out and flush values). Each call is executed immediately against
//! texrs-zlib (so case generators can drive loops off the results) and
//! recorded. `Harness::finish()` writes the recorded script, runs
//! tests/oracle/zoracle.py, which replays the same calls against C zlib
//! 1.3.2 through ctypes, and requires the two traces (return codes, avail_*,
//! total_*, adler, data_type, msg, CRC of each call's output) and the full
//! output bytes of every case to be identical.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use texrs_zlib::{crc32, Deflate, GzHeader, GzReader, GzWriter, Inflate};

// ------------------------------------------------------------------ data

/// xorshift64* PRNG (deterministic test data).
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }
    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len() as u64) as usize]
    }
}

const WORDS: &[&str] = &[
    "the", "of", "and", "to", "in", "is", "that", "for", "it", "as", "with", "was", "on", "be",
    "by", "this", "are", "or", "from", "at", "which", "an", "but", "not", "have", "has", "one",
    "all", "were", "when", "we", "there", "can", "been", "their", "if", "more", "will", "would",
    "about", "into", "them", "some", "could", "these", "other", "than", "then", "its", "only",
    "theorem", "proof", "lemma", "equation", "function", "matrix", "integral", "space", "group",
    "typesetting", "paragraph", "font", "glyph", "kerning", "ligature", "hyphenation", "box",
    "glue", "penalty", "TeX", "LaTeX", "pdfTeX", "document", "section", "chapter", "figure",
];

pub fn text(rng: &mut Rng, n: usize) -> Vec<u8> {
    let mut s = Vec::with_capacity(n + 32);
    let mut col = 0;
    while s.len() < n {
        let w = *rng.pick(WORDS);
        s.extend_from_slice(w.as_bytes());
        col += w.len() + 1;
        let r = rng.below(20);
        if r == 0 {
            s.extend_from_slice(b". ");
        } else if r == 1 {
            s.extend_from_slice(b", ");
        } else if col > 70 {
            s.push(b'\n');
            col = 0;
        } else {
            s.push(b' ');
        }
    }
    s.truncate(n);
    s
}

pub fn random(rng: &mut Rng, n: usize) -> Vec<u8> {
    (0..n).map(|_| rng.next() as u8).collect()
}

/// PDF content-stream-like data (text operators, paths, numbers).
pub fn pdf_like(rng: &mut Rng, n: usize) -> Vec<u8> {
    let mut s = String::new();
    let mut y = 720.0f64;
    while s.len() < n {
        match rng.below(6) {
            0 | 1 | 2 => {
                let _ = write!(
                    s,
                    "BT\n/F{} {:.4} Tf\n{:.3} {:.3} Td\n[(",
                    rng.range(1, 12),
                    [9.9626, 10.9091, 7.97, 5.9776][rng.below(4) as usize],
                    72.0 + rng.below(400) as f64 / 7.0,
                    y
                );
                for _ in 0..rng.range(1, 6) {
                    let w = *rng.pick(WORDS);
                    let _ = write!(s, "{})-{}(", w, rng.range(1, 400));
                }
                s.push_str(")]TJ\nET\n");
                y -= 11.955;
                if y < 72.0 {
                    y = 720.0;
                }
            }
            3 => {
                let _ = write!(
                    s,
                    "q 1 0 0 1 {} {} cm\n0 0 m {:.2} 0 l S\nQ\n",
                    rng.below(500),
                    rng.below(700),
                    rng.below(30000) as f64 / 100.0
                );
            }
            4 => {
                let _ = write!(s, "{:.3} g\n{:.2} w\n", rng.below(1000) as f64 / 1000.0, 0.4);
            }
            _ => {
                let _ = write!(
                    s,
                    "{} {} {} {} re f\n",
                    rng.below(600),
                    rng.below(800),
                    rng.below(50),
                    rng.below(50)
                );
            }
        }
    }
    let mut v = s.into_bytes();
    v.truncate(n);
    v
}

/// Format-file-like binary data: little-endian memory words with small
/// values, repeated structures and zero runs.
pub fn fmt_like(rng: &mut Rng, n: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(n + 16);
    while v.len() < n {
        match rng.below(5) {
            0 => {
                let k = rng.range(1, 400) as usize;
                v.extend(std::iter::repeat(0u8).take(k));
            }
            1 => {
                for _ in 0..rng.range(1, 50) {
                    v.extend_from_slice(&(rng.below(300) as u32).to_le_bytes());
                }
            }
            2 => {
                for i in 0..rng.range(1, 40) {
                    v.extend_from_slice(&((i * 4 + 0x1000) as u32).to_le_bytes());
                    v.extend_from_slice(&(rng.below(65536) as u32).to_le_bytes());
                }
            }
            3 => {
                let k = rng.range(5, 60) as usize;
                v.extend_from_slice(&text(rng, k))
            }
            _ => {
                for _ in 0..rng.range(1, 20) {
                    v.extend_from_slice(&rng.next().to_le_bytes());
                }
            }
        }
    }
    v.truncate(n);
    v
}

/// Data with matches at distances near and beyond 32K and runs.
pub fn far_matches(rng: &mut Rng, n: usize) -> Vec<u8> {
    let block = random(rng, 3000);
    let mut v = Vec::with_capacity(n);
    while v.len() < n {
        match rng.below(4) {
            0 => v.extend_from_slice(&block[..rng.range(3, 3000) as usize]),
            1 => {
                let k = rng.range(1, 33000) as usize;
                v.extend(random(rng, k));
            }
            2 => {
                let b = rng.next() as u8;
                let k = rng.range(3, 600) as usize;
                v.extend(std::iter::repeat(b).take(k));
            }
            _ => {
                if v.len() > 40000 {
                    let d = rng.range(1, 40000) as usize;
                    let start = v.len() - d;
                    let len = rng.range(3, 300) as usize;
                    for i in 0..len {
                        let b = v[start + i];
                        v.push(b);
                    }
                } else {
                    v.extend(text(rng, 100));
                }
            }
        }
    }
    v.truncate(n);
    v
}

pub fn repetitive(kind: u32, n: usize) -> Vec<u8> {
    match kind {
        0 => vec![0u8; n],
        1 => b"abc".iter().cycle().take(n).copied().collect(),
        2 => (0..n).map(|i| (i % 7) as u8 + b'a').collect(),
        _ => (0..n).map(|i| ((i / 300) % 256) as u8).collect(),
    }
}

// --------------------------------------------------------------- harness

pub fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        let _ = write!(s, "{:02x}", x);
    }
    s
}

pub fn unhex(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn msgstr(m: Option<&str>) -> String {
    match m {
        None => "-".into(),
        Some("") => "''".into(),
        Some(s) => s.replace(' ', "_"),
    }
}

pub struct Harness {
    pub name: String,
    pub dir: PathBuf,
    data: BTreeMap<String, Vec<u8>>,
    script: String,
    trace: String,
    blobs: Vec<(String, Vec<u8>)>,
    pub calls: usize,
    pub cases: usize,
}

impl Harness {
    pub fn new(name: &str) -> Harness {
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let dir = base.join(format!("oracle-{}", name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Harness {
            name: name.to_string(),
            dir,
            data: BTreeMap::new(),
            script: String::new(),
            trace: String::new(),
            blobs: Vec::new(),
            calls: 0,
            cases: 0,
        }
    }

    pub fn add_data(&mut self, id: &str, bytes: Vec<u8>) {
        let path = self.dir.join(format!("data-{}.bin", id));
        fs::write(&path, &bytes).unwrap();
        let _ = writeln!(self.script, "data {} {}", id, path.to_str().unwrap().replace('\\', "/"));
        self.data.insert(id.to_string(), bytes);
    }

    pub fn data(&self, id: &str) -> &[u8] {
        &self.data[id]
    }

    pub fn data_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("data-{}.bin", id))
    }

    pub fn case(&mut self, name: &str) -> Case<'_> {
        Case {
            name: name.to_string(),
            h: self,
            cmds: String::new(),
            trace: String::new(),
            out: Vec::new(),
            ds: None,
            is: None,
            cur: String::new(),
            gzw: None,
            gzr: None,
            calls: 0,
        }
    }

    /// Run the oracle and compare. Panics with the first difference.
    pub fn finish(self) -> (usize, usize) {
        let script_path = self.dir.join("script.txt");
        fs::write(&script_path, &self.script).unwrap();
        let oracle = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/zoracle.py");
        let status = Command::new("python")
            .arg("-I")
            .arg(&oracle)
            .arg(&script_path)
            .arg(&self.dir)
            .status()
            .expect("failed to run python oracle");
        assert!(status.success(), "oracle failed: {:?}", status);
        let otrace = fs::read_to_string(self.dir.join("oracle.trace")).unwrap();
        fs::write(self.dir.join("rust.trace"), &self.trace).unwrap();
        if otrace != self.trace {
            // report first differing line with context
            let a: Vec<&str> = otrace.lines().collect();
            let b: Vec<&str> = self.trace.lines().collect();
            let mut case = "";
            for i in 0..a.len().max(b.len()) {
                let x = a.get(i).copied().unwrap_or("<eof>");
                let y = b.get(i).copied().unwrap_or("<eof>");
                if x.starts_with("case ") {
                    case = x;
                }
                if x != y {
                    // find the command that produced this line
                    panic!(
                        "[{}] trace mismatch at line {} ({}):\n  C   : {}\n  Rust: {}\n(traces in {})",
                        self.name,
                        i + 1,
                        case,
                        x,
                        y,
                        self.dir.display()
                    );
                }
            }
        }
        for (name, blob) in &self.blobs {
            let c = fs::read(self.dir.join(format!("{}.c.bin", name))).unwrap();
            if &c != blob {
                let pos = c.iter().zip(blob.iter()).position(|(x, y)| x != y);
                panic!(
                    "[{}] output mismatch in case {}: C {} bytes, Rust {} bytes, first diff at {:?}",
                    self.name,
                    name,
                    c.len(),
                    blob.len(),
                    pos
                );
            }
        }
        eprintln!(
            "[{}] {} cases, {} calls: traces and outputs identical to C zlib 1.3.2",
            self.name, self.cases, self.calls
        );
        // clean up bulky outputs on success
        for (name, _) in &self.blobs {
            let _ = fs::remove_file(self.dir.join(format!("{}.c.bin", name)));
        }
        (self.cases, self.calls)
    }
}

/// One test case: commands are executed immediately against texrs-zlib.
pub struct Case<'a> {
    pub name: String,
    h: &'a mut Harness,
    cmds: String,
    trace: String,
    pub out: Vec<u8>,
    pub ds: Option<Deflate>,
    pub is: Option<Inflate>,
    cur: String,
    gzw: Option<GzWriter<File>>,
    gzr: Option<GzReader<File>>,
    calls: usize,
}

/// Results of one deflate()/inflate() call.
#[derive(Clone, Copy, Debug)]
pub struct Call {
    pub ret: i32,
    pub consumed: usize,
    pub produced: usize,
    pub avail_in: usize,
    pub avail_out: usize,
}

impl<'a> Case<'a> {
    fn rec(&mut self, cmd: String, result: String) {
        self.cmds.push_str(&cmd);
        self.cmds.push('\n');
        self.trace.push_str(&result);
        self.trace.push('\n');
        self.calls += 1;
    }

    pub fn data(&self, id: &str) -> &[u8] {
        self.h.data(id)
    }

    pub fn data_len(&self, id: &str) -> usize {
        self.h.data(id).len()
    }

    pub fn use_data(&mut self, id: &str) {
        self.cur = id.to_string();
        self.cmds.push_str(&format!("use {}\n", id));
    }

    pub fn cur_len(&self) -> usize {
        self.h.data(&self.cur).len()
    }

    // ---------------------------------------------------------- deflate

    pub fn dinit(&mut self, level: i32, method: i32, wbits: i32, mem: i32, strategy: i32) -> i32 {
        let r = match Deflate::with_params(level, method, wbits, mem, strategy) {
            Ok(d) => {
                self.ds = Some(d);
                0
            }
            Err(e) => {
                self.ds = None;
                e.code()
            }
        };
        self.rec(
            format!("dinit {} {} {} {} {}", level, method, wbits, mem, strategy),
            format!("dinit {} -", r),
        );
        r
    }

    fn dstate_line(&self, op: &str, r: i32, ain: usize, aout: usize, prod: &[u8]) -> String {
        let d = self.ds.as_ref().unwrap();
        format!(
            "{} {} {} {} {} {} {} {} {} {}",
            op,
            r,
            ain,
            aout,
            d.total_in(),
            d.total_out(),
            d.adler(),
            d.data_type(),
            msgstr(d.msg()),
            crc32(0, prod)
        )
    }

    pub fn d(&mut self, off: usize, ain: usize, aout: usize, flush: i32) -> Call {
        let data = self.h.data(&self.cur);
        let input = &data[off..off + ain];
        let mut buf = vec![0u8; aout];
        let r = self.ds.as_mut().unwrap().deflate_raw(input, &mut buf, flush);
        let prod = &buf[..r.produced];
        let line = self.dstate_line("d", r.status.code(), ain - r.consumed, aout - r.produced, prod);
        self.out.extend_from_slice(prod);
        self.rec(format!("d {} {} {} {}", off, ain, aout, flush), line);
        Call {
            ret: r.status.code(),
            consumed: r.consumed,
            produced: r.produced,
            avail_in: ain - r.consumed,
            avail_out: aout - r.produced,
        }
    }

    pub fn dparams(&mut self, off: usize, ain: usize, aout: usize, level: i32, strategy: i32) -> Call {
        let data = self.h.data(&self.cur);
        let input = &data[off..off + ain];
        let mut buf = vec![0u8; aout];
        let r = self.ds.as_mut().unwrap().params(input, &mut buf, level, strategy);
        let prod = &buf[..r.produced];
        let line =
            self.dstate_line("dparams", r.status.code(), ain - r.consumed, aout - r.produced, prod);
        self.out.extend_from_slice(prod);
        self.rec(format!("dparams {} {} {} {} {}", off, ain, aout, level, strategy), line);
        Call {
            ret: r.status.code(),
            consumed: r.consumed,
            produced: r.produced,
            avail_in: ain - r.consumed,
            avail_out: aout - r.produced,
        }
    }

    pub fn dreset(&mut self) {
        let d = self.ds.as_mut().unwrap();
        let r = d.reset().code();
        let line = format!(
            "dreset {} {} {} {} {} {}",
            r,
            d.total_in(),
            d.total_out(),
            d.adler(),
            d.data_type(),
            msgstr(d.msg())
        );
        self.rec("dreset".into(), line);
    }

    pub fn dresetkeep(&mut self) {
        let d = self.ds.as_mut().unwrap();
        let r = d.reset_keep().code();
        let line = format!("dresetkeep {} {} {} {}", r, d.total_in(), d.total_out(), d.adler());
        self.rec("dresetkeep".into(), line);
    }

    pub fn ddict(&mut self, id: &str, off: usize, len: usize) -> i32 {
        let dict = self.h.data(id)[off..off + len].to_vec();
        let d = self.ds.as_mut().unwrap();
        let r = d.set_dictionary(&dict).code();
        let line = format!("ddict {} {} {} {}", r, d.adler(), d.total_in(), d.total_out());
        self.rec(format!("ddict {} {} {}", id, off, len), line);
        r
    }

    pub fn dgetdict(&mut self) {
        let v = self.ds.as_ref().unwrap().get_dictionary();
        self.rec("dgetdict".into(), format!("dgetdict 0 {} {}", v.len(), crc32(0, &v)));
    }

    pub fn dprime(&mut self, bits: i32, value: i32) -> i32 {
        let r = self.ds.as_mut().unwrap().prime(bits, value).code();
        self.rec(format!("dprime {} {}", bits, value), format!("dprime {}", r));
        r
    }

    pub fn dtune(&mut self, g: i32, l: i32, n: i32, c: i32) {
        let r = self.ds.as_mut().unwrap().tune(g, l, n, c).code();
        self.rec(format!("dtune {} {} {} {}", g, l, n, c), format!("dtune {}", r));
    }

    pub fn dbound(&mut self, n: u64) {
        // deflateBound() returns uLong (saturated); the oracle calls that
        let v = texrs_zlib::zlib::ulong_saturate(self.ds.as_ref().unwrap().bound(n));
        self.rec(format!("dbound {}", n), format!("dbound {}", v));
    }

    pub fn dbound0(&mut self, n: u64) {
        let v = texrs_zlib::zlib::ulong_saturate(texrs_zlib::deflate_bound_no_stream(n));
        self.rec(format!("dbound0 {}", n), format!("dbound0 {}", v));
    }

    pub fn dpending(&mut self) {
        let (p, b) = self.ds.as_ref().unwrap().pending();
        self.rec("dpending".into(), format!("dpending 0 {} {}", p, b));
    }

    pub fn dused(&mut self) {
        let b = self.ds.as_ref().unwrap().used();
        self.rec("dused".into(), format!("dused 0 {}", b));
    }

    /// deflateSetHeader with name/comment/extra (None = absent).
    pub fn dheader(
        &mut self,
        text: i32,
        time: u32,
        os: i32,
        hcrc: i32,
        name: Option<&[u8]>,
        comment: Option<&[u8]>,
        extra: Option<&[u8]>,
    ) -> i32 {
        let h = GzHeader {
            text,
            time,
            os,
            hcrc,
            name: name.map(|n| n.to_vec()),
            comment: comment.map(|c| c.to_vec()),
            extra_len: extra.map_or(0, |e| e.len() as u32),
            extra: extra.map(|e| e.to_vec()),
            ..Default::default()
        };
        let r = self.ds.as_mut().unwrap().set_header(h).code();
        let f = |o: Option<&[u8]>| o.map_or("-".to_string(), hex);
        self.rec(
            format!(
                "dheader {} {} {} 0 {} {} {} {}",
                text,
                time,
                os,
                hcrc,
                f(name),
                f(comment),
                f(extra)
            ),
            format!("dheader {}", r),
        );
        r
    }

    pub fn dcopy(&mut self) {
        let c = self.ds.as_ref().unwrap().clone();
        self.ds = Some(c);
        self.rec("dcopy".into(), "dcopy 0".into());
    }

    pub fn dend(&mut self) -> i32 {
        let r = self.ds.take().unwrap().end().code();
        self.rec("dend".into(), format!("dend {}", r));
        r
    }

    // ---------------------------------------------------------- inflate

    pub fn iinit(&mut self, wbits: i32) -> i32 {
        let r = match Inflate::with_window_bits(wbits) {
            Ok(s) => {
                self.is = Some(s);
                0
            }
            Err(e) => {
                self.is = None;
                e.code()
            }
        };
        self.rec(format!("iinit {}", wbits), format!("iinit {} -", r));
        r
    }

    pub fn i(&mut self, off: usize, ain: usize, aout: usize, flush: i32) -> Call {
        let data = self.h.data(&self.cur);
        let input = &data[off..off + ain];
        let mut buf = vec![0u8; aout];
        let s = self.is.as_mut().unwrap();
        let r = s.inflate_raw(input, &mut buf, flush);
        let prod = &buf[..r.produced];
        let line = format!(
            "i {} {} {} {} {} {} {} {} {}",
            r.status.code(),
            ain - r.consumed,
            aout - r.produced,
            s.total_in(),
            s.total_out(),
            s.adler(),
            s.data_type(),
            msgstr(s.msg()),
            crc32(0, prod)
        );
        self.out.extend_from_slice(prod);
        self.rec(format!("i {} {} {} {}", off, ain, aout, flush), line);
        Call {
            ret: r.status.code(),
            consumed: r.consumed,
            produced: r.produced,
            avail_in: ain - r.consumed,
            avail_out: aout - r.produced,
        }
    }

    pub fn ireset(&mut self) {
        let s = self.is.as_mut().unwrap();
        let r = s.reset().code();
        let line = format!("ireset {} {} {} {}", r, s.total_in(), s.total_out(), s.adler());
        self.rec("ireset".into(), line);
    }

    pub fn iresetkeep(&mut self) {
        let s = self.is.as_mut().unwrap();
        let r = s.reset_keep().code();
        let line = format!("iresetkeep {} {}", r, s.adler());
        self.rec("iresetkeep".into(), line);
    }

    pub fn ireset2(&mut self, w: i32) {
        let s = self.is.as_mut().unwrap();
        let r = s.reset2(w).code();
        let line = format!("ireset2 {} {}", r, s.adler());
        self.rec(format!("ireset2 {}", w), line);
    }

    pub fn idict(&mut self, id: &str, off: usize, len: usize) -> i32 {
        let dict = self.h.data(id)[off..off + len].to_vec();
        let r = self.is.as_mut().unwrap().set_dictionary(&dict).code();
        self.rec(format!("idict {} {} {}", id, off, len), format!("idict {}", r));
        r
    }

    pub fn igetdict(&mut self) {
        let v = self.is.as_ref().unwrap().get_dictionary();
        self.rec("igetdict".into(), format!("igetdict 0 {} {}", v.len(), crc32(0, &v)));
    }

    pub fn isync(&mut self, off: usize, ain: usize) -> Call {
        let data = self.h.data(&self.cur);
        let input = &data[off..off + ain];
        let s = self.is.as_mut().unwrap();
        let r = s.sync(input);
        let line = format!(
            "isync {} {} {} {}",
            r.status.code(),
            ain - r.consumed,
            s.total_in(),
            s.total_out()
        );
        self.rec(format!("isync {} {}", off, ain), line);
        Call {
            ret: r.status.code(),
            consumed: r.consumed,
            produced: 0,
            avail_in: ain - r.consumed,
            avail_out: 0,
        }
    }

    pub fn isyncpoint(&mut self) {
        let v = self.is.as_ref().unwrap().sync_point() as i32;
        self.rec("isyncpoint".into(), format!("isyncpoint {}", v));
    }

    pub fn iprime(&mut self, bits: i32, value: i32) {
        let r = self.is.as_mut().unwrap().prime(bits, value).code();
        self.rec(format!("iprime {} {}", bits, value), format!("iprime {}", r));
    }

    pub fn imark(&mut self) {
        let v = self.is.as_ref().unwrap().mark();
        // C long is 32 bits on Windows
        let v = v as i32;
        self.rec("imark".into(), format!("imark {}", v));
    }

    pub fn icodes(&mut self) {
        let v = self.is.as_ref().unwrap().codes_used();
        self.rec("icodes".into(), format!("icodes {}", v));
    }

    pub fn ivalidate(&mut self, check: i32) {
        let r = self.is.as_mut().unwrap().validate(check != 0).code();
        self.rec(format!("ivalidate {}", check), format!("ivalidate {}", r));
    }

    pub fn igethdr(&mut self, em: u32, nm: u32, cm: u32) {
        let h = GzHeader {
            extra: Some(vec![0u8; em as usize]),
            extra_max: em,
            name: Some(vec![0u8; nm as usize]),
            name_max: nm,
            comment: Some(vec![0u8; cm as usize]),
            comm_max: cm,
            ..Default::default()
        };
        let r = self.is.as_mut().unwrap().get_header(h).code();
        self.rec(format!("igethdr {} {} {}", em, nm, cm), format!("igethdr {}", r));
    }

    pub fn ihdr(&mut self) {
        let h = self.is.as_ref().unwrap().header().cloned().unwrap_or_default();
        let c = |o: &Option<Vec<u8>>| o.as_ref().map_or(0, |v| crc32(0, v));
        let line = format!(
            "ihdr {} {} {} {} {} {} {} {} {} {} {} {} {}",
            h.text,
            h.time,
            h.xflags,
            h.os,
            h.extra_len,
            h.hcrc,
            h.done,
            h.extra.is_some() as i32,
            h.name.is_some() as i32,
            h.comment.is_some() as i32,
            c(&h.extra),
            c(&h.name),
            c(&h.comment)
        );
        self.rec("ihdr".into(), line);
    }

    pub fn iend(&mut self) {
        let r = self.is.take().unwrap().end().code();
        self.rec("iend".into(), format!("iend {}", r));
    }

    // ---------------------------------------------------------- utility

    pub fn adler(&mut self, id: &str, off: usize, len: usize, init: u32) {
        let v = texrs_zlib::adler32(init, &self.h.data(id)[off..off + len]);
        self.rec(format!("adler {} {} {} {}", id, off, len, init), format!("adler {}", v));
    }

    pub fn crc(&mut self, id: &str, off: usize, len: usize, init: u32) {
        let v = texrs_zlib::crc32(init, &self.h.data(id)[off..off + len]);
        self.rec(format!("crc {} {} {} {}", id, off, len, init), format!("crc {}", v));
    }

    pub fn adlercomb(&mut self, a: u32, b: u32, len: i64) {
        let v = texrs_zlib::adler32_combine(a, b, len);
        self.rec(format!("adlercomb {} {} {}", a, b, len), format!("adlercomb {}", v));
    }

    pub fn crccomb(&mut self, a: u32, b: u32, len: i64) {
        let v = texrs_zlib::crc32_combine(a, b, len);
        self.rec(format!("crccomb {} {} {}", a, b, len), format!("crccomb {}", v));
    }

    pub fn cbound(&mut self, n: u64) {
        let v = texrs_zlib::zlib::compressBound(n as texrs_zlib::zlib::uLong);
        self.rec(format!("cbound {}", n), format!("cbound {}", v));
    }

    pub fn compress2(&mut self, id: &str, off: usize, len: usize, dest_len: usize, level: i32) {
        let src = self.h.data(id)[off..off + len].to_vec();
        let mut dest = vec![0u8; dest_len];
        let (r, n) = texrs_zlib::compress2(&mut dest, &src, level);
        let prod = &dest[..n];
        let line = format!("compress2 {} {} {}", r, n, crc32(0, prod));
        self.out.extend_from_slice(prod);
        self.rec(format!("compress2 {} {} {} {} {}", id, off, len, dest_len, level), line);
    }

    pub fn uncompress2(&mut self, id: &str, off: usize, len: usize, dest_len: usize) {
        let src = self.h.data(id)[off..off + len].to_vec();
        let mut dest = vec![0u8; dest_len];
        let (r, n, used) = texrs_zlib::uncompress2(&mut dest, &src);
        let prod = &dest[..n];
        let line = format!("uncompress2 {} {} {} {}", r, n, used, crc32(0, prod));
        self.out.extend_from_slice(prod);
        self.rec(format!("uncompress2 {} {} {} {}", id, off, len, dest_len), line);
    }

    // --------------------------------------------------------------- gz

    fn gz_path(&self) -> PathBuf {
        self.h.dir.join(format!("{}.r.gz", self.name))
    }

    pub fn gzwopen(&mut self, mode: &str) {
        let p = self.gz_path();
        let w = GzWriter::create(&p, mode).ok();
        let ok = w.is_some() as i32;
        self.gzw = w;
        self.rec(format!("gzwopen {}", mode), format!("gzwopen {}", ok));
    }

    pub fn gzropen(&mut self, id: &str, mode: &str) {
        let p = self.h.data_path(id);
        let f = File::open(&p).unwrap();
        let r = GzReader::with_mode(f, mode, p.to_str().unwrap());
        let ok = r.is_some() as i32;
        self.gzr = r;
        self.rec(format!("gzropen {} {}", id, mode), format!("gzropen {}", ok));
    }

    pub fn gzbuffer(&mut self, n: u32) {
        let r = if let Some(w) = self.gzw.as_mut() {
            w.gzbuffer(n)
        } else {
            self.gzr.as_mut().unwrap().gzbuffer(n)
        };
        self.rec(format!("gzbuffer {}", n), format!("gzbuffer {}", r));
    }

    pub fn gzw(&mut self, id: &str, off: usize, len: usize) -> i32 {
        let r = {
            let data = &self.h.data[id];
            self.gzw.as_mut().unwrap().gzwrite(&data[off..off + len])
        };
        self.rec(format!("gzw {} {} {}", id, off, len), format!("gzw {}", r));
        r
    }

    pub fn gzsetparams(&mut self, l: i32, s: i32) {
        let r = self.gzw.as_mut().unwrap().gzsetparams(l, s);
        self.rec(format!("gzsetparams {} {}", l, s), format!("gzsetparams {}", r));
    }

    pub fn gzflush(&mut self, f: i32) {
        let r = self.gzw.as_mut().unwrap().gzflush(f);
        self.rec(format!("gzflush {}", f), format!("gzflush {}", r));
    }

    pub fn gzputc(&mut self, c: i32) {
        let r = self.gzw.as_mut().unwrap().gzputc(c);
        self.rec(format!("gzputc {}", c), format!("gzputc {}", r));
    }

    pub fn gzputs(&mut self, s: &[u8]) {
        let r = self.gzw.as_mut().unwrap().gzputs(s);
        let h = if s.is_empty() { "-".to_string() } else { hex(s) };
        self.rec(format!("gzputs {}", h), format!("gzputs {}", r));
    }

    pub fn gzprintf(&mut self, s: &str) {
        let r = self.gzw.as_mut().unwrap().gzprintf(s);
        let h = if s.is_empty() { "-".to_string() } else { hex(s.as_bytes()) };
        self.rec(format!("gzprintf {}", h), format!("gzprintf {}", r));
    }

    pub fn gzseek(&mut self, off: i64, whence: i32) {
        let r = if let Some(w) = self.gzw.as_mut() {
            w.gzseek(off, whence)
        } else {
            self.gzr.as_mut().unwrap().gzseek(off, whence)
        };
        self.rec(format!("gzseek {} {}", off, whence), format!("gzseek {}", r));
    }

    pub fn gztell(&mut self) {
        let r = if let Some(w) = self.gzw.as_ref() {
            w.gztell()
        } else {
            self.gzr.as_ref().unwrap().gztell()
        };
        self.rec("gztell".into(), format!("gztell {}", r));
    }

    pub fn gzr(&mut self, n: usize) -> i32 {
        let mut buf = vec![0u8; n];
        let r = self.gzr.as_mut().unwrap().gzread(&mut buf);
        let prod = &buf[..r.max(0) as usize];
        let line = format!("gzr {} {}", r, crc32(0, prod));
        self.out.extend_from_slice(prod);
        self.rec(format!("gzr {}", n), line);
        r
    }

    pub fn gzgetc(&mut self) -> i32 {
        let r = self.gzr.as_mut().unwrap().gzgetc();
        self.rec("gzgetc".into(), format!("gzgetc {}", r));
        r
    }

    pub fn gzungetc(&mut self, c: i32) {
        let r = self.gzr.as_mut().unwrap().gzungetc(c);
        self.rec(format!("gzungetc {}", c), format!("gzungetc {}", r));
    }

    pub fn gzgets(&mut self, n: usize) {
        let mut buf = vec![0u8; n];
        let line = match self.gzr.as_mut().unwrap().gzgets(&mut buf) {
            Some(k) => {
                let s = &buf[..k];
                // C reports up to the first NUL
                let s = &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())];
                self.out.extend_from_slice(s);
                format!("gzgets {} {}", s.len(), crc32(0, s))
            }
            None => "gzgets -1 0".to_string(),
        };
        self.rec(format!("gzgets {}", n), line);
    }

    pub fn gzeof(&mut self) {
        let r = self.gzr.as_ref().unwrap().gzeof() as i32;
        self.rec("gzeof".into(), format!("gzeof {}", r));
    }

    pub fn gzdirect(&mut self) {
        let r = self.gzr.as_mut().unwrap().gzdirect() as i32;
        self.rec("gzdirect".into(), format!("gzdirect {}", r));
    }

    pub fn gzerr(&mut self) {
        let (m, e) = if let Some(w) = self.gzw.as_ref() {
            w.gzerror()
        } else {
            self.gzr.as_ref().unwrap().gzerror()
        };
        let m = match m.split_once(": ") {
            Some((_, rest)) => rest.to_string(),
            None => m,
        };
        let m = if m.is_empty() { "-".to_string() } else { m.replace(' ', "_") };
        self.rec("gzerr".into(), format!("gzerr {} {}", e, m));
    }

    pub fn gzclose(&mut self) {
        let r = if let Some(w) = self.gzw.take() {
            w.gzclose()
        } else {
            self.gzr.take().unwrap().gzclose()
        };
        self.rec("gzclose".into(), format!("gzclose {}", r));
    }

    pub fn gzfile(&mut self) {
        let content = fs::read(self.gz_path()).unwrap();
        let line = format!("gzfile {} {}", content.len(), crc32(0, &content));
        self.out.extend_from_slice(&content);
        self.rec("gzfile".into(), line);
    }

    // ------------------------------------------------------------ drivers

    /// zpipe-style deflate: feed the current data in chunks of `in_size()`
    /// bytes; for each chunk call deflate with a fresh output buffer of
    /// `out_size()` bytes until a call leaves output space unused. The flush
    /// for a chunk is `flush_for(is_last_chunk)`; if the last chunk is not
    /// flushed with Z_FINISH, a final Z_FINISH loop follows.
    pub fn deflate_stream(
        &mut self,
        mut in_size: impl FnMut() -> usize,
        mut out_size: impl FnMut() -> usize,
        mut flush_for: impl FnMut(bool) -> i32,
    ) {
        let n = self.cur_len();
        let mut pos = 0usize;
        let mut guard = 0usize;
        loop {
            let chunk = in_size().min(n - pos);
            let last = pos + chunk == n;
            let flush = flush_for(last);
            let mut ain = chunk;
            loop {
                let aout = out_size().max(1);
                let c = self.d(pos, ain, aout, flush);
                pos += c.consumed;
                ain -= c.consumed;
                guard += 1;
                assert!(guard < 3_000_000, "runaway deflate loop");
                if c.ret < 0 && c.ret != -5 {
                    return;
                }
                if c.avail_out != 0 {
                    break;
                }
            }
            // Unconsumed input (possible at level 0 with small output
            // buffers) is simply part of the next chunk.
            if last {
                if flush != 4 {
                    loop {
                        let aout = out_size().max(1);
                        let c = self.d(pos, 0, aout, 4);
                        guard += 1;
                        assert!(guard < 3_000_000, "runaway deflate loop");
                        if c.avail_out != 0 || c.ret < 0 && c.ret != -5 {
                            break;
                        }
                    }
                }
                break;
            }
        }
    }

    /// pdfTeX writezip()-style deflate: a persistent output buffer of
    /// `bufsize` bytes that is drained only when full; input in chunks.
    pub fn deflate_pdftex(&mut self, mut in_size: impl FnMut() -> usize, bufsize: usize) {
        let n = self.cur_len();
        let mut pos = 0usize;
        let mut avail_out = bufsize;
        loop {
            let chunk = in_size().min(n - pos);
            let finish = pos + chunk == n;
            let mut ain = chunk;
            loop {
                if avail_out == 0 {
                    avail_out = bufsize;
                }
                let c = self.d(pos, ain, avail_out, if finish { 4 } else { 0 });
                pos += c.consumed;
                ain -= c.consumed;
                avail_out = c.avail_out;
                if finish && c.ret == 1 {
                    return;
                }
                assert!(c.ret == 0, "deflate failed: {}", c.ret);
                if !finish && ain == 0 {
                    break;
                }
            }
        }
    }

    /// Inflate the current data with explicit chunking; stops at stream end
    /// or error (or when no progress is possible).
    pub fn inflate_stream(
        &mut self,
        mut in_size: impl FnMut() -> usize,
        mut out_size: impl FnMut() -> usize,
        flush: i32,
    ) -> i32 {
        let n = self.cur_len();
        let mut pos = 0usize;
        let mut have = 0usize;
        let mut stall = 0;
        loop {
            if have == 0 {
                have = in_size().min(n - pos);
            }
            let aout = out_size();
            let c = self.i(pos, have, aout, flush);
            pos += c.consumed;
            have -= c.consumed;
            if c.ret == 1 || (c.ret < 0 && c.ret != -5) || c.ret == 2 {
                return c.ret;
            }
            if c.consumed == 0 && c.produced == 0 {
                stall += 1;
                if pos == n || stall > 3 {
                    return c.ret;
                }
            } else {
                stall = 0;
            }
        }
    }

    pub fn end(self) {
        let Case {
            name,
            h,
            cmds,
            trace,
            out,
            calls,
            ..
        } = self;
        let _ = writeln!(h.script, "case {}\n{}end", name, cmds);
        let _ = writeln!(h.trace, "case {}\n{}end", name, trace);
        h.blobs.push((name, out));
        h.calls += calls;
        h.cases += 1;
    }
}

/// Read a whole file.
pub fn read_file(p: &Path) -> Vec<u8> {
    let mut v = Vec::new();
    File::open(p).unwrap().read_to_end(&mut v).unwrap();
    v
}

pub fn write_file(p: &Path, b: &[u8]) {
    File::create(p).unwrap().write_all(b).unwrap();
}

/// Standard named inputs used across tests.
pub fn standard_inputs(big: bool) -> Vec<(&'static str, Vec<u8>)> {
    let mut rng = Rng::new(42);
    let mut v: Vec<(&'static str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("one", b"a".to_vec()),
        ("two", b"ab".to_vec()),
        ("three", b"abc".to_vec()),
        ("tiny", b"hello, hello world\n".to_vec()),
        ("zeros", repetitive(0, 100_000)),
        ("abc", repetitive(1, 70_000)),
        ("period7", repetitive(2, 50_000)),
        ("runs", repetitive(3, 80_000)),
        ("random", random(&mut rng, 70_000)),
        ("text", text(&mut rng, 200_000)),
        ("pdf", pdf_like(&mut rng, 300_000)),
        ("fmt", fmt_like(&mut rng, 250_000)),
        ("far", far_matches(&mut rng, 200_000)),
    ];
    if big {
        let mut m = Vec::new();
        m.extend(text(&mut rng, 400_000));
        m.extend(pdf_like(&mut rng, 500_000));
        m.extend(random(&mut rng, 100_000));
        m.extend(fmt_like(&mut rng, 300_000));
        m.extend(repetitive(0, 50_000));
        m.extend(far_matches(&mut rng, 200_000));
        v.push(("big", m)); // ~1.55 MB
    }
    v
}
