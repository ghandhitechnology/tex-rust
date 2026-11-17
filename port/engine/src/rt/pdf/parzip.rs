//! Parallel compression of PDF streams (an engine speed-up, not part of pdfTeX).
//!
//! pdfTeX compresses each stream with zlib as its data is produced (writezip.c), so the
//! compressed size, and with it every later file offset, is known immediately. Here a
//! finished stream is instead handed to a worker thread, which repeats exactly the zlib
//! calls writezip would have made (same level, same input pieces, NO_FLUSH then FINISH),
//! so the compressed bytes are identical. Until then:
//! - the engine works with *virtual* offsets, in which every pending stream counts as
//!   empty (pdf_gone does not advance during a deferred stream);
//! - bytes after a pending stream wait in memory, so the file is still written in order;
//! - /Length patches are recorded and applied once the sizes are known;
//! - the offsets of the objects (obj_tab[].int2, all taken at "n 0 obj" and so never at
//!   a stream's own position) are recorded and translated to real offsets;
//! - pdf_last_byte gets the true last byte of the zlib stream (the low byte of the
//!   Adler-32 trailer).
//! Before the cross-reference section (the only reader of object offsets, together with
//! the final byte count), `parzip_sync` waits for all streams, writes them out, applies
//! the patches, converts the recorded offsets and switches back to compressing inline.
//! Disable with TEXRS_PAR_ZIP=0.

use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use texrs_zlib::{Deflate, Flush, Status};

use crate::gen::globals::Globals;
use crate::rt::*;

const ZIP_BUF_SIZE: usize = 32768;

struct Job {
    level: i32,
    pieces: Vec<Vec<u8>>,
    done: Sender<Result<Vec<u8>, i32>>,
}

enum Seg {
    Lit(Vec<u8>),
    /// a pending compressed stream: index into `streams`
    Stream(usize),
}

struct Stream {
    /// virtual offset of its first byte
    vpos: i64,
    rx: Option<Receiver<Result<Vec<u8>, i32>>>,
    size: Option<i64>,
}

enum PatchLen {
    Known(i64),
    Stream(usize),
}

#[derive(Default)]
pub struct ParZip {
    /// 0 = not decided yet, 1 = deferring, 2 = off (disabled or after the sync)
    mode: u8,
    workers: Option<Sender<Job>>,
    streams: Vec<Stream>,
    /// output not yet written to the file (starts with a pending stream)
    queue: VecDeque<Seg>,
    /// the stream being collected: level and input pieces
    cur: Option<(i32, Vec<Vec<u8>>, u32)>,
    /// the last finished stream and the virtual offset where it ended
    last: Option<(usize, i64)>,
    patches: Vec<(i64, PatchLen)>,
    objs: Vec<(i32, i64)>,
}

fn compress(level: i32, pieces: &[Vec<u8>]) -> Result<Vec<u8>, i32> {
    // writezip.c's calls: deflateInit (or deflateReset, the same state), then for each
    // piece deflate(NO_FLUSH) until the input is used up, the last piece with FINISH
    let mut d = Deflate::new(level).map_err(|e| e as i32)?;
    let mut out = Vec::new();
    let mut buf = vec![0u8; ZIP_BUF_SIZE];
    for (k, p) in pieces.iter().enumerate() {
        let finish = k + 1 == pieces.len();
        let mut inpos = 0;
        let mut pos = 0;
        loop {
            if pos == ZIP_BUF_SIZE {
                out.extend_from_slice(&buf);
                pos = 0;
            }
            let r = d.deflate(&p[inpos..], &mut buf[pos..], if finish { Flush::Finish } else { Flush::NoFlush });
            inpos += r.consumed;
            pos += r.produced;
            if finish && r.status == Status::StreamEnd {
                break;
            }
            if r.status != Status::Ok {
                return Err(r.status as i32);
            }
            if !finish && inpos == p.len() {
                break;
            }
        }
        out.extend_from_slice(&buf[..pos]);
    }
    Ok(out)
}

fn start_workers() -> Sender<Job> {
    let (tx, rx) = channel::<Job>();
    let rx = Arc::new(Mutex::new(rx));
    let n = std::thread::available_parallelism().map_or(2, |n| n.get()).saturating_sub(1).clamp(1, 6);
    for _ in 0..n {
        let rx = rx.clone();
        std::thread::spawn(move || loop {
            let job = match rx.lock().unwrap().recv() {
                Ok(j) => j,
                Err(_) => return,
            };
            let _ = job.done.send(compress(job.level, &job.pieces));
        });
    }
    tx
}

impl Globals {
    fn parzip_active(&mut self) -> bool {
        let z = &mut self.rt.parzip;
        if z.mode == 0 {
            z.mode = if std::env::var("TEXRS_PAR_ZIP").as_deref() == Ok("0") { 2 } else { 1 };
        }
        z.mode == 1
    }

    /// writezip(finish) while deferring: collect the stream's input.
    /// Returns false when not deferring (the caller compresses inline).
    pub fn parzip_writezip(&mut self, finish: bool) -> bool {
        if !self.parzip_active() {
            return false;
        }
        let level = self.getpdfcompresslevel();
        if self.pdfstreamlength == 0 {
            // a new stream (pdf_begin_stream reset the length)
            self.rt.parzip.cur = Some((level, Vec::new(), 1));
        }
        let input = slice(self.pdfbuf, 0, self.pdfptr.max(0) as usize).to_vec();
        let z = &mut self.rt.parzip;
        let (_, pieces, adler) = z.cur.as_mut().expect("deferred stream");
        *adler = texrs_zlib::adler32(*adler, &input);
        pieces.push(input);
        // nonzero, as after any writezip call (the zlib header is always written)
        self.pdfstreamlength = 2;
        if finish {
            let (level, pieces, adler) = z.cur.take().unwrap();
            let (tx, rx) = channel();
            let id = z.streams.len();
            z.streams.push(Stream { vpos: self.pdfgone, rx: Some(rx), size: None });
            z.queue.push_back(Seg::Stream(id));
            z.last = Some((id, self.pdfgone));
            if z.workers.is_none() {
                z.workers = Some(start_workers());
            }
            let _ = z.workers.as_ref().unwrap().send(Job { level, pieces, done: tx });
            // the last byte of a zlib stream: the low byte of its Adler-32 trailer
            self.pdflastbyte = adler as u8;
            self.parzip_drain(false);
        }
        true
    }

    /// Bytes for the PDF file (pdf_flush). Returns false when not deferring.
    pub fn parzip_write(&mut self, data: &[u8]) -> bool {
        if self.rt.parzip.mode != 1 {
            return false;
        }
        let z = &mut self.rt.parzip;
        if z.queue.is_empty() {
            let f = self.pdffile;
            self.rt.files.write(f, data);
        } else {
            match z.queue.back_mut() {
                Some(Seg::Lit(v)) => v.extend_from_slice(data),
                _ => z.queue.push_back(Seg::Lit(data.to_vec())),
            }
        }
        true
    }

    /// write_stream_length(length, offset) while deferring: record the patch.
    /// Returns false when not deferring.
    pub fn parzip_stream_length(&mut self, length: i64, offset: i64) -> bool {
        if self.rt.parzip.mode != 1 {
            return false;
        }
        let here = self.pdfgone + self.pdfptr as i64;
        let z = &mut self.rt.parzip;
        let len = match z.last.take() {
            // right after a deferred stream (pdf_end_stream): its compressed length
            Some((id, end)) if end == here => PatchLen::Stream(id),
            _ => PatchLen::Known(length),
        };
        z.patches.push((offset, len));
        true
    }

    /// obj_tab[i].int2 was just set to the (virtual) offset of object i.
    pub fn parzip_note_offset(&mut self, i: i32) {
        if self.rt.parzip.mode == 1 && !self.rt.parzip.streams.is_empty() {
            let v = ix(self.objtab, i).int2;
            self.rt.parzip.objs.push((i, v));
        }
    }

    /// Write out finished streams at the head of the queue (all of them if `wait`).
    fn parzip_drain(&mut self, wait: bool) {
        loop {
            let z = &mut self.rt.parzip;
            let Some(front) = z.queue.front() else { return };
            match front {
                Seg::Lit(_) => {
                    let Some(Seg::Lit(v)) = z.queue.pop_front() else { unreachable!() };
                    let f = self.pdffile;
                    self.rt.files.write(f, &v);
                }
                &Seg::Stream(id) => {
                    let rx = z.streams[id].rx.as_ref().unwrap();
                    let r = if wait {
                        rx.recv().ok()
                    } else {
                        match rx.try_recv() {
                            Ok(r) => Some(r),
                            Err(_) => return,
                        }
                    };
                    let data = match r {
                        Some(Ok(d)) => d,
                        Some(Err(code)) => self.pdftex_fail(&format!("zlib: deflate() failed (error code {})", code)),
                        None => self.pdftex_fail("zlib: compression thread failed"),
                    };
                    let z = &mut self.rt.parzip;
                    z.streams[id].size = Some(data.len() as i64);
                    z.streams[id].rx = None;
                    z.queue.pop_front();
                    let f = self.pdffile;
                    self.rt.files.write(f, &data);
                }
            }
        }
    }

    /// Real file offset of virtual offset v (after all streams are written).
    fn parzip_real(&self, v: i64) -> i64 {
        let mut r = v;
        for s in &self.rt.parzip.streams {
            if s.vpos < v {
                r += s.size.unwrap();
            } else {
                break;
            }
        }
        r
    }

    /// Wait for all deferred streams, write them, apply the length patches, translate
    /// the recorded object offsets and pdf_gone, and compress inline from now on.
    pub fn parzip_sync(&mut self) {
        if self.rt.parzip.mode != 1 {
            self.rt.parzip.mode = 2;
            return;
        }
        crate::rt::phase::mark("parzip sync");
        self.parzip_drain(true);
        crate::rt::phase::mark("parzip drained");
        let patches = std::mem::take(&mut self.rt.parzip.patches);
        let f = self.pdffile;
        if !patches.is_empty() {
            self.rt.files.flush(f);
        }
        let total: i64 = self.rt.parzip.streams.iter().map(|s| s.size.unwrap()).sum();
        for (off, len) in patches {
            let l = match len {
                PatchLen::Known(l) => l,
                PatchLen::Stream(id) => self.rt.parzip.streams[id].size.unwrap(),
            };
            let pos = self.parzip_real(off);
            self.rt.files.seek_out(f, pos as u64);
            self.rt.files.write(f, format!("{}", l).as_bytes());
        }
        for (i, v) in std::mem::take(&mut self.rt.parzip.objs) {
            if ix(self.objtab, i).int2 == v {
                ix(self.objtab, i).int2 = self.parzip_real(v);
            }
        }
        self.pdfgone += total;
        // back to the end of what has been written (pdf_buf holds the rest)
        self.rt.files.seek_out(f, self.pdfgone as u64);
        let z = &mut self.rt.parzip;
        z.mode = 2;
        z.streams.clear();
        z.last = None;
        z.workers = None;
    }

    /// At exit without reaching the cross-reference section (an aborted job whose PDF is
    /// kept): write out what was produced, in order.
    pub fn parzip_flush_at_exit(&mut self) {
        if self.rt.parzip.mode == 1 && (!self.rt.parzip.queue.is_empty() || !self.rt.parzip.patches.is_empty()) {
            self.parzip_sync();
        }
    }
}

impl Globals {
    /// The PDF file is about to be removed (a failed job): drop what is pending.
    pub fn parzip_abandon(&mut self) {
        let z = &mut self.rt.parzip;
        z.queue.clear();
        z.mode = 2;
    }
}
