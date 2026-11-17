//! Hand-made edge-case PDFs.  Each file is checked three ways:
//! 1. direct assertions on the values we parse (expected values follow
//!    from xpdf's Lexer/Parser/XRef code),
//! 2. TeX Live's `pdftosrc` (real xpdf): xref dump and decoded streams,
//! 3. TeX Live's `pdftex` (real xpdf + pdftoepdf): page 1 is included and
//!    every object pdfTeX re-serialized is compared with ours.
//! (2) and (3) are skipped if the binaries are missing.

mod common;

use common::builder::*;
use common::epdf::check_with_pdftex;
use common::*;
use pdfread::{set_err_quiet, Object, PdfDoc};

/// Run the pdftosrc and pdftex oracles on a file.
fn oracle(name: &str, pdf: &[u8]) {
    oracle_opts(name, pdf, true)
}

fn find_bytes(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn oracle_opts(name: &str, pdf: &[u8], with_pdftex: bool) {
    set_err_quiet(true);
    let doc = PdfDoc::from_bytes(pdf.to_vec());
    if let Some(tool) = tl_bin("pdftosrc") {
        let tmp = TempDir::new("edge");
        let theirs = pdftosrc(&tool, tmp.path(), pdf, -1, 0);
        assert_eq!(doc.is_ok(), theirs.is_some(), "{}: open status differs from xpdf", name);
        if let Some(t) = theirs {
            let ours = xref_dump(&doc);
            assert!(
                normalize_dump(&ours) == normalize_dump(&t),
                "{}: xref dump differs\nours:\n{}\nxpdf:\n{}",
                name,
                String::from_utf8_lossy(&ours),
                String::from_utf8_lossy(&t)
            );
            for (num, gen) in stream_objects(&doc) {
                let o = doc.get_xref().unwrap().fetch(num, gen);
                let ours = o.get_stream().unwrap().get_decoded_data().unwrap();
                let theirs = pdftosrc(&tool, tmp.path(), pdf, num, gen).expect("pdftosrc stream");
                assert!(
                    ours == theirs,
                    "{}: stream {} {} differs\nours: {:?}\nxpdf: {:?}",
                    name,
                    num,
                    gen,
                    String::from_utf8_lossy(&ours),
                    String::from_utf8_lossy(&theirs)
                );
            }
        }
    }
    if !with_pdftex {
        return;
    }
    if let Err(e) = check_with_pdftex(pdf, 1) {
        panic!("{}: pdftex inclusion differs:\n{}", name, e);
    }
}

const CATALOG: &str = "<< /Type /Catalog /Pages 2 0 R >>";
const PAGES: &str = "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 100] >>";
const CONTENT: &[u8] = b"q 1 0 0 1 10 10 cm 0 0 m 50 50 l S Q";

fn page_with(extra: &str) -> String {
    format!("<< /Type /Page /Parent 2 0 R /Resources << /ProcSet [/PDF] >> /Contents 4 0 R {} >>", extra)
}

fn basic_objs(piece_info: &str) -> Vec<Obj> {
    vec![
        obj(1, CATALOG),
        obj(2, PAGES),
        obj(3, &page_with(&format!("/PieceInfo {}", piece_info))),
        obj_bytes(4, 0, stream_body("", CONTENT)),
    ]
}

fn piece_info(doc: &PdfDoc) -> Object {
    let cat = doc.get_catalog().unwrap();
    let r = cat.get_page_ref(1).unwrap();
    doc.get_xref().unwrap().fetch(r.num, r.gen).dict_lookup("PieceInfo")
}

// ------------------------------------------------------------------------
// numbers
// ------------------------------------------------------------------------

#[test]
fn numbers() {
    let pi = "<< /Ints [0 -0 +3 -17 007 2147483647 2147483648 4294967296 4294967297 \
              99999999999999999999 --5 50-100 -50-100 - + 12-] \
              /Reals [.5 -.0 1. -.5 +.25 3.14159265358979 0.000001 0.0000004 -0.0000006 \
              1.-2 --1.25 -. . 0.1234565 2.5000005 32767.99999999 12.5-3 0.30000000000000004] >>";
    let pdf = classic_pdf(&basic_objs(pi), "");
    let doc = PdfDoc::from_bytes(pdf.clone());
    assert!(doc.is_ok());
    let p = piece_info(&doc);
    let ints: Vec<i32> = (0..16).map(|i| p.dict_lookup("Ints").array_get(i).get_int()).collect();
    assert_eq!(
        ints,
        vec![
            0,
            0,
            3,
            -17,
            7,
            2147483647,
            -2147483648, // int overflow wraps
            0,
            1,
            7766279631452241919u64 as i64 as i32, // 10^20-1 mod 2^32, signed
            0,                                   // "--5"
            50,                                  // "50-100"
            -50,
            0, // "-"
            0, // "+"
            12,
        ]
    );
    let reals = p.dict_lookup("Reals");
    assert!(reals.array_get(0).is_real() && reals.array_get(0).get_real() == 0.5);
    let r1 = reals.array_get(1).get_real();
    assert!(r1 == 0.0 && r1.is_sign_negative(), "-.0 is a negative zero real");
    assert!(reals.array_get(2).is_real() && reals.array_get(2).get_real() == 1.0);
    // "1.-2": the '-' is skipped with a warning and the digit continues
    // the fraction
    let v = reals.array_get(9).get_real();
    assert_eq!(v, 1.0 + 0.1 * 2.0);
    assert_eq!(reals.array_get(10).get_real(), -1.25); // "--1.25"
    // fractions accumulate as xf + scale*digit with scale *= 0.1
    let mut xf = 0.0f64;
    let mut scale = 0.1f64;
    for d in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 5.0] {
        xf += scale * d;
        scale *= 0.1;
    }
    assert_eq!(reals.array_get(13).get_real().to_bits(), xf.to_bits());
    // "12.5-3" -> 12.53
    assert_eq!(reals.array_get(16).get_real(), 12.0 + 0.1 * 5.0 + 0.1 * 0.1 * 3.0);
    oracle("numbers", &pdf);
}

#[test]
fn huge_real_keeps_1e20_prefix() {
    // xf stops accumulating once >= 1e20; the fraction is still added
    let pdf = classic_pdf(&basic_objs("<< /R [123456789012345678901234.5] >>"), "");
    let doc = PdfDoc::from_bytes(pdf);
    let r = piece_info(&doc).dict_lookup("R").array_get(0).get_real();
    let mut xf = 0.0f64;
    for d in "123456789012345678901".bytes() {
        if xf < 1e20 {
            xf = xf * 10.0 + (d - b'0') as f64;
        }
    }
    assert_eq!(r, xf + 0.1 * 5.0);
}

// ------------------------------------------------------------------------
// strings
// ------------------------------------------------------------------------

#[test]
fn strings() {
    let mut pi: Vec<u8> = Vec::new();
    pi.extend_from_slice(b"<< /S [(simple) (a\\nb\\\\c\\)d\\(e\\r\\t\\b\\f) (\\101\\1012\\0\\7) (\\777\\400) ");
    pi.extend_from_slice(b"(oct\\0123) (nested (paren) ok) (unknown \\q escape) (cont\\\ninued) ");
    pi.extend_from_slice(b"(cont\\\r\ninued2) (cr\rlf\r\nx\n) (cont3\\\rx) <414> < 41 4\n2 > <> <4g41> ");
    pi.extend_from_slice(b"(bin\\200\\377\xe9) (nul\\000mid) (del\x7f) (end\\\\) <61626>] >>");
    let page = format!(
        "<< /Type /Page /Parent 2 0 R /Resources << /ProcSet [/PDF] >> /Contents 4 0 R /PieceInfo {} >>",
        String::from_utf8_lossy(&pi)
    );
    // keep the raw bytes (latin-1 \xe9) intact
    let mut page_bytes = b"<< /Type /Page /Parent 2 0 R /Resources << /ProcSet [/PDF] >> /Contents 4 0 R /PieceInfo ".to_vec();
    page_bytes.extend_from_slice(&pi);
    page_bytes.extend_from_slice(b" >>");
    let _ = page;
    let objs = vec![
        obj(1, CATALOG),
        obj(2, PAGES),
        obj_bytes(3, 0, page_bytes),
        obj_bytes(4, 0, stream_body("", CONTENT)),
    ];
    let pdf = classic_pdf(&objs, "");
    let doc = PdfDoc::from_bytes(pdf.clone());
    assert!(doc.is_ok());
    let s = piece_info(&doc).dict_lookup("S");
    let get = |i: i32| s.array_get(i).get_string().to_vec();
    assert_eq!(get(0), b"simple");
    assert_eq!(get(1), b"a\nb\\c)d(e\r\t\x08\x0c");
    assert_eq!(get(2), b"AA2\x00\x07");
    assert_eq!(get(3), &[0xff, 0x00]); // \777 keeps the low 8 bits
    assert_eq!(get(4), b"oct\n3");
    assert_eq!(get(5), b"nested (paren) ok");
    assert_eq!(get(6), b"unknown q escape");
    assert_eq!(get(7), b"continued");
    assert_eq!(get(8), b"continued2");
    assert_eq!(get(9), b"cr\nlf\nx\n");
    assert_eq!(get(10), b"cont3x");
    assert_eq!(get(11), b"A@"); // odd hex length: last digit padded
    assert_eq!(get(12), b"AB");
    assert_eq!(get(13), b"");
    assert_eq!(get(14), b"@A"); // illegal 'g' is dropped but counts as a digit
    assert_eq!(get(15), b"bin\x80\xff\xe9");
    assert_eq!(get(16), b"nul\x00mid");
    assert_eq!(get(17), b"del\x7f");
    assert_eq!(get(18), b"end\\");
    assert_eq!(get(19), b"ab`");
    oracle("strings", &pdf);
}

// ------------------------------------------------------------------------
// names, dictionaries, arrays
// ------------------------------------------------------------------------

#[test]
fn names_and_dicts() {
    let long = "x".repeat(140);
    let pi = format!(
        "<< /N [/A#20B /#41 /a#2 /a#zz /a#0G /Name.With-_+ /{} / /Hash#23 /lower#6a] \
         /Dup << /A 1 /B 2 /A 3 >> \
         /BadKey << 1 /K 2 (s) /L 3 >> \
         /Deep [1 [2 [3 [4 << /E [null true false] >>]]]] \
         /Ref 5 0 R /Gen1 6 1 R /Missing 99 0 R /FreeObj 7 0 R /WrongGen 6 0 R >>",
        long
    );
    let mut objs = basic_objs(&pi);
    objs.push(obj(5, "[/in-five (five) 5.5]"));
    objs.push(obj_bytes(6, 1, b"<< /gen 1 >>".to_vec()));
    objs.push(obj(7, "(free but present)"));
    let mut pdf = classic_pdf(&objs, "");
    // mark object 7 free in the xref table
    let off7 = find_bytes(&pdf, b"
7 0 obj").unwrap() + 1;
    let pat = format!("{:010} 00000 n", off7);
    let pos = find_bytes(&pdf, pat.as_bytes()).unwrap();
    pdf[pos + 17] = b'f';
    let doc = PdfDoc::from_bytes(pdf.clone());
    assert!(doc.is_ok());
    let p = piece_info(&doc);
    let n = p.dict_lookup("N");
    let name = |i: i32| n.array_get(i).get_name().to_vec();
    assert_eq!(name(0), b"A B");
    assert_eq!(name(1), b"A");
    assert_eq!(name(2), b"a\x02"); // "#2" + delimiter: the nibble value is kept
    assert_eq!(name(3), b"a#zz");
    assert_eq!(name(4), b"a"); // "#0G" stores a NUL, which ends the C string
    assert_eq!(name(5), b"Name.With-_+");
    assert_eq!(name(6), long.as_bytes());
    assert_eq!(name(7), b"");
    assert_eq!(name(8), b"Hash#");
    assert_eq!(name(9), b"lowerj");
    let dup = p.dict_lookup("Dup");
    let d = dup.get_dict().unwrap();
    assert_eq!(d.get_length(), 2);
    assert_eq!(&*d.get_key(0), b"A");
    assert_eq!(d.get_val(0).get_int(), 3);
    assert_eq!(&*d.get_key(1), b"B");
    let bk = p.dict_lookup("BadKey");
    let d = bk.get_dict().unwrap();
    assert_eq!(d.get_length(), 2);
    assert_eq!(d.lookup("K").get_int(), 2);
    assert_eq!(d.lookup("L").get_int(), 3);
    assert!(p.dict_lookup("Missing").is_null());
    assert!(p.dict_lookup("FreeObj").is_null());
    assert!(p.dict_lookup("WrongGen").is_null());
    assert_eq!(p.dict_lookup("Gen1").dict_lookup("gen").get_int(), 1);
    oracle("names_and_dicts", &pdf);
}

// ------------------------------------------------------------------------
// xref variants
// ------------------------------------------------------------------------

fn xref_stream_pdf(index: bool, predictor: bool, with_objstm: bool) -> Vec<u8> {
    // objects 1..4 classic, 5..7 in object stream 8, xref stream 9
    let mut objs = vec![
        obj(1, CATALOG),
        obj(2, PAGES),
        obj(3, &page_with("/PieceInfo << /A 5 0 R /B 6 0 R /C 7 0 R >>")),
        obj_bytes(4, 0, stream_body("", CONTENT)),
    ];
    if with_objstm {
        objs.push(obj_bytes(
            8,
            0,
            object_stream(&[(5, "[1 2.5 (in objstm)]"), (6, "<< /X /Y /Z 5 0 R >>"), (7, "(seven)")], " /Extends 12 0 R", true),
        ));
    } else {
        objs.push(obj(5, "[1 2.5 (in objstm)]"));
        objs.push(obj(6, "<< /X /Y /Z 5 0 R >>"));
        objs.push(obj(7, "(seven)"));
    }
    let (mut out, offs) = write_objects(b"%PDF-1.5\n%\xe2\xe3\xcf\xd3\n", &objs);
    let off = |n: i32| offs.iter().find(|o| o.0 == n).map(|o| o.2 as i64).unwrap_or(0);
    let xref_pos = out.len();
    let mut entries = vec![XEntry::Free(0, 65535)];
    for n in 1..=4 {
        entries.push(XEntry::Uncompressed(off(n), 0));
    }
    if with_objstm {
        for (i, _) in (5..=7).enumerate() {
            entries.push(XEntry::Compressed(8, i as i64));
        }
        entries.push(XEntry::Uncompressed(off(8), 0));
    } else {
        for n in 5..=7 {
            entries.push(XEntry::Uncompressed(off(n), 0));
        }
        entries.push(XEntry::Free(0, 0));
    }
    entries.push(XEntry::Uncompressed(xref_pos as i64, 0));
    let w = [1usize, 3, 2];
    let (rows, index_str) = if index {
        // two subsections: 0..5 and 5..10, listed in reverse order
        let mut r = xref_rows(&entries[5..10], w);
        r.extend(xref_rows(&entries[0..5], w));
        (r, " /Index [5 5 0 5]")
    } else {
        (xref_rows(&entries, w), "")
    };
    let (data, parms) = if predictor {
        (zlib_fixed(&png_up(&rows, 6)), " /DecodeParms << /Predictor 12 /Columns 6 >>")
    } else {
        (zlib_stored(&rows), "")
    };
    let body = stream_body(
        &format!("/Type /XRef /Size 10 /W [1 3 2] /Root 1 0 R /Filter /FlateDecode{}{}", index_str, parms),
        &data,
    );
    out.extend_from_slice(b"9 0 obj\n");
    out.extend_from_slice(&body);
    out.extend_from_slice(format!("\nendobj\nstartxref\n{}\n%%EOF\n", xref_pos).as_bytes());
    out
}

#[test]
fn xref_streams_and_object_streams() {
    for (index, pred, objstm) in [(false, false, false), (true, false, true), (false, true, true), (true, true, true)] {
        let pdf = xref_stream_pdf(index, pred, objstm);
        let doc = PdfDoc::from_bytes(pdf.clone());
        assert!(doc.is_ok(), "index {} pred {} objstm {}", index, pred, objstm);
        let p = piece_info(&doc);
        assert_eq!(p.dict_lookup("A").array_get(2).get_string(), b"in objstm");
        assert_eq!(p.dict_lookup("B").dict_lookup("Z").array_get(1).get_real(), 2.5);
        assert_eq!(p.dict_lookup("C").get_string(), b"seven");
        assert!(!doc.get_xref().unwrap().is_repaired());
        oracle(&format!("xrefstm i{} p{} o{}", index, pred, objstm), &pdf);
    }
}

#[test]
fn hybrid_file() {
    // classic table lists 1-4; the XRefStm lists the compressed 5-7
    let objs = vec![
        obj(1, CATALOG),
        obj(2, PAGES),
        obj(3, &page_with("/PieceInfo << /A 5 0 R /C 7 0 R >>")),
        obj_bytes(4, 0, stream_body("", CONTENT)),
        obj_bytes(8, 0, object_stream(&[(5, "(five from objstm)"), (6, "6"), (7, "(seven)")], "", false)),
    ];
    let (mut out, offs) = write_objects(b"%PDF-1.5\n", &objs);
    let off = |n: i32| offs.iter().find(|o| o.0 == n).map(|o| o.2 as i64).unwrap();
    let xs_pos = out.len();
    let entries = [
        XEntry::Compressed(8, 0),
        XEntry::Compressed(8, 1),
        XEntry::Compressed(8, 2),
        XEntry::Uncompressed(off(8), 0),
    ];
    let rows = xref_rows(&entries, [1, 2, 1]);
    out.extend_from_slice(b"9 0 obj\n");
    out.extend_from_slice(&stream_body("/Type /XRef /Size 10 /W [1 2 1] /Index [5 4]", &rows));
    out.extend_from_slice(b"\nendobj\n");
    let xref_pos = out.len();
    let mut t = String::from("xref\n0 5\n0000000000 65535 f \n");
    for n in 1..=4 {
        t.push_str(&format!("{:010} 00000 n \n", off(n)));
    }
    out.extend_from_slice(t.as_bytes());
    out.extend_from_slice(
        format!("trailer\n<< /Size 10 /Root 1 0 R /XRefStm {} >>\nstartxref\n{}\n%%EOF\n", xs_pos, xref_pos).as_bytes(),
    );
    let doc = PdfDoc::from_bytes(out.clone());
    assert!(doc.is_ok());
    assert_eq!(piece_info(&doc).dict_lookup("A").get_string(), b"five from objstm");
    oracle("hybrid", &out);
}

#[test]
fn incremental_update_prev_chain() {
    let base = classic_pdf(&basic_objs("<< /Version (old) >>"), " /Info 5 0 R");
    // the base has no object 5; the update adds it and replaces 3
    let start_old = {
        let s = String::from_utf8_lossy(&base).into_owned();
        let i = s.rfind("startxref\n").unwrap();
        s[i + 10..].lines().next().unwrap().trim().parse::<usize>().unwrap()
    };
    let mut out = base.clone();
    let off3 = out.len();
    out.extend_from_slice(format!("3 0 obj\n{}\nendobj\n", page_with("/PieceInfo << /Version (new) >>")).as_bytes());
    let off5 = out.len();
    out.extend_from_slice(b"5 0 obj\n<< /Title (updated) >>\nendobj\n");
    let xref_pos = out.len();
    out.extend_from_slice(
        format!(
            "xref\n3 1\n{:010} 00000 n \n5 1\n{:010} 00000 n \ntrailer\n<< /Size 6 /Root 1 0 R /Info 5 0 R /Prev {} >>\nstartxref\n{}\n%%EOF\n",
            off3, off5, start_old, xref_pos
        )
        .as_bytes(),
    );
    let doc = PdfDoc::from_bytes(out.clone());
    assert!(doc.is_ok());
    assert_eq!(piece_info(&doc).dict_lookup("Version").get_string(), b"new");
    assert_eq!(doc.get_xref().unwrap().get_num_xref_tables(), 2);
    oracle("incremental", &out);
}

#[test]
fn broken_xref_offsets_are_repaired() {
    let pdf = classic_pdf(&basic_objs("<< /K (v) >>"), "");
    // shift every offset in the table by +3
    let s = String::from_utf8_lossy(&pdf).into_owned();
    let xi = s.find("xref\n").unwrap();
    let mut t = s[..xi].to_string();
    for line in s[xi..].split_inclusive('\n') {
        if line.len() == 20 && line.ends_with(" n \n") {
            let off: usize = line[..10].parse().unwrap();
            t.push_str(&format!("{:010}{}", off + 3, &line[10..]));
        } else {
            t.push_str(line);
        }
    }
    let pdf = t.into_bytes();
    let doc = PdfDoc::from_bytes(pdf.clone());
    assert!(doc.is_ok());
    assert!(doc.get_xref().unwrap().is_repaired());
    assert_eq!(piece_info(&doc).dict_lookup("K").get_string(), b"v");
    oracle("broken offsets", &pdf);
}

#[test]
fn missing_xref_is_reconstructed() {
    let (mut pdf, _) = write_objects(b"%PDF-1.4\n", &basic_objs("<< /K 42 >>"));
    pdf.extend_from_slice(b"trailer\n<< /Root 1 0 R >>\n%%EOF\n");
    let doc = PdfDoc::from_bytes(pdf.clone());
    assert!(doc.is_ok());
    assert!(doc.get_xref().unwrap().is_repaired());
    assert_eq!(piece_info(&doc).dict_lookup("K").get_int(), 42);
    oracle("no xref", &pdf);
}

#[test]
fn garbage_before_header() {
    let mut header = b"GARBAGE GARBAGE GARBAGE\n".to_vec();
    let base = header.len();
    header.extend_from_slice(b"%PDF-1.3\n");
    let pdf = classic_pdf_with_header(&header, &basic_objs("<< /K /ok >>"), "", base);
    let doc = PdfDoc::from_bytes(pdf.clone());
    assert!(doc.is_ok());
    assert!(!doc.get_xref().unwrap().is_repaired());
    assert_eq!(doc.get_pdf_version(), 1.3);
    assert!(piece_info(&doc).dict_lookup("K").is_name_of("ok"));
    oracle("garbage header", &pdf);
}

// ------------------------------------------------------------------------
// stream lengths
// ------------------------------------------------------------------------

fn length_variant(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut body = format!("<<{}>>\nstream\n", dict).into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    // (pdfTeX asserts that a page content stream has /Length, so the test
    // stream is reached through /PieceInfo instead)
    let objs = vec![
        obj(1, CATALOG),
        obj(2, PAGES),
        obj(3, &page_with("/PieceInfo << /S 6 0 R >>")),
        obj_bytes(4, 0, stream_body("", CONTENT)),
        obj(5, &data.len().to_string()),
        obj_bytes(6, 0, body),
    ];
    classic_pdf(&objs, "")
}

#[test]
fn stream_lengths() {
    // (no 'e' in the data: see stream_without_length_endstream_search_quirk)
    let data = b"0 0 m 10 10 l S % padding padding padding padding";
    let cases: Vec<(&str, String)> = vec![
        ("exact", format!(" /Length {}", data.len())),
        ("indirect", " /Length 5 0 R".to_string()),
        ("short", format!(" /Length {}", data.len() - 10)),
        ("long", format!(" /Length {}", data.len() + 7)),
        ("missing", String::new()),
        ("negative", " /Length -3".to_string()),
        ("real", " /Length 12.0".to_string()),
    ];
    for (name, dict) in cases {
        let pdf = length_variant(&dict, data);
        let doc = PdfDoc::from_bytes(pdf.clone());
        assert!(doc.is_ok(), "{}", name);
        let c = piece_info(&doc).dict_lookup("S");
        let s = c.get_stream().expect(name);
        let raw = s.get_undecoded_data();
        match name {
            "exact" | "indirect" => assert_eq!(raw, data),
            "missing" | "real" => assert_eq!(raw, [&data[..], b"\n"].concat()), // up to 'endstream'
            "short" | "long" | "negative" => assert!(raw.len() > data.len()), // +5000 kludge
            _ => {}
        }
        oracle(&format!("length {}", name), &pdf);
    }
}

#[test]
fn stream_without_length_endstream_search_quirk() {
    // Parser::makeStream reads 8 bytes after every 'e' while searching for
    // "endstream" without rewinding, so an 'e' just before the real
    // "endstream" hides it: the stream becomes an error object.
    let pdf = length_variant("", b"0 0 m 1 1 l S % size");
    let doc = PdfDoc::from_bytes(pdf.clone());
    assert!(doc.is_ok());
    assert!(piece_info(&doc).dict_lookup("S").is_error());
    // pdfTeX would fail to copy the error object, so only pdftosrc is used
    oracle_opts("endstream quirk", &pdf, false);
}

// ------------------------------------------------------------------------
// filters (content stream arrays are decoded by pdfTeX)
// ------------------------------------------------------------------------

fn lzw_encode_literal(data: &[u8]) -> Vec<u8> {
    // 9-bit codes only: emit a clear code often enough that the table
    // never needs 10 bits
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut n = 0;
    let mut put = |code: u32, out: &mut Vec<u8>| {
        acc = (acc << 9) | code;
        n += 9;
        while n >= 8 {
            out.push((acc >> (n - 8)) as u8);
            n -= 8;
        }
    };
    put(256, &mut out);
    for (i, &b) in data.iter().enumerate() {
        if i > 0 && i % 200 == 0 {
            put(256, &mut out);
        }
        put(b as u32, &mut out);
    }
    put(257, &mut out);
    if n > 0 {
        out.push((acc << (8 - n)) as u8);
    }
    out
}

fn ascii85_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in data.chunks(4) {
        let mut b = [0u8; 4];
        b[..chunk.len()].copy_from_slice(chunk);
        let mut v = u32::from_be_bytes(b) as u64;
        if chunk.len() == 4 && v == 0 {
            out.push(b'z');
            continue;
        }
        let mut c = [0u8; 5];
        for k in (0..5).rev() {
            c[k] = (v % 85) as u8 + 0x21;
            v /= 85;
        }
        out.extend_from_slice(&c[..chunk.len() + 1]);
    }
    out.extend_from_slice(b"~>");
    out
}

fn run_length_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in data.chunks(100) {
        out.push((chunk.len() - 1) as u8);
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&[0xfd, b' ']); // run of 4 spaces
    out.push(0x80);
    out
}

#[test]
fn content_array_filters() {
    let c: &[u8] = b"q 1 0 0 1 0 0 cm 0 0 m 100 100 l S Q % ends without newline";
    let hex: String = c.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ") + ">";
    let streams: Vec<Vec<u8>> = vec![
        stream_body("/Filter /FlateDecode", &zlib_stored(c)),
        stream_body("/Filter /Fl", &zlib_fixed(b"aaaaaaaaaabbbbbbbbbbbbbbbbbbcdcdcdcd 0 0 m\n")),
        stream_body("/Filter /ASCIIHexDecode", hex.as_bytes()),
        stream_body("/Filter /A85", &ascii85_encode(c)),
        stream_body("/Filter /LZWDecode", &lzw_encode_literal(c)),
        stream_body("/Filter /LZWDecode /DecodeParms << /EarlyChange 0 >>", &lzw_encode_literal(c)),
        stream_body("/Filter /RunLengthDecode", &run_length_encode(c)),
        stream_body("/Filter [/ASCIIHexDecode /FlateDecode]", {
            let z = zlib_fixed(c);
            let h: String = z.iter().map(|b| format!("{:02x}", b)).collect::<String>() + ">";
            Box::leak(h.into_bytes().into_boxed_slice())
        }),
        stream_body(
            "/Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 7 >>",
            &zlib_stored(&png_up(b"0 0 m 1 1 l S Q q 2 2", 7)),
        ),
        stream_body("/Filter /FlateDecode /DecodeParms << /Predictor 2 /Columns 5 /Colors 1 >>", &zlib_stored(c)),
        stream_body("/Filter /UnknownFilter", c),
        stream_body("", b""),
        stream_body("/Filter /FlateDecode", b"\x78\x9c\x00garbage"),
        // partial final ASCII85 groups
        stream_body("/Filter /ASCII85Decode", &ascii85_encode(b"0 0 m 1 1 l S\n%x")),
        stream_body("/Filter /ASCII85Decode", &ascii85_encode(b"0 0 m 1 1 l S\n%xy")),
        stream_body("/Filter /ASCII85Decode", &ascii85_encode(b"0 0 m 1 1 l S\n%xyz")),
        // truncated last predictor row keeps the stale tail
        stream_body(
            "/Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 7 >>",
            &zlib_stored(&png_up(b"0 0 m 1 1 l S Q q 2 2 m\n", 7)),
        ),
    ];
    let mut objs = vec![obj(1, CATALOG), obj(2, PAGES)];
    let mut refs = String::new();
    for (i, s) in streams.iter().enumerate() {
        let n = 10 + i as i32;
        objs.push(obj_bytes(n, 0, s.clone()));
        refs.push_str(&format!(" {} 0 R", n));
    }
    objs.push(obj(
        3,
        &format!("<< /Type /Page /Parent 2 0 R /Resources << /ProcSet [/PDF] >> /Contents [{}] >>", refs),
    ));
    let pdf = classic_pdf(&objs, "");
    let doc = PdfDoc::from_bytes(pdf.clone());
    assert!(doc.is_ok());
    let contents = doc.get_catalog().unwrap().get_page(1).unwrap().get_contents();
    let dec = |i: i32| contents.array_get(i).get_stream().unwrap().get_decoded_data().unwrap();
    assert_eq!(dec(0), c);
    assert_eq!(dec(1), b"aaaaaaaaaabbbbbbbbbbbbbbbbbbcdcdcdcd 0 0 m\n");
    assert_eq!(dec(2), c);
    if c.len() % 4 == 0 {
        assert_eq!(dec(3), c);
    }
    assert_eq!(dec(4), c);
    assert_eq!(dec(6), [c, b"    "].concat());
    assert_eq!(dec(7), c);
    assert_eq!(dec(8), b"0 0 m 1 1 l S Q q 2 2");
    assert_eq!(dec(10), b""); // EOFStream
    let d = dec(16);
    assert_eq!(d.len(), 28); // 3 full rows + a row with 3 new and 4 stale bytes
    assert_eq!(&d[..24], b"0 0 m 1 1 l S Q q 2 2 m\n");
    oracle("content filters", &pdf);
}

// ------------------------------------------------------------------------
// page tree inheritance and resource merging
// ------------------------------------------------------------------------

#[test]
fn page_tree_inheritance() {
    let objs = vec![
        obj(1, CATALOG),
        obj(
            2,
            "<< /Type /Pages /Kids [10 0 R 3 0 R] /Count 3 /MediaBox [0 0 300 400] /Rotate 90 \
             /Resources << /ProcSet [/PDF /Text] /ExtGState << /G1 << /CA 0.5 >> /G2 << /ca 1 >> >> \
             /Font << /F1 20 0 R >> >> >>",
        ),
        obj(
            10,
            "<< /Type /Pages /Parent 2 0 R /Kids [11 0 R] /Count 1 /CropBox [10 10 290 390] /Rotate -90 \
             /Resources << /ExtGState << /G2 << /ca 0.25 >> /G3 << /LW 2 >> >> /XObject << >> >> >>",
        ),
        obj(
            11,
            "<< /Type /Page /Parent 10 0 R /Contents 4 0 R /MediaBox [300 400 0 0] \
             /Resources << /Font << /F2 21 0 R /F1 22 0 R >> /Pattern << >> >> >>",
        ),
        obj(3, "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Rotate 450 >>"),
        obj_bytes(4, 0, stream_body("", CONTENT)),
        obj(20, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"),
        obj(21, "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>"),
        obj(22, "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>"),
    ];
    let pdf = classic_pdf(&objs, "");
    let doc = PdfDoc::from_bytes(pdf.clone());
    assert!(doc.is_ok());
    let cat = doc.get_catalog().unwrap();
    assert_eq!(cat.get_num_pages(), 3);
    let p1 = cat.get_page(1).unwrap();
    assert_eq!(*p1.get_media_box(), pdfread::PDFRectangle::new(0.0, 0.0, 300.0, 400.0));
    assert_eq!(*p1.get_crop_box(), pdfread::PDFRectangle::new(10.0, 10.0, 290.0, 390.0));
    assert_eq!(p1.get_rotate(), 270);
    let res = p1.get_resource_dict().unwrap();
    let keys: Vec<String> = (0..res.get_length()).map(|i| String::from_utf8_lossy(&res.get_key(i)).into()).collect();
    // ProcSet (an array) is dropped by the merge; parent keys come first
    assert_eq!(keys, ["ExtGState", "Font", "XObject", "Pattern"]);
    let font = res.lookup("Font");
    let fkeys: Vec<String> =
        (0..font.dict_get_length()).map(|i| String::from_utf8_lossy(&font.dict_get_key(i)).into()).collect();
    assert_eq!(fkeys, ["F1", "F2"]);
    assert_eq!(font.dict_lookup_nf("F1").get_ref_num(), 22);
    let gs = res.lookup("ExtGState");
    assert_eq!(gs.dict_lookup("G2").dict_lookup("ca").get_real(), 0.25);
    assert_eq!(cat.get_page_ref(1).unwrap().num, 11);
    // page 2 is 3 0 R: Rotate 450 -> 90, inherits root resources as is
    let p2 = cat.get_page(2).unwrap();
    assert_eq!(p2.get_rotate(), 90);
    assert!(p2.get_resource_dict().unwrap().lookup("ProcSet").is_array());
    // page 3 does not exist in the tree (Count says 3): dummy page
    let p3 = cat.get_page(3).unwrap();
    assert_eq!(*p3.get_media_box(), pdfread::PDFRectangle::new(0.0, 0.0, 50.0, 50.0));
    assert_eq!(cat.get_page_ref(3).unwrap(), pdfread::Ref::new(-1, -1));
    assert_eq!(cat.find_page(3, 0), 2);
    oracle("inheritance", &pdf);
}

#[test]
fn named_destinations() {
    let objs = vec![
        obj(
            1,
            "<< /Type /Catalog /Pages 2 0 R /Dests << /d1 [3 0 R /Fit] >> \
             /Names << /Dests 6 0 R >> >>",
        ),
        obj(2, "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 /MediaBox [0 0 200 100] >>"),
        obj(3, &page_with("")),
        obj_bytes(4, 0, stream_body("", CONTENT)),
        obj(5, "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"),
        obj(6, "<< /Kids [7 0 R 8 0 R] >>"),
        obj(7, "<< /Limits [(a) (m)] /Names [(alpha) [5 0 R /XYZ 1 2 null] (beta) << /D [3 0 R /FitH 5] >>] >>"),
        obj(8, "<< /Limits [(n) (zzz)] /Names [(zeta) [9 0 R /Fit]] >>"),
    ];
    let pdf = classic_pdf(&objs, "");
    let doc = PdfDoc::from_bytes(pdf.clone());
    assert!(doc.is_ok());
    let d = doc.find_dest(b"d1").unwrap();
    assert_eq!(d.get_page_ref(), pdfread::Ref::new(3, 0));
    let d = doc.find_dest(b"alpha").unwrap();
    assert_eq!(doc.get_catalog().unwrap().find_page(d.get_page_ref().num, d.get_page_ref().gen), 2);
    assert!(d.get_change_left() && !d.get_change_zoom());
    let d = doc.find_dest(b"beta").unwrap();
    assert_eq!(d.get_kind(), pdfread::LinkDestKind::FitH);
    assert!(doc.find_dest(b"gamma").is_none());
    assert_eq!(doc.find_dest(b"zeta").unwrap().get_page_ref().num, 9);
    oracle("dests", &pdf);
}

#[test]
fn self_referential_length_terminates() {
    // /Length 6 0 R inside object 6 itself, and a 7 <-> 8 cycle: xpdf
    // recurses until objectRecursionLimit, then the inner dict parses as a
    // command, so the Length is invalid and 'endstream' is searched for
    let mut objs = basic_objs("<< /S 6 0 R /T 7 0 R >>");
    objs.push(obj_bytes(6, 0, b"<< /Length 6 0 R >>\nstream\n0 0 m S\nendstream".to_vec()));
    objs.push(obj_bytes(7, 0, b"<< /Length 8 0 R >>\nstream\nxx\nendstream".to_vec()));
    objs.push(obj_bytes(8, 0, b"<< /Length 7 0 R >>\nstream\nyy\nendstream".to_vec()));
    let pdf = classic_pdf(&objs, "");
    let doc = PdfDoc::from_bytes(pdf.clone());
    assert!(doc.is_ok());
    let s = piece_info(&doc).dict_lookup("S");
    assert_eq!(s.get_stream().unwrap().get_undecoded_data(), b"0 0 m S\n");
    let t = piece_info(&doc).dict_lookup("T");
    assert_eq!(t.get_stream().unwrap().get_undecoded_data(), b"xx\n");
    oracle_opts("self length", &pdf, false);
}
