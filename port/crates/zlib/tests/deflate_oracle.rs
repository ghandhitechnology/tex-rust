//! Differential tests of deflate against C zlib 1.3.2 (see tests/common).

mod common;
use common::*;

const NO_FLUSH: i32 = 0;
const FINISH: i32 = 4;

fn add_inputs(h: &mut Harness, big: bool) -> Vec<String> {
    let mut ids = Vec::new();
    for (id, d) in standard_inputs(big) {
        h.add_data(id, d);
        ids.push(id.to_string());
    }
    ids
}

/// All levels 0-9 x all strategies x all standard inputs (incl. 1.5 MB),
/// driven exactly like pdfTeX's writezip(): 16 KiB input chunks, a
/// persistent 32 KiB output buffer, Z_NO_FLUSH then Z_FINISH.
#[test]
fn levels_strategies_pdftex_pattern() {
    let mut h = Harness::new("levels_strategies");
    let ids = add_inputs(&mut h, true);
    for id in &ids {
        for level in 0..=9 {
            for strategy in 0..=4 {
                let mut c = h.case(&format!("ls-{}-{}-{}", id, level, strategy));
                c.use_data(id);
                c.dinit(level, 8, 15, 8, strategy);
                c.deflate_pdftex(|| 16384, 32768);
                c.dend();
                c.end();
            }
        }
    }
    h.finish();
}

/// windowBits 8..15 x raw/zlib/gzip x memLevel 1..9 x several levels.
#[test]
fn window_bits_mem_levels() {
    let mut h = Harness::new("wbits_memlevel");
    let mut rng = Rng::new(7);
    h.add_data("text", text(&mut rng, 60_000));
    h.add_data("far", far_matches(&mut rng, 90_000));
    h.add_data("rnd", random(&mut rng, 20_000));
    for id in ["text", "far", "rnd"] {
        for wbits in 8..=15 {
            for wrap in 0..3 {
                let w = match wrap {
                    0 => -wbits,
                    1 => wbits,
                    _ => wbits + 16,
                };
                let mems: Vec<i32> = if wbits == 15 || wbits == 9 {
                    (1..=9).collect()
                } else {
                    vec![1, 2, 5, 8, 9]
                };
                for &mem in &mems {
                    for &(level, strategy) in &[(0, 0), (1, 0), (3, 0), (4, 1), (6, 0), (9, 0), (6, 3), (5, 2)]
                    {
                        let mut c = h.case(&format!("wm-{}-{}-{}-{}-{}", id, w, mem, level, strategy));
                        c.use_data(id);
                        if c.dinit(level, 8, w, mem, strategy) == 0 {
                            c.deflate_stream(|| 8192, || 4096, |last| if last { FINISH } else { NO_FLUSH });
                            c.dbound(c.data_len(id) as u64);
                            c.dend();
                        }
                        c.end();
                    }
                }
            }
        }
    }
    h.finish();
}

/// Varied input chunk sizes and output buffer sizes (incl. 1-byte output
/// buffers), Z_NO_FLUSH then Z_FINISH, with emphasis on level 0 whose
/// stored-block logic depends on avail_out.
#[test]
fn chunked_input_and_output() {
    let mut h = Harness::new("chunked");
    let mut rng = Rng::new(11);
    h.add_data("text", text(&mut rng, 100_000));
    h.add_data("pdf", pdf_like(&mut rng, 100_000));
    h.add_data("rnd", random(&mut rng, 40_000));
    h.add_data("zeros", repetitive(0, 70_000));
    h.add_data("far", far_matches(&mut rng, 120_000));
    h.add_data("small", text(&mut rng, 3_000));
    h.add_data("rsmall", random(&mut rng, 2_500));
    let ids = ["text", "pdf", "rnd", "zeros", "far", "small", "rsmall"];
    let in_modes = [1usize, 7, 100, 4096, 65536, 0];
    let out_modes = [1usize, 3, 17, 255, 4096, 32768, 100_000, 0];
    for n in 0..700 {
        let level = if n % 3 == 0 { 0 } else { *rng.pick(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]) };
        let strategy = if rng.below(4) == 0 { rng.below(5) as i32 } else { 0 };
        let mut id = *rng.pick(&ids);
        let im = *rng.pick(&in_modes);
        let om = *rng.pick(&out_modes);
        // keep call counts reasonable for tiny buffers
        if (im == 1 || om < 17) && !id.ends_with("small") {
            id = if rng.below(2) == 0 { "small" } else { "rsmall" };
        }
        let mem = *rng.pick(&[8, 8, 8, 1, 2, 9]);
        let wbits = *rng.pick(&[15, 15, 15, 9, 12, -15, 31]);
        let mut c = h.case(&format!("ch-{}", n));
        c.use_data(id);
        c.dinit(level, 8, wbits, mem, strategy);
        let mut r1 = Rng::new(1000 + n);
        let mut r2 = Rng::new(2000 + n);
        c.deflate_stream(
            move || if im == 0 { r1.range(1, 70_000) as usize } else { im },
            move || if om == 0 { r2.range(1, 40_000) as usize } else { om },
            |last| if last { FINISH } else { NO_FLUSH },
        );
        c.dend();
        c.end();
    }
    h.finish();
}

/// Mixed flush modes: every chunk uses a random flush value (including
/// Z_BLOCK and repeated flushes), then Z_FINISH.
#[test]
fn mixed_flush_modes() {
    let mut h = Harness::new("flush_modes");
    let mut rng = Rng::new(23);
    h.add_data("text", text(&mut rng, 80_000));
    h.add_data("pdf", pdf_like(&mut rng, 60_000));
    h.add_data("rnd", random(&mut rng, 30_000));
    h.add_data("fmt", fmt_like(&mut rng, 60_000));
    let ids = ["text", "pdf", "rnd", "fmt"];
    for n in 0..500u64 {
        let level = rng.below(10) as i32;
        let strategy = if rng.below(3) == 0 { rng.below(5) as i32 } else { 0 };
        let id = *rng.pick(&ids);
        let wbits = *rng.pick(&[15, 15, -15, 31, 10, -9]);
        let mem = *rng.pick(&[8, 8, 1, 3, 9]);
        let mut c = h.case(&format!("fl-{}", n));
        c.use_data(id);
        c.dinit(level, 8, wbits, mem, strategy);
        let mut r1 = Rng::new(3000 + n);
        let mut r2 = Rng::new(4000 + n);
        let mut r3 = Rng::new(5000 + n);
        let small_out = r2.below(4) == 0;
        c.deflate_stream(
            move || match r1.below(4) {
                0 => r1.range(0, 50) as usize,
                1 => r1.range(0, 2000) as usize,
                _ => r1.range(0, 30_000) as usize,
            },
            move || if small_out { r2.range(1, 300) as usize } else { r2.range(1, 70_000) as usize },
            move |last| {
                if last && r3.below(2) == 0 {
                    FINISH
                } else {
                    *r3.pick(&[0, 0, 1, 2, 3, 5, 5, 2])
                }
            },
        );
        // repeated useless calls after the end
        let len = c.cur_len();
        c.d(len, 0, 100, FINISH);
        c.d(len, 0, 100, 2);
        c.dend();
        c.end();
    }
    h.finish();
}

/// deflateParams() mid-stream between every kind of level/strategy, with
/// pending input and small output buffers (Z_BUF_ERROR paths), including
/// switches to and from level 0.
#[test]
fn params_midstream() {
    let mut h = Harness::new("params");
    let mut rng = Rng::new(31);
    h.add_data("text", text(&mut rng, 120_000));
    h.add_data("pdf", pdf_like(&mut rng, 90_000));
    h.add_data("far", far_matches(&mut rng, 150_000));
    h.add_data("rnd", random(&mut rng, 30_000));
    let ids = ["text", "pdf", "far", "rnd"];
    for n in 0..500u64 {
        let id = *rng.pick(&ids);
        let level = rng.below(10) as i32;
        let strategy = rng.below(5) as i32;
        let wbits = *rng.pick(&[15, 15, -15, 31, 11]);
        let mem = *rng.pick(&[8, 8, 2, 9]);
        let mut c = h.case(&format!("pa-{}", n));
        c.use_data(id);
        c.dinit(level, 8, wbits, mem, strategy);
        let total = c.cur_len();
        let mut pos = 0usize;
        let mut guard = 0;
        while pos < total && guard < 200 {
            guard += 1;
            let chunk = (rng.range(1, 40_000) as usize).min(total - pos);
            // deflate a chunk with NO_FLUSH, random output sizes
            let mut ain = chunk;
            loop {
                let aout = rng.range(1, 50_000) as usize;
                let r = c.d(pos, ain, aout, NO_FLUSH);
                pos += r.consumed;
                ain -= r.consumed;
                if r.avail_out != 0 || r.ret < 0 {
                    break;
                }
            }
            // change parameters, sometimes with input pending or tiny output
            let nl = if rng.below(4) == 0 { -1 } else { rng.below(10) as i32 };
            let ns = rng.below(5) as i32;
            let pend = if rng.below(3) == 0 { (rng.below(500) as usize).min(total - pos) } else { 0 };
            let aout = *rng.pick(&[1usize, 10, 200, 5000, 100_000]);
            let r = c.dparams(pos, pend, aout, nl, ns);
            pos += r.consumed;
            if r.ret == -5 && rng.below(2) == 0 {
                // retry with more room, as an application would
                let r = c.dparams(pos, 0, 100_000, nl, ns);
                pos += r.consumed;
            }
            if rng.below(8) == 0 {
                let r = c.dparams(pos, 0, 100_000, rng.below(12) as i32 - 1, rng.below(6) as i32);
                pos += r.consumed;
            }
        }
        loop {
            let r = c.d(pos, total - pos, rng.range(1, 70_000) as usize, FINISH);
            pos += r.consumed;
            if r.ret != 0 {
                break;
            }
        }
        c.dend();
        c.end();
    }
    h.finish();
}

/// pdfTeX's stream reuse pattern: one deflate stream, deflateReset between
/// objects, deflateEnd/deflateInit on level changes.
#[test]
fn reset_reuse_pdftex() {
    let mut h = Harness::new("reuse");
    let mut rng = Rng::new(41);
    let mut objs = Vec::new();
    for i in 0..60 {
        let n = rng.range(0, 30_000) as usize;
        let d = match i % 4 {
            0 => pdf_like(&mut rng, n),
            1 => text(&mut rng, n),
            2 => fmt_like(&mut rng, n),
            _ => random(&mut rng, n.min(3000)),
        };
        h.add_data(&format!("o{}", i), d);
        objs.push(format!("o{}", i));
    }
    for (k, levels) in [[6, 6, 6, 9], [9, 1, 9, 9], [1, 1, 2, 3], [6, 0, 6, 9]].iter().enumerate() {
        let mut c = h.case(&format!("reuse-{}", k));
        let mut cur_level = levels[0];
        c.dinit(cur_level, 8, 15, 8, 0);
        for (i, o) in objs.iter().enumerate() {
            let level = levels[(i / 15) % 4];
            if level != cur_level {
                c.dend();
                c.dinit(level, 8, 15, 8, 0);
                cur_level = level;
            } else if i > 0 {
                c.dreset();
            }
            c.use_data(o);
            c.deflate_pdftex(|| 16384, 32768);
        }
        c.dend();
        c.end();
    }
    // deflateResetKeep and deflateCopy mid-stream
    for n in 0..20u64 {
        let mut c = h.case(&format!("keep-{}", n));
        c.use_data(&objs[n as usize]);
        c.dinit(rng.below(10) as i32, 8, *rng.pick(&[15, -15, 31]), 8, 0);
        let len = c.cur_len();
        let half = len / 2;
        c.d(0, half, 100_000, NO_FLUSH);
        c.dcopy();
        c.d(half, len - half, 100_000, FINISH);
        c.dresetkeep();
        c.d(0, len, 100_000, FINISH);
        c.dend();
        c.end();
    }
    h.finish();
}

/// Dictionaries, deflatePrime, deflateTune, deflateBound, deflatePending,
/// deflateUsed, gzip headers, and error returns.
#[test]
fn misc_api() {
    let mut h = Harness::new("misc");
    let mut rng = Rng::new(53);
    h.add_data("text", text(&mut rng, 50_000));
    h.add_data("dict", text(&mut rng, 45_000));
    h.add_data("rnd", random(&mut rng, 5_000));
    h.add_data("pdf", pdf_like(&mut rng, 20_000));

    // dictionaries
    for (n, &(wbits, dlen, level)) in [
        (15, 0usize, 6),
        (15, 10, 6),
        (15, 1000, 1),
        (15, 32768, 9),
        (15, 40000, 6),
        (-15, 100, 6),
        (-15, 40000, 4),
        (-9, 600, 6),
        (9, 300, 2),
        (31, 100, 6),
        (15, 5000, 0),
    ]
    .iter()
    .enumerate()
    {
        let mut c = h.case(&format!("dict-{}", n));
        c.use_data("text");
        c.dinit(level, 8, wbits, 8, 0);
        c.ddict("dict", 0, dlen);
        c.dgetdict();
        c.dbound(50_000);
        c.deflate_stream(|| 7000, || 3000, |last| if last { FINISH } else { NO_FLUSH });
        c.dgetdict();
        // dictionary after data is an error for zlib streams
        c.ddict("dict", 0, 10);
        c.dend();
        c.end();
    }

    // deflatePrime. zlib.h only supports priming before the first deflate()
    // call; C also behaves deterministically when no output is pending
    // (pending == 0). Priming while output is pending makes C emit
    // uninitialized pending_buf bytes, so that is not tested.
    for n in 0..30u64 {
        let mut c = h.case(&format!("prime-{}", n));
        c.use_data("pdf");
        c.dinit(rng.below(10) as i32, 8, *rng.pick(&[-15, 15, 31]), 8, 0);
        for _ in 0..rng.range(1, 4) {
            c.dprime(rng.range(0, 18) as i32 - 1, rng.next() as i32);
        }
        c.dpending();
        c.d(0, 10_000, 100_000, NO_FLUSH);
        c.dpending();
        c.dprime(rng.range(0, 16) as i32, rng.next() as i32);
        c.dpending();
        c.d(10_000, 10_000, 100_000, 2);
        c.dprime(rng.range(0, 16) as i32, rng.next() as i32);
        c.dused();
        c.d(20_000, 0, 100_000, FINISH);
        c.dused();
        c.dend();
        c.end();
    }

    // deflateTune with odd values
    for n in 0..60u64 {
        let mut c = h.case(&format!("tune-{}", n));
        c.use_data(*rng.pick(&["text", "pdf", "rnd"]));
        c.dinit(rng.range(1, 9) as i32, 8, 15, 8, *rng.pick(&[0, 0, 1]));
        let g = rng.range(0, 300) as i32;
        let l = rng.range(0, 300) as i32;
        let ni = *rng.pick(&[0, 1, 3, 8, 50, 258, 300, -1]);
        let ch = *rng.pick(&[0, 1, 2, 4, 100, 5000]);
        c.dtune(g, l, ni, ch);
        c.deflate_stream(|| 9000, || 20_000, |last| if last { FINISH } else { NO_FLUSH });
        c.dend();
        c.end();
    }

    // deflateBound for many lengths and parameter sets
    {
        let mut c = h.case("bound");
        c.use_data("text");
        for &(w, m, l) in &[(15, 8, 6), (15, 9, 6), (14, 8, 6), (15, 1, 0), (9, 9, 9), (-15, 8, 6), (31, 8, 6), (10, 3, 0), (12, 2, 1)] {
            c.dinit(l, 8, w, m, 0);
            for &n in &[0u64, 1, 2, 100, 1000, 16383, 16384, 65535, 1 << 20, 3_000_000_000, 4_000_000_000] {
                c.dbound(n);
            }
            c.dend();
        }
        for &n in &[0u64, 1, 100, 1 << 20, 4_000_000_000] {
            c.dbound0(n);
        }
        c.dinit(6, 8, 15, 8, 0);
        c.ddict("dict", 0, 100);
        c.dbound(1000);
        c.dend();
        c.end();
    }

    // gzip headers through the header state machine with tiny buffers
    for n in 0..40u64 {
        let mut c = h.case(&format!("hdr-{}", n));
        c.use_data("pdf");
        c.dinit(rng.below(10) as i32, 8, 31, *rng.pick(&[1, 8]), 0);
        let name: Vec<u8> = (0..rng.below(600)).map(|_| rng.range(1, 255) as u8).collect();
        let comment: Vec<u8> = (0..rng.below(300)).map(|_| rng.range(1, 255) as u8).collect();
        let extra: Vec<u8> = (0..rng.below(1500)).map(|_| rng.next() as u8).collect();
        let has = rng.below(8);
        c.dheader(
            (has & 1) as i32,
            rng.next() as u32,
            rng.below(256) as i32,
            ((has >> 1) & 1) as i32,
            if has & 2 != 0 { Some(&name) } else { None },
            if has & 4 != 0 { Some(&comment) } else { None },
            if n % 3 != 0 { Some(&extra) } else { None },
        );
        c.dbound(20_000);
        let small = rng.range(1, 64) as usize;
        c.deflate_stream(|| 20_000, || small, |last| if last { FINISH } else { NO_FLUSH });
        c.dend();
        c.end();
    }
    // header on a non-gzip stream is an error
    {
        let mut c = h.case("hdr-err");
        c.dinit(6, 8, 15, 8, 0);
        c.dheader(0, 0, 3, 0, None, None, None);
        c.dend();
        c.end();
    }

    // error returns
    {
        let mut c = h.case("errors");
        c.use_data("text");
        for &(l, m, w, ml, s) in &[
            (10, 8, 15, 8, 0),
            (-2, 8, 15, 8, 0),
            (6, 7, 15, 8, 0),
            (6, 8, 7, 8, 0),
            (6, 8, 16, 8, 0),
            (6, 8, 24, 8, 0),
            (6, 8, -8, 8, 0),
            (6, 8, -16, 8, 0),
            (6, 8, 32, 8, 0),
            (6, 8, 15, 0, 0),
            (6, 8, 15, 10, 0),
            (6, 8, 15, 8, 5),
            (6, 8, 15, 8, -1),
            (-1, 8, 8, 8, 0),
        ] {
            if c.dinit(l, m, w, ml, s) == 0 {
                c.dend();
            }
        }
        c.dinit(6, 8, 15, 8, 0);
        c.d(0, 100, 0, NO_FLUSH); // avail_out == 0
        c.d(0, 100, 10, 6); // invalid flush
        c.d(0, 100, 10, -1);
        c.d(0, 100, 1000, NO_FLUSH);
        c.d(100, 0, 1000, NO_FLUSH); // nothing to do: Z_BUF_ERROR
        c.d(100, 0, 1000, 2);
        c.d(100, 0, 1000, 2); // duplicate flush
        c.d(100, 0, 1000, 1);
        c.d(100, 100, 1000, FINISH);
        c.d(200, 0, 1000, FINISH);
        c.d(200, 0, 1000, FINISH);
        c.d(200, 50, 1000, FINISH); // more input after finish
        c.d(200, 0, 1000, NO_FLUSH); // non-finish after finish
        c.dend();
        // deflateEnd on a busy stream
        c.dinit(6, 8, 15, 8, 0);
        c.d(0, 1000, 10, NO_FLUSH);
        c.dend();
        c.dinit(6, 8, 15, 8, 0);
        c.dend();
        c.end();
    }
    h.finish();
}

/// adler32, crc32, combine, compress2/uncompress2/compressBound.
#[test]
fn checksums_and_utility() {
    let mut h = Harness::new("utility");
    let mut rng = Rng::new(61);
    h.add_data("rnd", random(&mut rng, 100_000));
    h.add_data("text", text(&mut rng, 100_000));
    h.add_data("ff", vec![0xffu8; 70_000]);
    let mut c = h.case("checksums");
    for &len in &[0usize, 1, 2, 15, 16, 17, 100, 5551, 5552, 5553, 11104, 11105, 65536, 99_999] {
        for &init in &[0u32, 1, 0xffff_ffff, 65520, 65521, 0xfff0_fff0, 0x1234_5678] {
            c.adler("rnd", 1, len, init);
            c.adler("ff", 0, len.min(70_000), init);
            c.crc("rnd", 1, len, init);
            c.crc("text", 1, len, init);
        }
    }
    for _ in 0..200 {
        let a = rng.next() as u32;
        let b = rng.next() as u32;
        let rl = rng.below(1 << 40) as i64;
        let l = *rng.pick(&[0i64, 1, 2, 100, 65521, 65522, 1 << 33, rl]);
        c.adlercomb(a % 0x10000 | (b & 0xffff0000), b, l);
        c.crccomb(a, b, l);
    }
    c.adlercomb(1, 1, -1);
    c.crccomb(1, 1, -1);
    for &n in &[0u64, 1, 100, 65535, 1 << 20, 1 << 31, 4_000_000_000] {
        c.cbound(n);
    }
    c.end();

    let mut c = h.case("compress");
    for &level in &[-1, 0, 1, 6, 9, 10] {
        for &len in &[0usize, 1, 100, 50_000, 100_000] {
            for &dl in &[0usize, 10, 120_000] {
                c.compress2("text", 0, len, dl, level);
                c.compress2("rnd", 0, len, dl, level);
            }
        }
    }
    c.end();
    h.finish();
}
