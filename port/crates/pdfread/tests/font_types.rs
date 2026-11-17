//! Every embedded Type 1 / Type 1C font of the corpus is copied into a
//! one-page PDF under the name /CMR10 (which is in pdftex.map, so pdfTeX
//! replaces it and writes the encoding xpdf computed for it).  This
//! exercises FoFiType1 (PFA/PFB), FoFiType1C (CFF charsets/encodings)
//! and the Gfx8BitFont encoding rules on several hundred real fonts.
//! A sample is used unless `PDFREAD_ORACLE_FULL=1`.

mod common;

use std::collections::HashMap;

use common::builder::{classic_pdf, obj, obj_bytes, stream_body, Obj};
use common::epdf::check_font_encodings;
use common::*;
use pdfread::{set_err_quiet, Object, PdfDoc, Ref, XRefEntryType};

fn name_bytes(n: &[u8]) -> String {
    let mut s = String::from("/");
    for &c in n {
        if c.is_ascii_alphanumeric() || b"._-+".contains(&c) {
            s.push(c as char);
        } else {
            s.push_str(&format!("#{:02X}", c));
        }
    }
    s
}

/// Serialize an object, renumbering refs through `map` (allocating new
/// numbers and queueing the source refs).
fn ser(o: &Object, map: &mut HashMap<Ref, i32>, queue: &mut Vec<Ref>, next: &mut i32, out: &mut Vec<u8>) {
    match o {
        Object::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Object::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
        Object::Real(r) => out.extend_from_slice(format!("{}", r).as_bytes()),
        Object::String(s) => {
            out.push(b'<');
            for b in s.iter() {
                out.extend_from_slice(format!("{:02x}", b).as_bytes());
            }
            out.push(b'>');
        }
        Object::Name(n) => out.extend_from_slice(name_bytes(n).as_bytes()),
        Object::Array(a) => {
            out.push(b'[');
            for e in a.elems() {
                ser(e, map, queue, next, out);
                out.push(b' ');
            }
            out.push(b']');
        }
        Object::Dict(d) => {
            out.extend_from_slice(b"<<");
            for (k, v) in d.iter() {
                out.extend_from_slice(name_bytes(k).as_bytes());
                out.push(b' ');
                ser(v, map, queue, next, out);
                out.push(b'\n');
            }
            out.extend_from_slice(b">>");
        }
        Object::Ref(r) => {
            let n = *map.entry(*r).or_insert_with(|| {
                queue.push(*r);
                *next += 1;
                *next - 1
            });
            out.extend_from_slice(format!("{} 0 R", n).as_bytes());
        }
        _ => out.extend_from_slice(b"null"),
    }
}

/// A one-page PDF using the font `font_ref` of `src` under /CMR10.
fn font_case(src: &PdfDoc, font_ref: Ref) -> Vec<u8> {
    let xref = src.get_xref().unwrap();
    let mut map: HashMap<Ref, i32> = HashMap::new();
    let mut queue = vec![font_ref];
    map.insert(font_ref, 10);
    let mut next = 11;
    let mut objs: Vec<Obj> = vec![
        obj(1, "<< /Type /Catalog /Pages 2 0 R >>"),
        obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>"),
        obj(3, "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 10 0 R >> >> /Contents 4 0 R >>"),
        obj_bytes(4, 0, stream_body("", b"BT /F1 10 Tf (a) Tj ET")),
    ];
    let mut i = 0;
    while i < queue.len() {
        let r = queue[i];
        i += 1;
        let num = map[&r];
        let o = xref.fetch(r.num, r.gen);
        let mut body = Vec::new();
        match &o {
            Object::Stream(s) => {
                let raw = s.get_undecoded_data();
                body.extend_from_slice(b"<<");
                for (k, v) in s.get_dict().iter() {
                    if k == b"Length" {
                        continue;
                    }
                    body.extend_from_slice(name_bytes(k).as_bytes());
                    body.push(b' ');
                    ser(v, &mut map, &mut queue, &mut next, &mut body);
                    body.push(b'\n');
                }
                body.extend_from_slice(format!("/Length {}>>\nstream\n", raw.len()).as_bytes());
                body.extend_from_slice(&raw);
                body.extend_from_slice(b"\nendstream");
            }
            Object::Dict(d) if r == font_ref => {
                body.extend_from_slice(b"<<");
                for (k, v) in d.iter() {
                    body.extend_from_slice(name_bytes(k).as_bytes());
                    body.push(b' ');
                    if k == b"BaseFont" {
                        body.extend_from_slice(b"/CMR10");
                    } else {
                        ser(v, &mut map, &mut queue, &mut next, &mut body);
                    }
                    body.push(b'\n');
                }
                body.extend_from_slice(b">>");
            }
            other => ser(other, &mut map, &mut queue, &mut next, &mut body),
        }
        objs.push(obj_bytes(num, 0, body));
    }
    objs.sort_by_key(|o| o.num);
    classic_pdf(&objs, "")
}

#[test]
fn every_embedded_type1_font_encoding_matches_xpdf() {
    set_err_quiet(true);
    if tl_bin("pdftex").is_none() {
        eprintln!("pdftex not found; skipping");
        return;
    }
    let full = std::env::var("PDFREAD_ORACLE_FULL").is_ok();
    // collect (file, font ref) for embedded Type1/Type1C fonts
    let mut cases: Vec<(std::path::PathBuf, Ref)> = Vec::new();
    for f in corpus() {
        let doc = PdfDoc::open(&f);
        if !doc.is_ok() || doc.is_encrypted() {
            continue;
        }
        let x = doc.get_xref().unwrap();
        for n in 0..x.get_num_objects() {
            let e = x.get_entry(n);
            if e.type_ != XRefEntryType::Uncompressed && e.type_ != XRefEntryType::Compressed {
                continue;
            }
            let gen = if e.type_ == XRefEntryType::Uncompressed { e.gen } else { 0 };
            let o = x.fetch(n, gen);
            if o.dict_is("Font") && o.dict_lookup("Subtype").is_name_of("Type1") {
                let fd = o.dict_lookup("FontDescriptor");
                if fd.dict_lookup("FontFile").is_stream() || fd.dict_lookup("FontFile3").is_stream() {
                    cases.push((f.clone(), Ref::new(n, gen)));
                }
            }
        }
    }
    let selected: Vec<_> = cases.iter().enumerate().filter(|(i, _)| full || i % 5 == 0).map(|(_, c)| c.clone()).collect();
    let nthreads: usize = std::env::var("PDFREAD_ORACLE_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let chunks: Vec<Vec<_>> =
        (0..nthreads).map(|t| selected.iter().skip(t).step_by(nthreads).cloned().collect()).collect();
    let results: Vec<(usize, Vec<String>)> = std::thread::scope(|sc| {
        let hs: Vec<_> = chunks
            .iter()
            .map(|chunk| {
                sc.spawn(move || {
                    set_err_quiet(true);
                    let mut n = 0;
                    let mut v = Vec::new();
                    for (f, r) in chunk {
                        let doc = PdfDoc::open(f);
                        let pdf = font_case(&doc, *r);
                        match check_font_encodings(&pdf) {
                            Ok(k) => n += k,
                            Err(e) => {
                                let msg = format!("{} font {} {}: {}", f.display(), r.num, r.gen, e);
                                println!("{}", msg);
                                v.push(msg);
                            }
                        }
                    }
                    (n, v)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let fonts: usize = results.iter().map(|r| r.0).sum();
    let problems: Vec<&String> = results.iter().flat_map(|r| r.1.iter()).collect();
    println!("{} embedded fonts selected, {} compared with pdftex/xpdf", selected.len(), fonts);
    assert!(fonts > 0 || selected.is_empty());
    assert!(problems.is_empty(), "{} fonts with encoding differences", problems.len());
}
