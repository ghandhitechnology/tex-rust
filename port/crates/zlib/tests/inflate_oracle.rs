//! Differential tests of inflate against C zlib 1.3.2 (see tests/common),
//! plus round trips.

mod common;
use common::*;
use texrs_zlib::{Deflate, Flush, Inflate, Status};

const NO_FLUSH: i32 = 0;
const FINISH: i32 = 4;

/// Compress `data` with texrs-zlib (whose output is verified identical to C
/// by deflate_oracle) using random chunking and optional flushes.
fn make_stream(data: &[u8], level: i32, wbits: i32, mem: i32, strategy: i32, rng: &mut Rng, flushes: bool) -> Vec<u8> {
    let mut d = Deflate::with_params(level, 8, wbits, mem, strategy).unwrap();
    let mut out = Vec::new();
    let mut buf = vec![0u8; 65536];
    let mut pos = 0;
    while pos < data.len() {
        let n = (rng.range(1, 50_000) as usize).min(data.len() - pos);
        let f = if flushes {
            *rng.pick(&[Flush::NoFlush, Flush::NoFlush, Flush::SyncFlush, Flush::FullFlush, Flush::PartialFlush, Flush::Block])
        } else {
            Flush::NoFlush
        };
        let mut chunk = &data[pos..pos + n];
        loop {
            let r = d.deflate(chunk, &mut buf, f);
            out.extend_from_slice(&buf[..r.produced]);
            chunk = &chunk[r.consumed..];
            if r.produced < buf.len() {
                break;
            }
        }
        pos += n;
    }
    loop {
        let r = d.deflate(&[], &mut buf, Flush::Finish);
        out.extend_from_slice(&buf[..r.produced]);
        if r.status == Status::StreamEnd {
            break;
        }
    }
    out
}

fn inflate_all(stream: &[u8], wbits: i32, outsz: usize) -> (Status, Vec<u8>) {
    let mut inf = Inflate::with_window_bits(wbits).unwrap();
    let mut out = Vec::new();
    let mut buf = vec![0u8; outsz];
    let mut pos = 0;
    loop {
        let r = inf.inflate(&stream[pos..], &mut buf, Flush::NoFlush);
        out.extend_from_slice(&buf[..r.produced]);
        pos += r.consumed;
        if r.status != Status::Ok {
            return (r.status, out);
        }
    }
}

/// Round trip of every level/strategy/window/memLevel over the standard
/// inputs (Rust only; the compressed bytes themselves are checked against C
/// in deflate_oracle).
#[test]
fn round_trip_matrix() {
    let mut rng = Rng::new(5);
    let mut n = 0;
    for (_, data) in standard_inputs(true) {
        for level in 0..=9 {
            for strategy in 0..=4 {
                for &(wbits, mem) in &[(15, 8), (-15, 8), (31, 8), (9, 1), (12, 9), (10, 4)] {
                    let flushes = rng.below(3) == 0;
                    let s = make_stream(&data, level, wbits, mem, strategy, &mut rng, flushes);
                    let iw = if wbits > 15 { 47 } else { wbits };
                    let (st, out) = inflate_all(&s, iw, *rng.pick(&[1000usize, 65536, 300_000]));
                    assert_eq!(st, Status::StreamEnd, "level {} strat {} wbits {}", level, strategy, wbits);
                    assert!(out == data, "round trip mismatch level {} strat {} wbits {}", level, strategy, wbits);
                    n += 1;
                }
            }
        }
    }
    eprintln!("[round_trip_matrix] {} round trips OK", n);
}

/// Chunked inflate of valid streams of every kind with random avail_in /
/// avail_out and flush values, compared call by call with C.
#[test]
fn chunked_inflate() {
    let mut h = Harness::new("inflate_chunked");
    let mut rng = Rng::new(17);
    let srcs = vec![
        text(&mut rng, 60_000),
        pdf_like(&mut rng, 50_000),
        random(&mut rng, 20_000),
        far_matches(&mut rng, 80_000),
        fmt_like(&mut rng, 40_000),
        repetitive(0, 30_000),
        b"x".to_vec(),
        Vec::new(),
    ];
    let mut streams = Vec::new();
    for k in 0..120 {
        let src = &srcs[k % srcs.len()];
        let level = rng.below(10) as i32;
        let strategy = if rng.below(3) == 0 { rng.below(5) as i32 } else { 0 };
        let wb = *rng.pick(&[15, 15, -15, 31, 9, 12, -10]);
        let mem = *rng.pick(&[8, 1, 9]);
        let flushes = rng.below(2) == 0;
        let s = make_stream(src, level, wb, mem, strategy, &mut rng, flushes);
        let id = format!("s{}", k);
        h.add_data(&id, s);
        streams.push((id, wb));
    }
    for (n, (id, wb)) in streams.iter().enumerate() {
        for v in 0..3 {
            let iw = match (v, *wb) {
                (0, w) => w,
                (1, w) if w > 15 => 47,
                (1, w) if w > 0 => 0,
                (1, w) => w,
                (_, w) if w > 15 => 31,
                (_, w) if w > 0 => 47,
                (_, w) => w.min(-9),
            };
            let mut c = h.case(&format!("ic-{}-{}", n, v));
            c.use_data(id);
            c.iinit(iw);
            let mut r1 = Rng::new(n as u64 * 7 + v);
            let mut r2 = Rng::new(n as u64 * 11 + v);
            let mode = r1.below(4);
            let flush = *r2.pick(&[NO_FLUSH, NO_FLUSH, 2, FINISH, 5, 6]);
            c.inflate_stream(
                move || match mode {
                    0 => r1.range(1, 3) as usize,
                    1 => r1.range(1, 100) as usize,
                    _ => r1.range(1, 100_000) as usize,
                },
                move || match r2.below(3) {
                    0 => r2.range(1, 10) as usize,
                    1 => r2.range(1, 1000) as usize,
                    _ => r2.range(1, 200_000) as usize,
                },
                flush,
            );
            c.imark();
            c.icodes();
            c.iend();
            c.end();
        }
    }
    h.finish();
}

fn corrupt(rng: &mut Rng, s: &[u8]) -> Vec<u8> {
    let mut v = s.to_vec();
    if v.is_empty() {
        return vec![rng.next() as u8];
    }
    match rng.below(6) {
        0 | 1 => {
            // flip 1-3 bits
            for _ in 0..rng.range(1, 3) {
                let i = rng.below(v.len() as u64) as usize;
                v[i] ^= 1 << rng.below(8);
            }
        }
        2 => {
            // truncate
            let n = rng.below(v.len() as u64) as usize;
            v.truncate(n);
        }
        3 => {
            // overwrite a byte near the start (headers / tables)
            let i = rng.below((v.len() as u64).min(40)) as usize;
            v[i] = rng.next() as u8;
        }
        4 => {
            // corrupt the trailer
            let n = v.len();
            let i = n - 1 - rng.below((n as u64).min(8)) as usize;
            v[i] ^= rng.range(1, 255) as u8;
        }
        _ => {
            // append garbage
            for _ in 0..rng.range(1, 20) {
                v.push(rng.next() as u8);
            }
        }
    }
    v
}

/// Inflate of corrupt data must give the same error codes, messages and
/// partial output as C.
#[test]
fn corrupt_streams() {
    let mut h = Harness::new("inflate_corrupt");
    let mut rng = Rng::new(29);
    let srcs = vec![
        text(&mut rng, 20_000),
        pdf_like(&mut rng, 30_000),
        random(&mut rng, 3_000),
        far_matches(&mut rng, 50_000),
        b"hello".to_vec(),
    ];
    let mut cases = Vec::new();
    for k in 0..700 {
        let src = &srcs[k % srcs.len()];
        let level = rng.below(10) as i32;
        let strategy = if rng.below(3) == 0 { rng.below(5) as i32 } else { 0 };
        let wb = *rng.pick(&[15, -15, 31, 10]);
        let fl = rng.below(4) == 0;
        let s = make_stream(src, level, wb, 8, strategy, &mut rng, fl);
        let bad = corrupt(&mut rng, &s);
        let id = format!("b{}", k);
        h.add_data(&id, bad);
        let iw = if wb > 15 && rng.below(2) == 0 { 47 } else { wb };
        cases.push((id, iw));
    }
    // hand-made streams for specific errors
    let crafted: Vec<(&str, Vec<u8>, i32)> = vec![
        ("hdrcheck", vec![0x78, 0x00, 0x01, 0x00], 15),
        ("method", vec![0x77, 0x85, 0, 0], 15),
        ("wsize", vec![0x88, 0x98, 0, 0], 15),
        ("wsize2", vec![0x78, 0x9c, 0x03, 0x00], 9),
        ("gzflags", vec![0x1f, 0x8b, 8, 0xe0, 0, 0, 0, 0, 0, 3], 31),
        ("gzmethod", vec![0x1f, 0x8b, 7, 0, 0, 0, 0, 0, 0, 3], 31),
        ("gzhcrc", vec![0x1f, 0x8b, 8, 2, 0, 0, 0, 0, 0, 3, 0x12, 0x34, 3, 0], 31),
        ("btype3", vec![0x07, 0x00], -15),
        ("stored", vec![0x01, 0x05, 0x00, 0x00, 0x00], -15),
        ("toomany", vec![0xfd, 0xff, 0xff], -15),
        ("lens", vec![0x05, 0xe0, 0xff, 0xff, 0xff, 0xff], -15),
        ("fixed287", vec![0x03, 0xe3, 0x07], -15),
        ("fixeddist30", vec![0x63, 0x00, 0x3e, 0x00], -15),
        ("toofar", vec![0x63, 0x00, 0x42, 0x00], -15),
        ("empty", vec![], 15),
        ("zlibempty", vec![0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x02], 15),
        ("autozlib", vec![0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01], 47),
    ];
    for (name, bytes, iw) in crafted {
        let id = format!("x{}", name);
        h.add_data(&id, bytes);
        cases.push((id, iw));
    }
    for (n, (id, iw)) in cases.iter().enumerate() {
        let mut c = h.case(&format!("bad-{}", n));
        c.use_data(id);
        c.iinit(*iw);
        let mut r1 = Rng::new(n as u64 + 99);
        let big = r1.below(2) == 0;
        let mut r2 = Rng::new(n as u64 + 199);
        let flush = if r2.below(4) == 0 { FINISH } else { NO_FLUSH };
        c.inflate_stream(
            move || if big { 1 << 20 } else { r1.range(1, 5000) as usize },
            move || if r2.below(3) == 0 { r2.range(1, 300) as usize } else { 1 << 18 },
            flush,
        );
        // calls after an error keep returning the error
        let len = c.cur_len();
        c.i(len, 0, 100, NO_FLUSH);
        c.imark();
        c.iend();
        c.end();
    }
    // uncompress2 on good and bad data with various destination sizes
    {
        let mut c = h.case("uncompress");
        for k in 0..60 {
            let id = format!("b{}", k);
            for &dl in &[0usize, 1, 100, 1 << 17] {
                let len = c.data_len(&id);
                c.uncompress2(&id, 0, len, dl);
            }
        }
        c.end();
    }
    h.finish();
}

/// Dictionaries, inflateSync, gzip headers, prime, validate, reset2,
/// concatenated gzip members.
#[test]
fn misc_inflate_api() {
    let mut h = Harness::new("inflate_misc");
    let mut rng = Rng::new(37);
    let dict = text(&mut rng, 40_000);
    let src = text(&mut rng, 50_000);
    h.add_data("dict", dict.clone());

    // zlib stream with a preset dictionary
    for (n, &dlen) in [10usize, 1000, 32768, 40000].iter().enumerate() {
        let mut d = Deflate::with_params(6, 8, 15, 8, 0).unwrap();
        d.set_dictionary(&dict[..dlen]);
        let mut out = vec![0u8; 100_000];
        let r = d.deflate(&src, &mut out, Flush::Finish);
        assert_eq!(r.status, Status::StreamEnd);
        h.add_data(&format!("zd{}", n), out[..r.produced].to_vec());
        let mut c = h.case(&format!("dict-{}", n));
        c.use_data(&format!("zd{}", n));
        c.iinit(15);
        let c1 = c.i(0, 1000, 5000, NO_FLUSH); // Z_NEED_DICT
        assert_eq!(c1.ret, 2);
        c.idict("dict", 0, dlen.saturating_sub(1)); // wrong
        c.idict("dict", 0, dlen);
        c.igetdict();
        let len = c.cur_len();
        c.i(c1.consumed, len - c1.consumed, 200_000, NO_FLUSH);
        c.igetdict();
        c.iend();
        c.end();
    }
    // raw stream with dictionary
    {
        let mut d = Deflate::with_params(9, 8, -15, 8, 0).unwrap();
        d.set_dictionary(&dict[..5000]);
        let mut out = vec![0u8; 100_000];
        let r = d.deflate(&src, &mut out, Flush::Finish);
        h.add_data("rd", out[..r.produced].to_vec());
        let mut c = h.case("rawdict");
        c.use_data("rd");
        c.iinit(-15);
        c.idict("dict", 0, 5000);
        let len = c.cur_len();
        c.i(0, len, 200_000, NO_FLUSH);
        c.iend();
        c.end();
    }

    // inflateSync over full-flush points with corrupted beginnings
    for n in 0..20u64 {
        let mut d = Deflate::with_params(rng.range(1, 9) as i32, 8, *rng.pick(&[15, -15]), 8, 0).unwrap();
        let mut out = Vec::new();
        let mut buf = vec![0u8; 200_000];
        for part in src.chunks(7000) {
            let r = d.deflate(part, &mut buf, Flush::FullFlush);
            out.extend_from_slice(&buf[..r.produced]);
        }
        let r = d.deflate(&[], &mut buf, Flush::Finish);
        out.extend_from_slice(&buf[..r.produced]);
        let cut = rng.range(2, 3000) as usize;
        for b in out.iter_mut().take(cut).skip(2) {
            *b ^= 0x55;
        }
        let id = format!("sy{}", n);
        h.add_data(&id, out);
        let mut c = h.case(&format!("sync-{}", n));
        c.use_data(&id);
        c.iinit(if n % 2 == 0 { 15 } else { -15 });
        let len = c.cur_len();
        let mut pos = 0usize;
        for _ in 0..6 {
            let step = rng.range(1, 4000) as usize;
            let r = c.i(pos, step.min(len - pos), 100_000, NO_FLUSH);
            pos += r.consumed;
            if r.ret == -3 || r.ret == 0 && rng.below(2) == 0 {
                let s = c.isync(pos, (len - pos).min(rng.range(0, 9000) as usize));
                pos += s.consumed;
                c.isyncpoint();
            }
            if pos >= len {
                break;
            }
        }
        c.iend();
        c.end();
    }

    // gzip headers via inflateGetHeader, tiny input chunks
    for n in 0..30u64 {
        let mut d = Deflate::with_params(6, 8, 31, 8, 0).unwrap();
        let name: Vec<u8> = (0..rng.below(100)).map(|_| rng.range(1, 255) as u8).collect();
        let comment: Vec<u8> = (0..rng.below(100)).map(|_| rng.range(1, 255) as u8).collect();
        let extra: Vec<u8> = (0..rng.below(200)).map(|_| rng.next() as u8).collect();
        let has = rng.below(16);
        d.set_header(texrs_zlib::GzHeader {
            text: (has & 1) as i32,
            time: rng.next() as u32,
            os: rng.below(256) as i32,
            hcrc: ((has >> 1) & 1) as i32,
            name: if has & 4 != 0 { Some(name) } else { None },
            comment: if has & 8 != 0 { Some(comment) } else { None },
            extra_len: extra.len() as u32,
            extra: if n % 3 != 0 { Some(extra) } else { None },
            ..Default::default()
        });
        let mut out = vec![0u8; 100_000];
        let r = d.deflate(&src[..10_000], &mut out, Flush::Finish);
        let mut s = out[..r.produced].to_vec();
        if n % 5 == 4 {
            // corrupt header crc / flags
            let i = rng.below(12) as usize;
            s[i] ^= 0x20;
        }
        let id = format!("gh{}", n);
        h.add_data(&id, s);
        let mut c = h.case(&format!("ghdr-{}", n));
        c.use_data(&id);
        c.iinit(if n % 2 == 0 { 31 } else { 47 });
        c.igethdr(rng.below(300) as u32, rng.below(120) as u32, rng.below(120) as u32);
        let step = rng.range(1, 7) as usize;
        c.inflate_stream(move || step, || 4000, NO_FLUSH);
        c.ihdr();
        c.iend();
        c.end();
    }

    // concatenated gzip members, validate(0), prime, reset2, resetkeep
    {
        let mut all = Vec::new();
        for k in 0..3 {
            let mut d = Deflate::with_params(k * 3, 8, 31, 8, 0).unwrap();
            let mut out = vec![0u8; 100_000];
            let r = d.deflate(&src[k as usize * 1000..k as usize * 1000 + 3000], &mut out, Flush::Finish);
            all.extend_from_slice(&out[..r.produced]);
        }
        let n = all.len();
        all[n - 6] ^= 1; // bad crc in last member
        h.add_data("multi", all);
        let mut c = h.case("multi");
        c.use_data("multi");
        c.iinit(31);
        let len = c.cur_len();
        let r = c.i(0, len, 100_000, NO_FLUSH);
        let mut pos = r.consumed;
        c.ireset();
        let r = c.i(pos, len - pos, 100_000, NO_FLUSH);
        pos += r.consumed;
        c.iresetkeep();
        c.ivalidate(0);
        c.i(pos, len - pos, 100_000, NO_FLUSH);
        c.ireset2(-15);
        c.ireset2(47);
        c.ireset2(7);
        c.ireset2(-16);
        c.iprime(17, 1);
        c.iprime(-1, 0);
        c.iprime(8, 0x1f);
        c.iprime(8, 0x8b);
        c.i(2, 20, 100, NO_FLUSH);
        c.iend();
        for &w in &[8, 16, 7, -7, 48, 64, 0] {
            if c.iinit(w) == 0 {
                c.iend();
            }
        }
        c.end();
    }
    h.finish();
}
