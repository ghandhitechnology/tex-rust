//! A small PDF writer for hand-made test files: classic xref tables,
//! xref streams (with /Index and predictors), object streams, hybrid
//! files, incremental updates and deliberately broken variants.

#![allow(dead_code)]

/// zlib stream made of stored (uncompressed) deflate blocks.
pub fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    if data.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xff, 0xff]);
    }
    let mut chunks = data.chunks(65535).peekable();
    while let Some(c) = chunks.next() {
        let last = chunks.peek().is_none();
        out.push(if last { 1 } else { 0 });
        let len = c.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(c);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, v: u32, bits: u32) {
        self.acc |= v << self.n;
        self.n += bits;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }
    fn put_rev(&mut self, code: u32, bits: u32) {
        let mut r = 0;
        for i in 0..bits {
            r |= ((code >> i) & 1) << (bits - 1 - i);
        }
        self.put(r, bits);
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// zlib stream with one fixed-Huffman block; uses simple run-length
/// matches (distance 1) so that length/distance decoding is exercised.
pub fn zlib_fixed(data: &[u8]) -> Vec<u8> {
    let mut w = BitWriter { out: vec![0x78, 0x9c], acc: 0, n: 0 };
    w.put(1, 1); // BFINAL
    w.put(1, 2); // fixed
    let lit = |w: &mut BitWriter, v: u32| {
        if v < 144 {
            w.put_rev(0x30 + v, 8)
        } else if v < 256 {
            w.put_rev(0x190 + v - 144, 9)
        } else if v < 280 {
            w.put_rev(v - 256, 7)
        } else {
            w.put_rev(0xc0 + v - 280, 8)
        }
    };
    let mut i = 0;
    while i < data.len() {
        // run of the previous byte?
        if i > 0 {
            let mut run = 0;
            while i + run < data.len() && data[i + run] == data[i - 1] && run < 10 {
                run += 1;
            }
            if run >= 3 {
                // lengths 3..10 are codes 257..264 with no extra bits
                lit(&mut w, 257 + (run as u32 - 3));
                w.put_rev(0, 5); // distance code 0 = distance 1
                i += run;
                continue;
            }
        }
        lit(&mut w, data[i] as u32);
        i += 1;
    }
    lit(&mut w, 256);
    let mut out = w.finish();
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// PNG "Up" predictor encoding (Predictor 12) of rows of `cols` bytes.
pub fn png_up(data: &[u8], cols: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut prev = vec![0u8; cols];
    for row in data.chunks(cols) {
        out.push(2);
        for (j, &b) in row.iter().enumerate() {
            out.push(b.wrapping_sub(prev[j]));
        }
        prev[..row.len()].copy_from_slice(row);
    }
    out
}

/// An object to write: `num gen obj <body> endobj`.
#[derive(Clone)]
pub struct Obj {
    pub num: i32,
    pub gen: i32,
    pub body: Vec<u8>,
}

pub fn obj(num: i32, body: &str) -> Obj {
    Obj { num, gen: 0, body: body.as_bytes().to_vec() }
}

pub fn obj_bytes(num: i32, gen: i32, body: Vec<u8>) -> Obj {
    Obj { num, gen, body }
}

/// A stream object body: `<<dict /Length n>>\nstream\n<data>\nendstream`.
pub fn stream_body(dict_entries: &str, data: &[u8]) -> Vec<u8> {
    let mut v = format!("<<{} /Length {}>>\nstream\n", dict_entries, data.len()).into_bytes();
    v.extend_from_slice(data);
    v.extend_from_slice(b"\nendstream");
    v
}

/// Writes objects after a header; returns (bytes, offsets by object).
pub fn write_objects(header: &[u8], objs: &[Obj]) -> (Vec<u8>, Vec<(i32, i32, usize)>) {
    let mut out = header.to_vec();
    let mut offs = Vec::new();
    for o in objs {
        offs.push((o.num, o.gen, out.len()));
        out.extend_from_slice(format!("{} {} obj\n", o.num, o.gen).as_bytes());
        out.extend_from_slice(&o.body);
        out.extend_from_slice(b"\nendobj\n");
    }
    (out, offs)
}

/// Classic xref table text for the given entries (`base` is subtracted
/// from offsets, i.e. the position of `%PDF-`).
pub fn xref_table(offs: &[(i32, i32, usize)], base: usize) -> (String, i32) {
    let size = offs.iter().map(|o| o.0).max().unwrap_or(0) + 1;
    let mut s = format!("xref\n0 {}\n0000000000 65535 f \n", size);
    for n in 1..size {
        match offs.iter().find(|o| o.0 == n) {
            Some(&(_, g, off)) => s.push_str(&format!("{:010} {:05} n \n", off - base, g)),
            None => s.push_str("0000000000 65535 f \n"),
        }
    }
    (s, size)
}

/// A complete PDF with a classic xref table.  `trailer_extra` is added to
/// the trailer dictionary (which always has /Size and /Root 1 0 R).
pub fn classic_pdf(objs: &[Obj], trailer_extra: &str) -> Vec<u8> {
    classic_pdf_with_header(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n", objs, trailer_extra, 0)
}

pub fn classic_pdf_with_header(header: &[u8], objs: &[Obj], trailer_extra: &str, base: usize) -> Vec<u8> {
    let (mut out, offs) = write_objects(header, objs);
    let xref_pos = out.len();
    let (table, size) = xref_table(&offs, base);
    out.extend_from_slice(table.as_bytes());
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R{} >>\nstartxref\n{}\n%%EOF\n", size, trailer_extra, xref_pos - base)
            .as_bytes(),
    );
    out
}

/// One entry of an xref stream.
#[derive(Clone, Copy)]
pub enum XEntry {
    Free(i64, i64),
    Uncompressed(i64, i64),
    Compressed(i64, i64),
}

/// Encode xref stream rows with field widths `w`.
pub fn xref_rows(entries: &[XEntry], w: [usize; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    let put = |out: &mut Vec<u8>, v: i64, n: usize| {
        for i in (0..n).rev() {
            out.push(((v >> (8 * i)) & 0xff) as u8);
        }
    };
    for e in entries {
        let (t, a, b) = match *e {
            XEntry::Free(a, b) => (0, a, b),
            XEntry::Uncompressed(a, b) => (1, a, b),
            XEntry::Compressed(a, b) => (2, a, b),
        };
        put(&mut out, t, w[0]);
        put(&mut out, a, w[1]);
        put(&mut out, b, w[2]);
    }
    out
}

/// An object stream body holding `objs` (num, text), plus the header.
pub fn object_stream(objs: &[(i32, &str)], extra: &str, compress: bool) -> Vec<u8> {
    let mut header = String::new();
    let mut body = String::new();
    for (n, t) in objs {
        header.push_str(&format!("{} {} ", n, body.len()));
        body.push_str(t);
        body.push('\n');
    }
    let first = header.len();
    let data = format!("{}{}", header, body).into_bytes();
    if compress {
        stream_body(
            &format!("/Type /ObjStm /N {} /First {} /Filter /FlateDecode{}", objs.len(), first, extra),
            &zlib_fixed(&data),
        )
    } else {
        stream_body(&format!("/Type /ObjStm /N {} /First {}{}", objs.len(), first, extra), &data)
    }
}
