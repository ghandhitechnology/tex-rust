//! Port of pdftexdir/writezip.c: compress PDF stream data with zlib (texrs-zlib, a
//! byte-exact port of zlib 1.3.2), same buffer size and call sequence as the C code.

use texrs_zlib::{Deflate, Flush, Status};

use crate::gen::globals::Globals;
use crate::rt::*;

const ZIP_BUF_SIZE: usize = 32768;

#[derive(Default)]
pub struct ZipState {
    zipbuf: Vec<u8>,
    /// ZIP_BUF_SIZE - c_stream.avail_out
    out_pos: usize,
    stream: Option<Deflate>,
    level_old: i32,
}

impl Globals {
    fn zip_check(&mut self, st: Status, func: &str) {
        if st != Status::Ok {
            self.pdftex_fail(&format!("zlib: {}() failed (error code {})", func, st as i32));
        }
    }

    pub fn writezip(&mut self, finish: bool) {
        let level = self.getpdfcompresslevel();
        assert!(level > 0);
        self.rt.pdfu.cur_file_name = None;
        if self.parzip_writezip(finish) {
            return;
        }
        if self.pdfstreamlength == 0 {
            if self.rt.zip.stream.is_none() {
                self.rt.zip.zipbuf = vec![0u8; ZIP_BUF_SIZE];
                match Deflate::new(level) {
                    Ok(d) => self.rt.zip.stream = Some(d),
                    Err(e) => self.zip_check(e, "deflateInit"),
                }
            } else if level != self.rt.zip.level_old {
                let old = self.rt.zip.stream.take().unwrap();
                let st = old.end();
                self.zip_check(st, "deflateEnd");
                match Deflate::new(level) {
                    Ok(d) => self.rt.zip.stream = Some(d),
                    Err(e) => self.zip_check(e, "deflateInit"),
                }
            } else {
                let st = self.rt.zip.stream.as_mut().unwrap().reset();
                self.zip_check(st, "deflateReset");
            }
            self.rt.zip.level_old = level;
            self.rt.zip.out_pos = 0;
        }
        let input = slice(self.pdfbuf, 0, self.pdfptr.max(0) as usize).to_vec();
        let mut inpos = 0usize;
        loop {
            if self.rt.zip.out_pos == ZIP_BUF_SIZE {
                let f = self.pdffile;
                let data = std::mem::take(&mut self.rt.zip.zipbuf);
                self.rt.files.write(f, &data);
                self.pdfgone += ZIP_BUF_SIZE as i64;
                self.pdflastbyte = data[ZIP_BUF_SIZE - 1];
                self.rt.zip.zipbuf = data;
                self.rt.zip.out_pos = 0;
            }
            let flush = if finish { Flush::Finish } else { Flush::NoFlush };
            let z = &mut self.rt.zip;
            let pos = z.out_pos;
            let r = z.stream.as_mut().unwrap().deflate(&input[inpos..], &mut z.zipbuf[pos..], flush);
            inpos += r.consumed;
            z.out_pos += r.produced;
            if finish && r.status == Status::StreamEnd {
                break;
            }
            self.zip_check(r.status, "deflate");
            if !finish && inpos == input.len() {
                break;
            }
        }
        if finish {
            let n = self.rt.zip.out_pos;
            if n > 0 {
                let f = self.pdffile;
                let data = self.rt.zip.zipbuf[..n].to_vec();
                self.rt.files.write(f, &data);
                self.pdfgone += n as i64;
                self.pdflastbyte = data[n - 1];
            }
            let f = self.pdffile;
            self.rt.files.flush(f);
        }
        self.pdfstreamlength = self.rt.zip.stream.as_ref().unwrap().total_out() as i64;
    }

    /// writezip.c zip_free
    pub fn zip_free(&mut self) {
        if let Some(d) = self.rt.zip.stream.take() {
            let st = d.end();
            self.zip_check(st, "deflateEnd");
        }
    }
}
