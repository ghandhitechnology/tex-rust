//! Differential check through the real consumer: include page 1 of a
//! PDF with TeX Live's `pdftex.exe` (whose `pdftoepdf.cc` re-serializes
//! xpdf's in-memory objects), then verify that the objects pdfTeX wrote
//! are exactly what the objects *we* parsed from the source would give
//! under pdftoepdf's `copyObject()` rules.
//!
//! Fonts are copied verbatim (`\pdfinclusioncopyfonts=1`) and nothing is
//! compressed, so the output can be compared structurally.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;

use pdfread::{Lexer, Object, PdfDoc, Ref};

use super::TempDir;

/// Total number of objects compared (for reporting).
pub static COMPARED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Run pdftex on a file including page `page` of `pdf` and return the
/// produced PDF (or the log on failure).
pub fn pdftex_include(pdftex: &Path, pdf: &[u8], page: i32) -> Result<Vec<u8>, String> {
    pdftex_include_opts(pdftex, pdf, page, true)
}

/// Like [`pdftex_include`]; with `copy_fonts = false` pdfTeX replaces the
/// embedded Type 1 fonts it finds in its map (using xpdf's GfxFont).
pub fn pdftex_include_opts(pdftex: &Path, pdf: &[u8], page: i32, copy_fonts: bool) -> Result<Vec<u8>, String> {
    let tmp = TempDir::new("pdftex");
    std::fs::write(tmp.path().join("inc.pdf"), pdf).unwrap();
    let tex = format!(
        "\\catcode`\\{{=1 \\catcode`\\}}=2 \\pdfoutput=1 \\pdfcompresslevel=0 \\pdfobjcompresslevel=0 \
         \\pdfinclusioncopyfonts={} \\pdfinclusionerrorlevel=-1 \
         \\pdfximage page {} {{inc.pdf}}\\setbox0\\hbox{{\\pdfrefximage\\pdflastximage}}\\shipout\\box0 \\end\n",
        if copy_fonts { 1 } else { 0 },
        page
    );
    std::fs::write(tmp.path().join("job.tex"), tex).unwrap();
    let out = Command::new(pdftex)
        .current_dir(tmp.path())
        .args(["-ini", "-interaction=batchmode", "-halt-on-error", "job.tex"])
        .output()
        .map_err(|e| e.to_string())?;
    match std::fs::read(tmp.path().join("job.pdf")) {
        Ok(d) if out.status.success() => Ok(d),
        _ => {
            let log = std::fs::read_to_string(tmp.path().join("job.log")).unwrap_or_default();
            Err(format!("pdftex failed: {}", log))
        }
    }
}

/// `convertNumToPDF(double n)` from pdftoepdf.cc.
pub fn convert_num_to_pdf(n: f64) -> String {
    let epsilon = 0.5e-6;
    if n.abs() < epsilon {
        return "0".to_string();
    }
    let mut s = String::new();
    let mut n = n;
    if n < 0.0 {
        s.push('-');
        n = -n;
    }
    n += epsilon; // for rounding
    let ival = n.floor() as i32;
    n -= ival as f64;
    s.push_str(&ival.to_string());
    let mut fval = (n * 1e6).floor() as i32;
    if fval != 0 {
        let mut digits = [b'0'; 6];
        for k in (0..6).rev() {
            digits[k] = b'0' + (fval % 10) as u8;
            fval /= 10;
        }
        let mut end = 6;
        while end > 0 && digits[end - 1] == b'0' {
            end -= 1;
        }
        s.push('.');
        s.push_str(std::str::from_utf8(&digits[..end]).unwrap());
    }
    s
}

fn parse_num(text: &str) -> Object {
    let mut lx = Lexer::new(Rc::from(text.as_bytes()), 0, None);
    lx.get_obj()
}

pub struct Cmp<'a> {
    pub src: &'a PdfDoc,
    pub out: &'a PdfDoc,
    pub map: HashMap<Ref, Ref>,
    pub queue: Vec<(Ref, Ref)>,
    pub errors: Vec<String>,
}

impl<'a> Cmp<'a> {
    pub fn new(src: &'a PdfDoc, out: &'a PdfDoc) -> Self {
        Cmp { src, out, map: HashMap::new(), queue: Vec::new(), errors: Vec::new() }
    }

    fn err(&mut self, path: &str, msg: String) {
        self.errors.push(format!("{}: {}", path, msg));
    }

    /// `copyObject(obj)` in the source vs the object pdfTeX wrote.
    pub fn eq(&mut self, path: &str, s: &Object, o: &Object) {
        COMPARED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        match s {
            Object::Bool(b) => {
                if !matches!(o, Object::Bool(x) if x == b) {
                    self.err(path, format!("bool {:?} vs {:?}", s, o));
                }
            }
            Object::Int(i) => {
                if !matches!(o, Object::Int(x) if x == i) {
                    self.err(path, format!("int {:?} vs {:?}", s, o));
                }
            }
            Object::Real(r) => {
                let want = parse_num(&convert_num_to_pdf(*r));
                let same = match (&want, o) {
                    (Object::Int(a), Object::Int(b)) => a == b,
                    (Object::Real(a), Object::Real(b)) => a.to_bits() == b.to_bits(),
                    _ => false,
                };
                if !same {
                    self.err(path, format!("real {:?} (-> {}) vs {:?}", r, convert_num_to_pdf(*r), o));
                }
            }
            Object::String(a) => {
                if !matches!(o, Object::String(b) if a == b) {
                    self.err(path, format!("string {:?} vs {:?}", s, o));
                }
            }
            Object::Name(a) => {
                if !matches!(o, Object::Name(b) if a == b) {
                    self.err(path, format!("name {:?} vs {:?}", s, o));
                }
            }
            Object::Null => {
                if !o.is_null() {
                    self.err(path, format!("null vs {:?}", o));
                }
            }
            Object::Array(a) => match o {
                Object::Array(b) if a.get_length() == b.get_length() => {
                    for i in 0..a.get_length() {
                        self.eq(&format!("{}[{}]", path, i), &a.get_nf(i), &b.get_nf(i));
                    }
                }
                _ => self.err(path, format!("array {:?} vs {:?}", s, o)),
            },
            Object::Dict(a) => match o {
                Object::Dict(b) => self.eq_dict(path, a, b),
                _ => self.err(path, format!("dict {:?} vs {:?}", s, o)),
            },
            Object::Stream(a) => match o {
                Object::Stream(b) => {
                    self.eq_dict(path, a.get_dict(), b.get_dict());
                    self.eq_stream_bytes(path, &a.get_undecoded_data(), b);
                }
                _ => self.err(path, format!("stream vs {:?}", o)),
            },
            Object::Ref(r) => match o {
                Object::Ref(q) => match self.map.get(r) {
                    Some(m) if m == q => {}
                    Some(m) => {
                        let m = *m;
                        self.err(path, format!("ref {:?} mapped to {:?} and {:?}", r, m, q))
                    }
                    None => {
                        self.map.insert(*r, *q);
                        self.queue.push((*r, *q));
                    }
                },
                _ => self.err(path, format!("ref {:?} vs {:?}", r, o)),
            },
            _ => self.err(path, format!("uncopyable source object {:?}", s)),
        }
    }

    fn eq_dict(&mut self, path: &str, a: &pdfread::Dict, b: &pdfread::Dict) {
        let ka: Vec<Vec<u8>> = (0..a.get_length()).map(|i| a.get_key(i).to_vec()).collect();
        let kb: Vec<Vec<u8>> = (0..b.get_length()).map(|i| b.get_key(i).to_vec()).collect();
        if ka != kb {
            self.err(
                path,
                format!(
                    "dict keys {:?} vs {:?}",
                    ka.iter().map(|k| String::from_utf8_lossy(k).into_owned()).collect::<Vec<_>>(),
                    kb.iter().map(|k| String::from_utf8_lossy(k).into_owned()).collect::<Vec<_>>()
                ),
            );
            return;
        }
        for i in 0..a.get_length() {
            let p = format!("{}/{}", path, String::from_utf8_lossy(&a.get_key(i)));
            self.eq(&p, &a.get_val_nf(i), &b.get_val_nf(i));
        }
    }

    /// The bytes pdfTeX copied (`copyStream(getUndecodedStream())`) are
    /// followed by "\nendstream" in the output, whatever /Length says.
    fn eq_stream_bytes(&mut self, path: &str, want: &[u8], o: &pdfread::Stream) {
        let data = self.out.get_data();
        let st = o.get_start() as usize;
        let ok = data.len() >= st + want.len()
            && &data[st..st + want.len()] == want
            && data[st + want.len()..].starts_with(b"\nendstream");
        if !ok {
            self.err(path, format!("stream data differs ({} bytes expected)", want.len()));
        }
    }

    /// Process `writeRefs()`: every referenced object is fetched and
    /// copied.
    pub fn drain(&mut self) {
        while let Some((r, q)) = self.queue.pop() {
            let s = self.src.get_xref().unwrap().fetch(r.num, r.gen);
            let o = self.out.get_xref().unwrap().fetch(q.num, q.gen);
            self.eq(&format!("{} {} R", r.num, r.gen), &s, &o);
        }
    }
}

/// Compare page `page` of `src` with the form XObject pdfTeX produced in
/// `out` (mirrors `write_epdf()` with \pdfinclusioncopyfonts=1).
pub fn compare_inclusion(src: &PdfDoc, out: &PdfDoc, page: i32) -> Vec<String> {
    let mut c = Cmp::new(src, out);
    let ocat = out.get_catalog().unwrap();
    let opage = ocat.get_page(1).unwrap();
    let ores = opage.get_resource_dict().unwrap().clone();
    let xobjs = ores.lookup("XObject");
    let form = xobjs.dict_get_val(0);
    let form = match form.get_stream() {
        Some(f) => f.clone(),
        None => return vec!["no form xobject in pdftex output".into()],
    };
    let fd = form.get_dict().clone();

    let scat = src.get_catalog().unwrap();
    let spage = scat.get_page(page).unwrap();
    let sref = scat.get_page_ref(page).unwrap();
    let spage_obj = src.get_xref().unwrap().fetch(sref.num, sref.gen);
    let spd = spage_obj.get_dict().unwrap().clone();

    // /PTEX.InfoDict
    if let Object::Ref(r) = src.get_doc_info_nf() {
        c.eq("InfoDict", &Object::Ref(r), &fd.lookup_nf("PTEX.InfoDict"));
    }
    // copied page dictionary entries
    for key in ["LastModified", "Metadata", "PieceInfo", "SeparationInfo"] {
        let v = spd.lookup_nf(key);
        if !v.is_null() {
            c.eq(key, &v, &fd.lookup_nf(key));
        }
    }
    // resources
    if let Some(sres) = spage.get_resource_dict() {
        let ores = fd.lookup_nf("Resources");
        let ores = ores.get_dict().unwrap().clone();
        let mut oi = 0;
        for i in 0..sres.get_length() {
            let key = sres.get_key(i);
            let val = sres.get_val(i);
            let k = String::from_utf8_lossy(&key).into_owned();
            if &*key == b"Font" {
                let ofont = ores.get_val_nf(oi);
                oi += 1;
                let sd = val.get_dict().unwrap();
                let od = ofont.get_dict().unwrap();
                if sd.get_length() != od.get_length() {
                    c.errors.push("Font dict length differs".into());
                    continue;
                }
                for j in 0..sd.get_length() {
                    if sd.get_key(j) != od.get_key(j) {
                        c.errors.push("Font key differs".into());
                    }
                    c.eq(&format!("Font/{}", j), &sd.get_val_nf(j), &od.get_val_nf(j));
                }
            } else if &*key == b"ProcSet" {
                let o = ores.get_val_nf(oi);
                oi += 1;
                c.eq("ProcSet", &val, &o);
            } else {
                let skip = if &*key == b"Subtype" { !val.is_name() } else { !val.is_dict() };
                if skip {
                    continue;
                }
                if &*ores.get_key(oi) != &*key {
                    c.errors.push(format!("resource key {} vs {}", k, String::from_utf8_lossy(&ores.get_key(oi))));
                }
                let o = ores.get_val_nf(oi);
                oi += 1;
                c.eq(&format!("Resources/{}", k), &val, &o);
            }
        }
        if oi != ores.get_length() {
            c.errors.push(format!("{} resources expected, pdftex wrote {}", oi, ores.get_length()));
        }
    }
    // contents
    match spage.get_contents() {
        Object::Stream(s) => {
            c.eq("Contents/Length", &s.get_dict().lookup("Length"), &fd.lookup_nf("Length"));
            let f = s.get_dict().lookup("Filter");
            if !f.is_null() {
                c.eq("Contents/Filter", &f, &fd.lookup_nf("Filter"));
            }
            c.eq_stream_bytes("Contents", &s.get_undecoded_data(), &form);
        }
        Object::Array(a) => {
            let mut want = Vec::new();
            let mut last = 0u8;
            for i in 0..a.get_length() {
                let d = a.get(i).get_stream().unwrap().get_decoded_data().unwrap();
                last = d.last().copied().unwrap_or(0);
                want.extend_from_slice(&d);
                if i < a.get_length() - 1 && last != b'\n' {
                    want.push(b'\n');
                    last = b'\n';
                }
            }
            let _ = last;
            let got = form.get_undecoded_data();
            if got != want {
                c.errors.push(format!(
                    "Contents array: {} bytes expected, pdftex wrote {}",
                    want.len(),
                    got.len()
                ));
            }
        }
        _ => {}
    }
    c.drain();
    c.errors
}

/// Include page `page` of `pdf` with pdftex and compare.  Returns
/// `Ok(None)` when pdftex is unavailable.
pub fn check_with_pdftex(pdf: &[u8], page: i32) -> Result<Option<()>, String> {
    let pdftex = match super::tl_bin("pdftex") {
        Some(p) => p,
        None => return Ok(None),
    };
    let src = PdfDoc::from_bytes(pdf.to_vec());
    let out = pdftex_include(&pdftex, pdf, page);
    let out = match (src.is_ok(), out) {
        (true, Ok(o)) => o,
        (false, Err(_)) => return Ok(Some(())), // both reject the file
        (true, Err(e)) => return Err(format!("we open the file, pdftex fails: {}", e)),
        (false, Ok(_)) => return Err("pdftex opens the file, we don't".into()),
    };
    let out = PdfDoc::from_bytes(out);
    if !out.is_ok() {
        return Err("cannot parse pdftex output".into());
    }
    let errs = compare_inclusion(&src, &out, page);
    if errs.is_empty() {
        Ok(Some(()))
    } else {
        Err(errs.join("\n"))
    }
}

/// Include page 1 with font replacement enabled and compare the encoding
/// pdfTeX wrote for every replaced font (from xpdf's
/// `Gfx8BitFont::getCharName`) with ours.  Returns the number of fonts
/// compared.
pub fn check_font_encodings(pdf: &[u8]) -> Result<usize, String> {
    let pdftex = match super::tl_bin("pdftex") {
        Some(p) => p,
        None => return Ok(0),
    };
    let src = PdfDoc::from_bytes(pdf.to_vec());
    if !src.is_ok() {
        return Ok(0);
    }
    let out = match pdftex_include_opts(&pdftex, pdf, 1, false) {
        Ok(o) => o,
        Err(_) => return Ok(0), // e.g. CID fonts: pdfTeX fails, nothing to compare
    };
    let out = PdfDoc::from_bytes(out);
    let ores = out.get_catalog().unwrap().get_page(1).unwrap().get_resource_dict().unwrap().clone();
    let form = ores.lookup("XObject").dict_get_val(0);
    let fd = form.stream_get_dict().unwrap().clone();
    let ofonts = fd.lookup("Resources").dict_lookup("Font");
    let spage = src.get_catalog().unwrap().get_page(1).unwrap();
    let sfonts = match spage.get_resource_dict() {
        Some(r) => r.lookup("Font"),
        None => return Ok(0),
    };
    let mut n = 0;
    let mut errs = Vec::new();
    for i in 0..ofonts.dict_get_length() {
        let tag = ofonts.dict_get_key(i);
        let ofont = ofonts.dict_get_val(i);
        if !ofont.dict_lookup_nf("BaseFont").is_ref() {
            continue; // not replaced
        }
        let enc = ofont.dict_lookup("Encoding");
        let diffs = enc.dict_lookup("Differences");
        let mut theirs: Vec<Option<Vec<u8>>> = vec![None; 256];
        let mut code = 0i32;
        for j in 0..diffs.array_get_length() {
            match diffs.array_get(j) {
                Object::Int(c) => code = c,
                Object::Name(nm) => {
                    if (0..256).contains(&code) {
                        theirs[code as usize] = Some(nm.to_vec());
                    }
                    code += 1;
                }
                _ => {}
            }
        }
        let sref = sfonts.dict_lookup_nf(&*tag).get_ref();
        let sdict = src.get_xref().unwrap().fetch(sref.num, sref.gen);
        let gf = pdfread::GfxFont::make_font(src.get_xref().unwrap(), &tag, sref, sdict.get_dict().unwrap());
        let ours: Vec<Option<Vec<u8>>> = (0..256).map(|c| gf.get_char_name(c).map(|s| s.to_vec())).collect();
        if ours != theirs {
            let diff: Vec<String> = (0..256)
                .filter(|&c| ours[c] != theirs[c])
                .take(8)
                .map(|c| {
                    format!(
                        "{}: ours {:?} xpdf {:?}",
                        c,
                        ours[c].as_ref().map(|v| String::from_utf8_lossy(v).into_owned()),
                        theirs[c].as_ref().map(|v| String::from_utf8_lossy(v).into_owned())
                    )
                })
                .collect();
            errs.push(format!("font {} ({:?}): {}", String::from_utf8_lossy(&tag), gf.get_type(), diff.join("; ")));
        }
        n += 1;
    }
    if errs.is_empty() {
        Ok(n)
    } else {
        Err(errs.join("
"))
    }
}
