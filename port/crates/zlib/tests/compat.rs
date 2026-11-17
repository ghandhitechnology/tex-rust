//! The C-named compatibility layer: a literal transliteration of pdfTeX's
//! writezip() (texk/web2c/pdftexdir/writezip.c) on `z_stream` must produce
//! exactly what the safe API produces, and the gz functions must round trip.

mod common;
use common::*;
use texrs_zlib::zlib::*;
use texrs_zlib::{Deflate, Flush, Status};

const ZIP_BUF_SIZE: usize = 32768;

/// pdfTeX's writezip() state.
struct Writer {
    zipbuf: Vec<u8>,
    c_stream: z_stream,
    level_old: i32,
    stream_length: u64,
    pdf: Vec<u8>,
    started: bool,
}

impl Writer {
    fn new() -> Writer {
        Writer {
            zipbuf: Vec::new(),
            c_stream: z_stream::default(),
            level_old: 0,
            stream_length: 0,
            pdf: Vec::new(),
            started: false,
        }
    }

    /// writezip(finish) with pdf_buf = `pdfbuf`; `new_stream` is
    /// `pdfstreamlength == 0`.
    fn writezip(&mut self, pdfbuf: &[u8], finish: bool, level: i32, new_stream: bool) {
        unsafe {
            if new_stream {
                if !self.started {
                    self.zipbuf = vec![0u8; ZIP_BUF_SIZE];
                    assert_eq!(deflateInit(&mut self.c_stream, level), Z_OK);
                    self.started = true;
                } else if level != self.level_old {
                    deflateEnd(&mut self.c_stream);
                    assert_eq!(deflateInit(&mut self.c_stream, level), Z_OK);
                } else {
                    assert_eq!(deflateReset(&mut self.c_stream), Z_OK);
                }
                self.level_old = level;
                self.c_stream.next_out = self.zipbuf.as_mut_ptr();
                self.c_stream.avail_out = ZIP_BUF_SIZE as u32;
            }
            self.c_stream.next_in = pdfbuf.as_ptr();
            self.c_stream.avail_in = pdfbuf.len() as u32;
            loop {
                if self.c_stream.avail_out == 0 {
                    self.pdf.extend_from_slice(&self.zipbuf);
                    self.c_stream.next_out = self.zipbuf.as_mut_ptr();
                    self.c_stream.avail_out = ZIP_BUF_SIZE as u32;
                }
                let err = deflate(&mut self.c_stream, if finish { Z_FINISH } else { Z_NO_FLUSH });
                if finish && err == Z_STREAM_END {
                    break;
                }
                assert_eq!(err, Z_OK);
                if !finish && self.c_stream.avail_in == 0 {
                    break;
                }
            }
            if finish && (self.c_stream.avail_out as usize) < ZIP_BUF_SIZE {
                let n = ZIP_BUF_SIZE - self.c_stream.avail_out as usize;
                self.pdf.extend_from_slice(&self.zipbuf[..n]);
            }
            self.stream_length = self.c_stream.total_out as u64;
        }
    }
}

fn safe_compress(data: &[u8], level: i32) -> Vec<u8> {
    let mut d = Deflate::new(level).unwrap();
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut pos = 0;
    loop {
        let r = d.deflate(&data[pos..], &mut buf, Flush::Finish);
        out.extend_from_slice(&buf[..r.produced]);
        pos += r.consumed;
        if r.status == Status::StreamEnd {
            return out;
        }
    }
}

#[test]
fn writezip_transliteration() {
    let mut rng = Rng::new(3);
    let mut w = Writer::new();
    let mut expected = Vec::new();
    for k in 0..40 {
        let n = rng.range(0, 200_000) as usize;
        let data = match k % 3 {
            0 => pdf_like(&mut rng, n),
            1 => text(&mut rng, n / 2),
            _ => fmt_like(&mut rng, n),
        };
        let level = [6, 6, 9, 1, 6][k % 5];
        // pdfTeX flushes its 16 KiB pdf_buf through writezip(false)
        let chunks: Vec<&[u8]> = data.chunks(16384).collect();
        if chunks.is_empty() {
            w.writezip(&[], true, level, true);
        } else {
            for (i, c) in chunks.iter().enumerate() {
                w.writezip(c, i + 1 == chunks.len(), level, i == 0);
            }
        }
        let e = safe_compress(&data, level);
        assert_eq!(w.stream_length, e.len() as u64);
        expected.extend_from_slice(&e);
    }
    assert!(w.pdf == expected);
    assert_eq!(deflateEnd(&mut w.c_stream), Z_OK);
}

#[test]
fn z_stream_inflate_and_utilities() {
    let mut rng = Rng::new(4);
    let data = text(&mut rng, 300_000);
    let comp = safe_compress(&data, 9);
    let mut strm = z_stream::default();
    assert_eq!(inflateInit(&mut strm), Z_OK);
    let mut out = vec![0u8; 1000];
    let mut got = Vec::new();
    strm.next_in = comp.as_ptr();
    strm.avail_in = 0;
    let mut fed = 0usize;
    loop {
        if strm.avail_in == 0 {
            let n = (comp.len() - fed).min(777);
            strm.next_in = comp[fed..].as_ptr();
            strm.avail_in = n as u32;
            fed += n;
        }
        strm.next_out = out.as_mut_ptr();
        strm.avail_out = out.len() as u32;
        let r = unsafe { inflate(&mut strm, Z_NO_FLUSH) };
        got.extend_from_slice(&out[..1000 - strm.avail_out as usize]);
        if r == Z_STREAM_END {
            break;
        }
        assert_eq!(r, Z_OK);
    }
    assert!(got == data);
    assert_eq!(strm.total_out as usize, data.len());
    assert_eq!(strm.total_in as usize, comp.len());
    assert_eq!(strm.adler as u32, texrs_zlib::adler32(1, &data));
    assert_eq!(inflateEnd(&mut strm), Z_OK);
    assert_eq!(inflateEnd(&mut strm), Z_STREAM_ERROR);

    // compress2 / uncompress / bounds
    let mut dest = vec![0u8; compressBound(data.len() as uLong) as usize];
    let mut dl = dest.len() as uLong;
    assert_eq!(compress2(&mut dest, &mut dl, &data, 9), Z_OK);
    assert!(dest[..dl as usize] == comp[..]);
    let mut back = vec![0u8; data.len()];
    let mut bl = back.len() as uLong;
    assert_eq!(uncompress(&mut back, &mut bl, &dest[..dl as usize]), Z_OK);
    assert!(back == data);
    assert_eq!(zError(Z_DATA_ERROR), "data error");
    assert_eq!(zlibVersion(), "1.3.2");
    // deflateBound(Z_NULL, n): max(fixed, stored) bound + 18
    let fixed = 1000 + (1000 >> 3) + (1000 >> 8) + (1000 >> 9) + 4;
    assert_eq!(deflateBound(None, 1000), fixed + 18);
}

#[test]
fn gz_functions() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("compat-gz");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.gz");
    let mut rng = Rng::new(5);
    let data = fmt_like(&mut rng, 500_000);
    // pdfTeX: gzdopen(fd, "wb"), gzsetparams(f, 1, Z_DEFAULT_STRATEGY)
    let f = std::fs::File::create(&path).unwrap();
    let mut gz = gzdopen(f, "wb").unwrap();
    assert_eq!(gzsetparams(&mut gz, 1, Z_DEFAULT_STRATEGY), Z_OK);
    for c in data.chunks(4) {
        assert_eq!(gzwrite(&mut gz, c), c.len() as i32);
    }
    assert_eq!(gzclose(gz), Z_OK);
    // reading back with gzdopen "rb" and gzread
    let f = std::fs::File::open(&path).unwrap();
    let mut gz = gzdopen(f, "rb").unwrap();
    let mut back = vec![0u8; data.len() + 10];
    let mut n = 0;
    loop {
        let k = gzread(&mut gz, &mut back[n..(n + 9999).min(data.len() + 10)]);
        assert!(k >= 0);
        if k == 0 {
            break;
        }
        n += k as usize;
    }
    assert_eq!(n, data.len());
    assert!(back[..n] == data[..]);
    assert_eq!(gzeof(&gz), 1);
    assert_eq!(gzclose(gz), Z_OK);
    // gzopen by path, rewind and seek
    let mut gz = gzopen(path.to_str().unwrap(), "rb").unwrap();
    let mut b = [0u8; 100];
    assert_eq!(gzread(&mut gz, &mut b), 100);
    assert_eq!(gzseek(&mut gz, 1000, 0), 1000);
    assert_eq!(gzread(&mut gz, &mut b), 100);
    assert!(b[..] == data[1000..1100]);
    assert_eq!(gzrewind(&mut gz), 0);
    assert_eq!(gzgetc(&mut gz), data[0] as i32);
    assert_eq!(gzungetc(7, &mut gz), 7);
    assert_eq!(gzgetc(&mut gz), 7);
    assert_eq!(gztell(&gz), 1);
    assert_eq!(gzclose(gz), Z_OK);
}
