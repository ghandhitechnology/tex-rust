//! Differential tests of the gzip file layer (gzwrite.c / gzread.c)
//! against C zlib 1.3.2 (see tests/common).

mod common;
use common::*;
use texrs_zlib::{Deflate, Flush};

/// pdfTeX format dumping: gzdopen(fd, "wb"); gzsetparams(f, 1,
/// Z_DEFAULT_STRATEGY); many gzwrite() calls of 4/8 bytes and large arrays;
/// gzclose(). The resulting file must be byte-identical.
#[test]
fn pdftex_fmt_dump_pattern() {
    let mut h = Harness::new("gz_fmt");
    let mut rng = Rng::new(71);
    h.add_data("fmt", fmt_like(&mut rng, 2_500_000));
    h.add_data("text", text(&mut rng, 600_000));
    for n in 0..12u64 {
        let id = if n % 3 == 2 { "text" } else { "fmt" };
        let mut c = h.case(&format!("dump-{}", n));
        c.gzwopen("wb");
        c.gzsetparams(1, 0);
        let len = c.data_len(id);
        let mut pos = 0usize;
        let mut r = Rng::new(n);
        while pos < len {
            let k = match r.below(10) {
                0..=5 => 4,
                6 => 8,
                7 => r.range(1, 100) as usize,
                8 => r.range(1000, 9000) as usize,
                _ => r.range(8192, 400_000) as usize,
            }
            .min(len - pos);
            c.gzw(id, pos, k);
            pos += k;
        }
        c.gztell();
        c.gzclose();
        c.gzfile();
        c.end();
    }
    h.finish();
}

/// All mode letters, buffer sizes, putc/puts/printf, flushes, parameter
/// changes and forward seeks.
#[test]
fn write_modes_and_operations() {
    let mut h = Harness::new("gz_write");
    let mut rng = Rng::new(73);
    h.add_data("text", text(&mut rng, 300_000));
    h.add_data("pdf", pdf_like(&mut rng, 200_000));
    h.add_data("rnd", random(&mut rng, 60_000));
    let ids = ["text", "pdf", "rnd"];
    let modes = [
        "wb", "w", "wb0", "wb1", "wb6", "wb9", "wbf", "wbh", "wbR", "wbF", "wb9F", "wbT", "w3h", "wb2R",
    ];
    for n in 0..160u64 {
        let mode = *rng.pick(&modes);
        let id = *rng.pick(&ids);
        let mut c = h.case(&format!("gw-{}", n));
        c.gzwopen(mode);
        if rng.below(3) == 0 {
            c.gzbuffer(*rng.pick(&[1, 8, 100, 4096, 65536]));
        }
        let len = c.data_len(id);
        let mut pos = 0usize;
        let mut ops = 0;
        // C's gztell() and gzseek(SEEK_CUR) read the uninitialized
        // state->past in write mode while a seek is pending (UB), so those
        // two are only issued when no skip is pending.
        let mut skip_pending = false;
        // A seek before the buffers are allocated makes C's gz_zero() skip
        // its memset and write uninitialized memory (UB), so seeks are only
        // issued once something has initialized the stream.
        let mut initialized = false;
        while pos < len && ops < 400 {
            ops += 1;
            match rng.below(14) {
                0..=6 => {
                    let k = (match rng.below(3) {
                        0 => rng.range(0, 20),
                        1 => rng.range(0, 9000),
                        _ => rng.range(0, 70_000),
                    } as usize)
                        .min(len - pos);
                    c.gzw(id, pos, k);
                    pos += k;
                    if k > 0 {
                        skip_pending = false;
                        initialized = true;
                    }
                }
                7 => {
                    c.gzputc(rng.below(256) as i32);
                    skip_pending = false;
                    initialized = true;
                }
                8 => {
                    let s: Vec<u8> = (0..rng.below(50)).map(|_| rng.range(32, 126) as u8).collect();
                    c.gzputs(&s);
                    if !s.is_empty() {
                        skip_pending = false;
                        initialized = true;
                    }
                }
                9 => {
                    let s: String = (0..rng.range(0, 120)).map(|_| rng.range(32, 126) as u8 as char).collect();
                    c.gzprintf(&s);
                    skip_pending = false;
                    initialized = true;
                }
                10 => {
                    let f = *rng.pick(&[0, 1, 2, 3, 5]);
                    c.gzflush(f);
                    if f <= 4 {
                        // Z_BLOCK is rejected before the pending seek runs
                        skip_pending = false;
                        initialized = true;
                    }
                }
                11 => c.gzsetparams(rng.range(0, 10) as i32 - 1, rng.below(5) as i32),
                12 if initialized => {
                    let off = rng.below(5000) as i64;
                    if rng.below(2) == 0 && !skip_pending {
                        c.gzseek(off, 1);
                    } else {
                        c.gzseek(off, 0);
                    }
                    skip_pending = true;
                }
                _ => {
                    if !skip_pending {
                        c.gztell()
                    }
                }
            }
        }
        if rng.below(4) == 0 {
            c.gzflush(4); // finish one member, then write another
            let k = (rng.below(5000) as usize).min(len);
            c.gzw(id, 0, k);
        }
        c.gzerr();
        c.gzclose();
        c.gzfile();
        c.end();
    }
    h.finish();
}

fn gzip_member(data: &[u8], level: i32) -> Vec<u8> {
    let mut d = Deflate::with_params(level, 8, 31, 8, 0).unwrap();
    let mut out = vec![0u8; data.len() + data.len() / 10 + 100];
    let r = d.deflate(data, &mut out, Flush::Finish);
    out.truncate(r.produced);
    out
}

/// gzread / gzgetc / gzungetc / gzgets / gzeof / gzdirect / gzseek over
/// valid, multi-member, garbage-trailed, transparent, truncated and corrupt
/// files.
#[test]
fn read_operations() {
    let mut h = Harness::new("gz_read");
    let mut rng = Rng::new(79);
    let t = text(&mut rng, 200_000);
    let p = pdf_like(&mut rng, 100_000);
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    files.push(("one".into(), gzip_member(&t, 6)));
    let mut multi = gzip_member(&t[..50_000], 1);
    multi.extend(gzip_member(&p, 9));
    multi.extend(gzip_member(&[], 6));
    multi.extend(gzip_member(&t[50_000..60_000], 0));
    files.push(("multi".into(), multi.clone()));
    let mut garb = gzip_member(&p[..30_000], 6);
    garb.extend_from_slice(b"trailing garbage that is not gzip");
    files.push(("garbage".into(), garb));
    let mut garb2 = gzip_member(&p[..30_000], 6);
    garb2.extend_from_slice(&[0x1f, 0x8b, 8, 0, 1, 2]);
    files.push(("garbage2".into(), garb2));
    files.push(("plain".into(), t[..70_000].to_vec()));
    files.push(("empty".into(), Vec::new()));
    files.push(("short".into(), b"ab".to_vec()));
    let one = gzip_member(&t, 6);
    files.push(("trunc".into(), one[..one.len() / 2].to_vec()));
    files.push(("trunctrailer".into(), one[..one.len() - 3].to_vec()));
    let mut bad = one.clone();
    let k = bad.len() / 3;
    bad[k] ^= 0x40;
    files.push(("corrupt".into(), bad));
    let mut badcrc = one.clone();
    let n = badcrc.len();
    badcrc[n - 7] ^= 1;
    files.push(("badcrc".into(), badcrc));
    let lines: Vec<u8> = t.iter().map(|&b| if b == b'.' { b'\n' } else { b }).collect();
    files.push(("lines".into(), gzip_member(&lines[..100_000], 6)));
    for (id, d) in &files {
        h.add_data(id, d.clone());
    }
    for n in 0..240u64 {
        let (id, _) = rng.pick(&files).clone();
        let mode = *rng.pick(&["rb", "rb", "r", "rbG"]);
        let mut c = h.case(&format!("gr-{}", n));
        c.gzropen(&id, mode);
        if rng.below(4) == 0 {
            c.gzbuffer(*rng.pick(&[8, 100, 4096, 70_000]));
        }
        if rng.below(3) == 0 {
            c.gzdirect();
        }
        for _ in 0..rng.range(1, 60) {
            match rng.below(12) {
                0..=4 => {
                    let k = *rng.pick(&[1usize, 7, 100, 8191, 16384, 16385, 40_000, 300_000]);
                    if c.gzr(k) <= 0 && rng.below(2) == 0 {
                        c.gzerr();
                    }
                }
                5 => {
                    c.gzgetc();
                }
                6 => c.gzungetc(rng.below(256) as i32),
                7 => c.gzgets(*rng.pick(&[1usize, 2, 10, 200, 5000])),
                8 => c.gzeof(),
                9 => {
                    let w = rng.below(2) as i32;
                    let off = rng.below(150_000) as i64 - if w == 1 { 50_000 } else { 0 };
                    c.gzseek(off, w);
                }
                10 => c.gztell(),
                _ => c.gzerr(),
            }
        }
        c.gzeof();
        c.gzerr();
        c.gzclose();
        c.end();
    }
    h.finish();
}
