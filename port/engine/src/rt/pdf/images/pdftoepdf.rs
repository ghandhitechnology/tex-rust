//! Port of pdftexdir/pdftoepdf.cc: include a page of a PDF file as a Form XObject,
//! copying its resources and every object they reference (through the pdfread crate,
//! a port of the xpdf 4.06 subset pdfTeX uses).

use std::rc::Rc;

use pdfread::{Dict, GfxFont, Object, PDFRectangle, Page, PdfDoc, Ref, XRef};

use crate::gen::globals::Globals;
use crate::rt::*;

const PDF_KEY_PREFIX: &str = "PTEX";
const MASK_SUPPRESS_PTEX_FILENAME: i32 = 0x02;
const MASK_SUPPRESS_PTEX_PAGENUMBER: i32 = 0x04;
const MASK_SUPPRESS_PTEX_INFODICT: i32 = 0x08;

#[derive(Clone, Copy, PartialEq)]
enum InObjType {
    Font,
    FontDesc,
    Other,
}

/// struct InObj (the linked list is a Vec in list order)
struct InObj {
    r: Ref,
    typ: InObjType,
    num: i32,
    fd: Option<usize>,
    enc_objnum: i32,
    written: bool,
}

/// struct UsedEncoding
struct UsedEncoding {
    enc_objnum: i32,
    font: GfxFont,
}

/// struct PdfDocument
struct PdfDocument {
    id: usize,
    file_name: Vec<u8>,
    doc: PdfDoc,
    xref: Option<Rc<XRef>>,
    in_obj_list: Vec<InObj>,
    occurences: i32,
}

/// pdftoepdf.cc static state
#[derive(Default)]
pub struct EpdfState {
    /// pdfDocuments, head first
    docs: Vec<PdfDocument>,
    next_id: usize,
    is_init: bool,
    /// the static inObjList / encodingList / xref while a page is written
    in_obj_list: Vec<InObj>,
    encoding_list: Vec<UsedEncoding>,
    xref: Option<Rc<XRef>>,
}

/// pdftoepdf.cc convertNumToPDF
fn convert_num_to_pdf(mut n: f64) -> Vec<u8> {
    const PRECISION: usize = 6;
    const FACT: f64 = 1E6;
    const EPSILON: f64 = 0.5E-6;
    let mut buf: Vec<u8> = Vec::new();
    if n.abs() < EPSILON {
        buf.push(b'0');
    } else {
        if n < 0.0 {
            buf.push(b'-');
            n = -n;
        }
        n += EPSILON;
        let ival = c_double_to_int(n.floor());
        n -= ival as f64;
        buf.extend_from_slice(format!("{}", ival).as_bytes());
        let mut fval = c_double_to_int((n * FACT).floor());
        if fval != 0 {
            buf.push(b'.');
            let bindex = buf.len();
            let mut digits = vec![0u8; PRECISION];
            let mut sindex = PRECISION as i32 - 1;
            while fval % 10 == 0 && sindex >= 0 {
                digits[sindex as usize] = 0;
                sindex -= 1;
                fval /= 10;
            }
            let end = (sindex + 1) as usize;
            while sindex >= 0 {
                digits[sindex as usize] = (fval % 10) as u8 + b'0';
                sindex -= 1;
                fval /= 10;
            }
            let _ = bindex;
            buf.extend_from_slice(&digits[..end]);
        }
    }
    buf
}

/// (int)d as compiled by MSVC on x64 (cvttsd2si): out-of-range values give INT_MIN.
fn c_double_to_int(d: f64) -> i32 {
    if d >= -2147483648.0 && d < 2147483648.0 {
        d as i32
    } else {
        i32::MIN
    }
}

/// utils.c stripzeros
pub fn stripzeros(a: &[u8]) -> Vec<u8> {
    #[derive(Clone, Copy, PartialEq)]
    enum S {
        NoNum,
        DotNoNum,
        Int,
        Dot,
        LeadDot,
        Frac,
    }
    let mut v = a.to_vec();
    let (mut s, mut t) = (S::NoNum, S::NoNum);
    let (mut p, mut q, mut r) = (0usize, 0usize, 0usize);
    while p < v.len() {
        let c = v[p];
        match s {
            S::NoNum => {
                if c.is_ascii_digit() {
                    s = S::Int;
                } else if c == b'.' {
                    s = S::LeadDot;
                }
            }
            S::DotNoNum => {
                if c != b'.' && !c.is_ascii_digit() {
                    s = S::NoNum;
                }
            }
            S::Int => {
                if c == b'.' {
                    s = S::Dot;
                } else if !c.is_ascii_digit() {
                    s = S::NoNum;
                }
            }
            S::Dot | S::LeadDot => {
                if c.is_ascii_digit() {
                    s = S::Frac;
                } else if c == b'.' {
                    s = S::DotNoNum;
                } else {
                    s = S::NoNum;
                }
            }
            S::Frac => {
                if c == b'.' {
                    s = S::DotNoNum;
                } else if !c.is_ascii_digit() {
                    s = S::NoNum;
                }
            }
        }
        match s {
            S::Dot => r = q,
            S::LeadDot => r = q + 1,
            S::Frac => {
                if c > b'0' {
                    r = q + 1;
                }
            }
            S::NoNum => {
                if (t == S::Frac || t == S::Dot) && r != 0 {
                    q = r;
                    r -= 1;
                    if v[r] == b'.' {
                        v[r] = b'0';
                    }
                    r = 0;
                }
            }
            _ => {}
        }
        v[q] = v[p];
        q += 1;
        p += 1;
        t = s;
    }
    v.truncate(q);
    v
}

/// C's %.Nf on a double.
fn fmt_f(x: f64, prec: usize) -> String {
    format!("{:.*}", prec, x)
}

/// pdftoepdf.cc initDictFromDict: a copy of the dict's entries (unresolved), bound to xref.
fn init_dict_from_dict(xref: &Option<Rc<XRef>>, dict: &Dict) -> Object {
    let mut d = Dict::new(xref.as_deref());
    for i in 0..dict.get_length() {
        d.add(&dict.get_key(i), dict.get_val_nf(i));
    }
    Object::Dict(Rc::new(d))
}

fn is_c_alnum_name_char(c: u8) -> bool {
    c.is_ascii_digit() || c.is_ascii_uppercase() || c.is_ascii_lowercase() || c == b'_' || c == b'.' || c == b'-' || c == b'+'
}

impl Globals {
    fn epdf_doc_index(&self, id: usize) -> Option<usize> {
        self.rt.images.pdf.docs.iter().position(|d| d.id == id)
    }

    fn epdf_xref(&self) -> Option<Rc<XRef>> {
        self.rt.images.pdf.xref.clone()
    }

    /// pdftoepdf.cc find_add_document
    fn find_add_document(&mut self, file_name: &[u8]) -> usize {
        let st = &mut self.rt.images.pdf;
        if let Some(p) = st.docs.iter_mut().find(|d| d.file_name == file_name) {
            st.xref = p.xref.clone();
            p.occurences += 1;
            return p.id;
        }
        st.xref = None;
        let path = self.os_path(file_name);
        let doc = PdfDoc::open(path);
        if !doc.is_ok() || !doc.ok_to_print(false) {
            self.pdftex_fail("xpdf: reading PDF image failed");
        }
        let st = &mut self.rt.images.pdf;
        st.next_id += 1;
        let id = st.next_id;
        st.docs.insert(
            0,
            PdfDocument { id, file_name: file_name.to_vec(), doc, xref: None, in_obj_list: Vec::new(), occurences: 0 },
        );
        id
    }

    /// pdftoepdf.cc delete_document
    fn delete_document(&mut self, id: usize) {
        let Some(i) = self.epdf_doc_index(id) else { return };
        let d = self.rt.images.pdf.docs.remove(i);
        self.rt.images.pdf.xref = d.xref.clone();
    }

    /// pdftoepdf.cc addInObj
    fn add_in_obj(&mut self, typ: InObjType, r: Ref, fd: Option<usize>, e: i32) -> i32 {
        if r.num == 0 {
            self.pdftex_fail("PDF inclusion: invalid reference");
        }
        if let Some(p) = self.rt.images.pdf.in_obj_list.iter().find(|p| p.r.num == r.num && p.r.gen == r.gen) {
            return p.num;
        }
        self.rt.images.pdf.in_obj_list.push(InObj { r, typ, num: 0, fd, enc_objnum: e, written: false });
        let num = if typ == InObjType::FontDesc { self.get_fd_objnum(fd.unwrap()) } else { self.pdfnewobjnum() };
        self.rt.images.pdf.in_obj_list.last_mut().unwrap().num = num;
        num
    }

    fn add_other(&mut self, r: Ref) -> i32 {
        self.add_in_obj(InObjType::Other, r, None, 0)
    }

    /// pdftoepdf.cc addEncoding
    fn add_encoding(&mut self, font: GfxFont) -> i32 {
        let enc_objnum = self.pdfnewobjnum();
        self.rt.images.pdf.encoding_list.insert(0, UsedEncoding { enc_objnum, font });
        enc_objnum
    }

    /// pdftoepdf.cc copyName
    fn copy_name(&mut self, s: &[u8]) {
        self.pdf_puts(b"/");
        for &c in s {
            if c == 0 {
                break;
            }
            if is_c_alnum_name_char(c) {
                self.pdfout(c);
            } else {
                self.pdf_printf(&format!("#{:02X}", c));
            }
        }
    }

    fn copy_dict_entry(&mut self, d: &Dict, i: i32) {
        let key = d.get_key(i);
        self.copy_name(&key);
        self.pdf_puts(b" ");
        let v = d.get_val_nf(i);
        self.copy_object(&v);
        self.pdf_puts(b"\n");
    }

    /// pdftoepdf.cc copyDict
    fn copy_dict(&mut self, obj: &Object) {
        let Some(d) = obj.get_dict() else {
            self.pdftex_fail(&format!("PDF inclusion: invalid dict type <{}>", obj.get_type_name()));
        };
        let d = d.clone();
        for i in 0..d.get_length() {
            self.copy_dict_entry(&d, i);
        }
    }

    /// pdftoepdf.cc copyFontDict
    fn copy_font_dict(&mut self, obj: &Object, fd: Option<usize>, enc_objnum: i32) {
        let Some(d) = obj.get_dict() else {
            self.pdftex_fail(&format!("PDF inclusion: invalid dict type <{}>", obj.get_type_name()));
        };
        let d = d.clone();
        self.pdf_puts(b"<<\n");
        for i in 0..d.get_length() {
            let key = d.get_key(i);
            if key.starts_with(b"FontDescriptor") || key.starts_with(b"BaseFont") || key.starts_with(b"Encoding") {
                continue;
            }
            self.copy_dict_entry(&d, i);
        }
        let fd = fd.unwrap();
        let a = self.get_fd_objnum(fd);
        self.pdf_printf(&format!("/FontDescriptor {} 0 R\n", a));
        let b = self.get_fn_objnum(fd);
        self.pdf_printf(&format!("/BaseFont {} 0 R\n", b));
        self.pdf_printf(&format!("/Encoding {} 0 R\n", enc_objnum));
        self.pdf_puts(b">>");
    }

    /// pdftoepdf.cc copyStream (over the bytes the xpdf stream delivers)
    fn copy_stream_bytes(&mut self, data: &[u8]) {
        self.pdfout_bytes(data);
        self.pdflastbyte = data.last().copied().unwrap_or(0);
    }

    /// pdftoepdf.cc copyProcSet
    fn copy_proc_set(&mut self, obj: &Object) {
        let Some(a) = obj.get_array() else {
            self.pdftex_fail(&format!("PDF inclusion: invalid ProcSet array type <{}>", obj.get_type_name()));
        };
        let a = a.clone();
        self.pdf_puts(b"/ProcSet [ ");
        for i in 0..a.get_length() {
            let p = a.get_nf(i);
            if !p.is_name() {
                self.pdftex_fail(&format!("PDF inclusion: invalid ProcSet entry type <{}>", p.get_type_name()));
            }
            self.copy_name(p.get_name());
            self.pdf_puts(b" ");
        }
        self.pdf_puts(b"]\n");
    }

    /// pdftoepdf.cc copyFont
    fn copy_font(&mut self, tag: &[u8], font_ref: &Object) {
        let r = font_ref.get_ref();
        if let Some(num) = self.rt.images.pdf.in_obj_list.iter().find(|p| p.r.num == r.num && p.r.gen == r.gen).map(|p| p.num) {
            self.copy_name(tag);
            self.pdf_printf(&format!(" {} 0 R ", num));
            return;
        }
        let xref = self.epdf_xref();
        if self.fixedinclusioncopyfont == 0 {
            let fontdict = font_ref.fetch(xref.as_deref());
            if let Some(fd_dict) = fontdict.get_dict() {
                let subtype = fd_dict.lookup("Subtype");
                let basefont = fd_dict.lookup("BaseFont");
                let fontdesc_ref = fd_dict.lookup_nf("FontDescriptor");
                if subtype.is_name() && subtype.get_name() == b"Type1" && basefont.is_name() && fontdesc_ref.is_ref() {
                    let fontdesc = fontdesc_ref.fetch(xref.as_deref());
                    if let Some(desc) = fontdesc.get_dict() {
                        let has_file = desc.lookup("FontFile").is_stream() || {
                            let ff3 = desc.lookup("FontFile3");
                            ff3.is_stream() && {
                                let st = ff3.stream_get_dict().map(|d| d.lookup("Subtype")).unwrap_or(Object::Null);
                                st.is_name() && st.get_name() == b"Type1C"
                            }
                        };
                        if has_file {
                            if let Some(fontmap) = self.epdf_lookup_fontmap(basefont.get_name()) {
                                let stemv = desc.lookup("StemV");
                                let fd = self.epdf_create_fontdescriptor(fontmap, zround(stemv.get_num()));
                                let charset = desc.lookup("CharSet");
                                if charset.is_string() && self.is_subsetable(fontmap) {
                                    let s = charset.get_string();
                                    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
                                    let cs = s[..end].to_vec();
                                    self.epdf_mark_glyphs(fd, &cs);
                                } else {
                                    self.embed_whole_font(fd);
                                }
                                self.add_in_obj(InObjType::FontDesc, fontdesc_ref.get_ref(), Some(fd), 0);
                                self.copy_name(tag);
                                let gfont = GfxFont::make_font(xref.as_deref().unwrap(), tag, font_ref.get_ref(), fd_dict);
                                let enc = self.add_encoding(gfont);
                                let n = self.add_in_obj(InObjType::Font, font_ref.get_ref(), Some(fd), enc);
                                self.pdf_printf(&format!(" {} 0 R ", n));
                                return;
                            }
                        }
                    }
                }
            }
        }
        self.copy_name(tag);
        self.pdf_puts(b" ");
        self.copy_object(font_ref);
    }

    /// pdftoepdf.cc copyFontResources
    fn copy_font_resources(&mut self, obj: &Object) {
        let Some(d) = obj.get_dict() else {
            self.pdftex_fail(&format!("PDF inclusion: invalid font resources dict type <{}>", obj.get_type_name()));
        };
        let d = d.clone();
        self.pdf_puts(b"/Font << ");
        for i in 0..d.get_length() {
            let font_ref = d.get_val_nf(i);
            let key = d.get_key(i);
            if font_ref.is_ref() {
                self.copy_font(&key, &font_ref);
            } else if font_ref.is_dict() {
                self.copy_name(&key);
                self.pdf_puts(b" ");
                self.copy_object(&font_ref);
            } else {
                self.pdftex_fail(&format!(
                    "PDF inclusion: invalid font in reference type <{}>",
                    font_ref.get_type_name()
                ));
            }
        }
        self.pdf_puts(b">>\n");
    }

    /// pdftoepdf.cc copyOtherResources
    fn copy_other_resources(&mut self, obj: &Object, key: &[u8]) {
        let k = String::from_utf8_lossy(key).into_owned();
        if key == b"Subtype" {
            if !obj.is_name() {
                self.pdftex_warn(&format!(
                    "PDF inclusion: Subtype in Resources dict is not a name (key '{}', type <{}>); ignored.",
                    k,
                    obj.get_type_name()
                ));
                return;
            }
        } else if !obj.is_dict() {
            self.pdftex_warn(&format!(
                "PDF inclusion: invalid other resource which is no dict (key '{}', type <{}>); ignored.",
                k,
                obj.get_type_name()
            ));
            return;
        }
        self.copy_name(key);
        self.pdf_puts(b" ");
        self.copy_object(obj);
    }

    /// pdftoepdf.cc copyObject
    fn copy_object(&mut self, obj: &Object) {
        match obj {
            Object::Bool(b) => self.pdf_printf(if *b { "true" } else { "false" }),
            Object::Int(i) => self.pdf_printf(&format!("{}", i)),
            Object::Real(r) => {
                let s = convert_num_to_pdf(*r);
                self.pdf_puts(&s);
            }
            Object::String(s) => {
                let l = s.len();
                let clen = s.iter().position(|&c| c == 0).unwrap_or(l);
                if clen == l {
                    self.pdf_puts(b"(");
                    for &c in s.iter() {
                        if c == b'(' || c == b')' || c == b'\\' {
                            self.pdf_printf(&format!("\\{}", c as char));
                        } else if c < 0x20 || c > 0x7F {
                            self.pdf_printf(&format!("\\{:03o}", c));
                        } else {
                            self.pdfout(c);
                        }
                    }
                    self.pdf_puts(b")");
                } else {
                    self.pdf_puts(b"<");
                    for &c in s.iter() {
                        self.pdf_printf(&format!("{:02x}", c));
                    }
                    self.pdf_puts(b">");
                }
            }
            Object::Name(n) => {
                let n = n.clone();
                self.copy_name(&n);
            }
            Object::Null => self.pdf_puts(b"null"),
            Object::Array(a) => {
                let a = a.clone();
                self.pdf_puts(b"[");
                for i in 0..a.get_length() {
                    let o = a.get_nf(i);
                    if !o.is_name() {
                        self.pdf_puts(b" ");
                    }
                    self.copy_object(&o);
                }
                self.pdf_puts(b"]");
            }
            Object::Dict(_) => {
                self.pdf_puts(b"<<\n");
                self.copy_dict(obj);
                self.pdf_puts(b">>");
            }
            Object::Stream(st) => {
                let st = st.clone();
                let xref = self.epdf_xref();
                let d = init_dict_from_dict(&xref, st.get_dict());
                self.pdf_puts(b"<<\n");
                self.copy_dict(&d);
                self.pdf_puts(b">>\n");
                self.pdf_puts(b"stream\n");
                let data = st.get_undecoded_data();
                self.copy_stream_bytes(&data);
                self.pdf_puts(b"\nendstream");
            }
            Object::Ref(r) => {
                if r.num == 0 {
                    self.pdftex_fail("PDF inclusion: reference to invalid object (is the included pdf broken?)");
                }
                let n = self.add_other(*r);
                self.pdf_printf(&format!("{} 0 R", n));
            }
            _ => self.pdftex_fail(&format!("PDF inclusion: type <{}> cannot be copied", obj.get_type_name())),
        }
    }

    /// pdftoepdf.cc writeRefs
    fn write_refs(&mut self) {
        let xref = self.epdf_xref();
        let mut i = 0;
        while i < self.rt.images.pdf.in_obj_list.len() {
            let (written, typ, r, num, fd, enc) = {
                let p = &self.rt.images.pdf.in_obj_list[i];
                (p.written, p.typ, p.r, p.num, p.fd, p.enc_objnum)
            };
            if !written {
                self.rt.images.pdf.in_obj_list[i].written = true;
                let obj1 = match &xref {
                    Some(x) => x.fetch(r.num, r.gen),
                    None => Object::Null,
                };
                if typ == InObjType::Font {
                    self.pdfbeginobj(num, 2);
                    self.copy_font_dict(&obj1, fd, enc);
                    self.pdf_puts(b"\n");
                    self.pdfendobj();
                } else if typ != InObjType::FontDesc {
                    if obj1.is_stream() {
                        self.pdfbeginobj(num, 0);
                    } else {
                        self.pdfbeginobj(num, 2);
                    }
                    self.copy_object(&obj1);
                    self.pdf_puts(b"\n");
                    self.pdfendobj();
                }
            }
            i += 1;
        }
    }

    /// pdftoepdf.cc writeEncodings
    fn write_encodings(&mut self) {
        let list = std::mem::take(&mut self.rt.images.pdf.encoding_list);
        for r in &list {
            let mut glyph_names: Vec<Option<Vec<u8>>> = vec![None; 256];
            for (i, g) in glyph_names.iter_mut().enumerate() {
                if r.font.is_cid_font() {
                    self.pdftex_fail(
                        "PDF inclusion: CID fonts are not supported (try to disable font replacement to fix this)",
                    );
                }
                *g = r.font.get_char_name(i as i32).map(|s| s.to_vec());
            }
            self.epdf_write_encoding(&glyph_names, r.enc_objnum);
        }
    }

    /// pdftoepdf.cc get_pagebox
    fn get_pagebox(&mut self, page: &Page, spec: i32) -> PDFRectangle {
        if spec == self.pdfboxspecmedia {
            *page.get_media_box()
        } else if spec == self.pdfboxspeccrop {
            *page.get_crop_box()
        } else if spec == self.pdfboxspecbleed {
            *page.get_bleed_box()
        } else if spec == self.pdfboxspectrim {
            *page.get_trim_box()
        } else if spec == self.pdfboxspecart {
            *page.get_art_box()
        } else {
            self.pdftex_fail(&format!("PDF inclusion: unknown value of pagebox spec ({})", spec));
        }
    }

    /// pdftoepdf.cc read_pdf_info
    #[allow(clippy::too_many_arguments)]
    pub fn read_pdf_info(
        &mut self,
        image_name: &[u8],
        page_name: Option<&[u8]>,
        mut page_num: i32,
        pagebox_spec: i32,
        major_pdf_version_wanted: i32,
        minor_pdf_version_wanted: i32,
        pdf_inclusion_errorlevel: i32,
    ) -> i32 {
        if !self.rt.images.pdf.is_init {
            pdfread::set_err_quiet(false);
            pdfread::set_error_callback(Some(Box::new(|cat, pos, msg| {
                use std::io::Write;
                let s = if pos >= 0 {
                    format!("\r\nlibxpdf: {} ({}): {}\r\n", cat.name(), pos as i32, msg)
                } else {
                    format!("\r\nlibxpdf: {}: {}\r\n", cat.name(), msg)
                };
                let _ = std::io::stderr().write_all(s.as_bytes());
            })));
            self.rt.images.pdf.is_init = true;
        }
        let id = self.find_add_document(image_name);
        self.rt.images.epdf_doc = id;
        let di = self.epdf_doc_index(id).unwrap();

        let pdf_version_found = self.rt.images.pdf.docs[di].doc.get_pdf_version() as f32;
        let pdf_version_wanted = (major_pdf_version_wanted as f64 + minor_pdf_version_wanted as f64 * 0.1) as f32;
        if pdf_version_found as f64 > pdf_version_wanted as f64 + 0.01 {
            let msg = format!(
                "PDF inclusion: found PDF version <{:.1}>, but at most version <{:.1}> allowed",
                pdf_version_found as f64, pdf_version_wanted as f64
            );
            if pdf_inclusion_errorlevel > 0 {
                self.pdftex_fail(&msg);
            } else if pdf_inclusion_errorlevel < 0 {
            } else {
                self.pdftex_warn(&msg);
            }
        }
        let cat_pages = self.rt.images.pdf.docs[di].doc.get_catalog().map(|c| c.get_num_pages()).unwrap_or(0);
        self.rt.images.epdf_num_pages = cat_pages;
        if let Some(name) = page_name {
            let pn = String::from_utf8_lossy(name).into_owned();
            let doc = &self.rt.images.pdf.docs[di].doc;
            let link = doc.find_dest(name);
            let r = match &link {
                Some(l) if l.is_ok() => Some(l.get_page_ref()),
                _ => None,
            };
            let Some(r) = r else {
                self.pdftex_fail(&format!("PDF inclusion: invalid destination <{}>", pn));
            };
            page_num = self.rt.images.pdf.docs[di].doc.get_catalog().map(|c| c.find_page(r.num, r.gen)).unwrap_or(0);
            if page_num == 0 {
                self.pdftex_fail(&format!("PDF inclusion: destination is not a page <{}>", pn));
            }
        } else if page_num <= 0 || page_num > cat_pages {
            self.pdftex_fail(&format!("PDF inclusion: required page does not exist <{}>", cat_pages));
        }
        let page = self.rt.images.pdf.docs[di].doc.get_catalog().and_then(|c| c.get_page(page_num)).expect("page");
        let pagebox = self.get_pagebox(&page, pagebox_spec);
        let e = &mut self.rt.images;
        if pagebox.x2 > pagebox.x1 {
            e.epdf_orig_x = pagebox.x1 as f32;
            e.epdf_width = (pagebox.x2 - pagebox.x1) as f32;
        } else {
            e.epdf_orig_x = pagebox.x2 as f32;
            e.epdf_width = (pagebox.x1 - pagebox.x2) as f32;
        }
        if pagebox.y2 > pagebox.y1 {
            e.epdf_orig_y = pagebox.y1 as f32;
            e.epdf_height = (pagebox.y2 - pagebox.y1) as f32;
        } else {
            e.epdf_orig_y = pagebox.y2 as f32;
            e.epdf_height = (pagebox.y1 - pagebox.y2) as f32;
        }
        let mut rot = (page.get_rotate() % 360) as f32;
        if rot < 0.0 {
            rot += 360.0;
        }
        e.epdf_rotate = rot;
        e.epdf_has_page_group = if page.get_group().is_some() { 1 } else { 0 };
        let x = self.rt.images.pdf.docs[di].doc.get_xref().cloned();
        self.rt.images.pdf.docs[di].xref = x;
        page_num
    }

    /// pdftoepdf.cc write_epdf
    pub fn write_epdf(&mut self) {
        let suppress_ptex_info = self.getpdfsuppressptexinfo();
        let sep = if self.getptexuseunderscore() { "_" } else { "." };
        let id = self.rt.images.epdf_doc;
        let di = self.epdf_doc_index(id).expect("PdfDocument");
        let sel = self.rt.images.epdf_selected_page;
        let (page, page_ref, file_name, info) = {
            let st = &mut self.rt.images.pdf;
            let d = &mut st.docs[di];
            d.occurences -= 1;
            st.xref = d.xref.clone();
            st.in_obj_list = std::mem::take(&mut d.in_obj_list);
            st.encoding_list = Vec::new();
            let cat = d.doc.get_catalog().expect("catalog");
            let page = cat.get_page(sel).expect("page");
            let page_ref = cat.get_page_ref(sel).unwrap_or_default();
            (page, page_ref, d.file_name.clone(), d.doc.get_doc_info_nf())
        };
        let xref = self.epdf_xref();
        let page_obj = match &xref {
            Some(x) => x.fetch(page_ref.num, page_ref.gen),
            None => Object::Null,
        };
        let page_dict: Rc<Dict> = page_obj.get_dict().cloned().unwrap_or_default();
        let rotate = page.get_rotate();
        self.pdf_puts(b"/Type /XObject\n");
        self.pdf_puts(b"/Subtype /Form\n");
        self.pdf_puts(b"/FormType 1\n");
        if suppress_ptex_info & MASK_SUPPRESS_PTEX_FILENAME == 0 {
            let fname = self.convert_string_to_pdf_string(&file_name);
            let mut s = format!("/{}{}FileName (", PDF_KEY_PREFIX, sep).into_bytes();
            s.extend_from_slice(&fname);
            s.extend_from_slice(b")\n");
            s.truncate(1023);
            self.pdf_puts(&s);
        }
        if suppress_ptex_info & MASK_SUPPRESS_PTEX_PAGENUMBER == 0 {
            self.pdf_printf(&format!("/{}{}PageNumber {}\n", PDF_KEY_PREFIX, sep, sel));
        }
        if suppress_ptex_info & MASK_SUPPRESS_PTEX_INFODICT == 0 && info.is_ref() {
            self.pdf_printf(&format!("/{}{}InfoDict ", PDF_KEY_PREFIX, sep));
            let n = self.add_other(info.get_ref());
            self.pdf_printf(&format!("{} 0 R\n", n));
        }
        let pagebox = self.get_pagebox(&page, self.rt.images.epdf_page_box);

        if rotate != 0 && rotate % 90 == 0 {
            self.tex_printf(&format!(", page is rotated {} degrees", rotate));
            let mut scale = [0f64; 6];
            let mut writematrix = false;
            match rotate {
                90 => {
                    scale[1] = -1.0;
                    scale[2] = 1.0;
                    scale[4] = pagebox.x1 - pagebox.y1;
                    scale[5] = pagebox.y1 + pagebox.x2;
                    writematrix = true;
                }
                180 => {
                    scale[0] = -1.0;
                    scale[3] = -1.0;
                    scale[4] = pagebox.x1 + pagebox.x2;
                    scale[5] = pagebox.y1 + pagebox.y2;
                    writematrix = true;
                }
                270 => {
                    scale[1] = 1.0;
                    scale[2] = -1.0;
                    scale[4] = pagebox.x1 + pagebox.y2;
                    scale[5] = pagebox.y1 - pagebox.x1;
                    writematrix = true;
                }
                _ => {}
            }
            if writematrix {
                let s = format!(
                    "/Matrix [{} {} {} {} {} {}]\n",
                    fmt_f(scale[0], 8),
                    fmt_f(scale[1], 8),
                    fmt_f(scale[2], 8),
                    fmt_f(scale[3], 8),
                    fmt_f(scale[4], 8),
                    fmt_f(scale[5], 8)
                );
                let s = stripzeros(s.as_bytes());
                self.pdf_puts(&s);
            }
        }
        let s = format!(
            "/BBox [{} {} {} {}]\n",
            fmt_f(pagebox.x1, 8),
            fmt_f(pagebox.y1, 8),
            fmt_f(pagebox.x2, 8),
            fmt_f(pagebox.y2, 8)
        );
        let s = stripzeros(s.as_bytes());
        self.pdf_puts(&s);

        let md = page_dict.lookup_nf("Metadata");
        if !md.is_null() && !md.is_ref() {
            self.pdftex_warn("PDF inclusion: /Metadata must be indirect object");
        }
        for key in ["LastModified", "Metadata", "PieceInfo", "SeparationInfo"] {
            let o = page_dict.lookup_nf(key);
            if !o.is_null() {
                self.pdf_newline();
                self.pdf_printf(&format!("/{} ", key));
                self.copy_object(&o);
            }
        }

        let mut group_dict: Option<Object> = None;
        let g = page_dict.lookup_nf("Group");
        if !g.is_null() {
            if self.pdfpagegroupval == 0 {
                if self.getpdfsuppresswarningpagegroup() == 0 {
                    self.pdftex_warn("PDF inclusion: multiple pdfs with page group included in a single page");
                }
                self.pdf_newline();
                self.pdf_puts(b"/Group ");
                self.copy_object(&g);
            } else {
                let g2 = page_dict.lookup("Group");
                if !g2.is_dict() {
                    self.pdftex_fail("PDF inclusion: /Group dict missing");
                }
                let gd = page.get_group().cloned().unwrap_or_default();
                group_dict = Some(init_dict_from_dict(&xref, &gd));
                self.pdf_printf(&format!("/Group {} 0 R\n", self.pdfpagegroupval));
            }
        }

        match page.get_resource_dict() {
            None => self.pdftex_warn(
                "PDF inclusion: /Resources missing. 'This practice is not recommended' (PDF Ref)",
            ),
            Some(rd) => {
                let obj1 = init_dict_from_dict(&xref, rd);
                let d = obj1.get_dict().unwrap().clone();
                self.pdf_newline();
                self.pdf_puts(b"/Resources <<\n");
                for i in 0..d.get_length() {
                    let obj2 = d.get_val(i);
                    let key = d.get_key(i);
                    if &key[..] == b"Font" {
                        self.copy_font_resources(&obj2);
                    } else if &key[..] == b"ProcSet" {
                        self.copy_proc_set(&obj2);
                    } else {
                        self.copy_other_resources(&obj2, &key);
                    }
                }
                self.pdf_puts(b">>\n");
            }
        }

        let contents = page.get_contents();
        if let Some(st) = contents.get_stream() {
            let sd = st.get_dict().clone();
            if !sd.lookup("F").is_null() {
                self.pdftex_fail("PDF inclusion: Unsupported external stream");
            }
            let len = sd.lookup("Length");
            self.pdf_puts(b"/Length ");
            self.copy_object(&len);
            self.pdf_puts(b"\n");
            let filter = sd.lookup("Filter");
            if !filter.is_null() {
                self.pdf_puts(b"/Filter ");
                self.copy_object(&filter);
                self.pdf_puts(b"\n");
                let dp = sd.lookup("DecodeParms");
                if !dp.is_null() {
                    self.pdf_puts(b"/DecodeParms ");
                    self.copy_object(&dp);
                    self.pdf_puts(b"\n");
                }
            }
            self.pdf_puts(b">>\nstream\n");
            let data = st.get_undecoded_data();
            self.copy_stream_bytes(&data);
            self.pdfendstream();
        } else if let Some(arr) = contents.get_array() {
            let arr = arr.clone();
            self.pdfbeginstream();
            let l = arr.get_length();
            for i in 0..l {
                let o = arr.get(i);
                let data = match o.get_stream() {
                    Some(s) => s.get_decoded_data().unwrap_or_default(),
                    None => Vec::new(),
                };
                self.copy_stream_bytes(&data);
                if i < l - 1 {
                    self.pdf_newline();
                }
            }
            self.pdfendstream();
        } else {
            self.pdfbeginstream();
            self.pdfendstream();
        }

        self.write_encodings();

        if let Some(gd) = group_dict {
            let g = self.pdfpagegroupval;
            self.pdfbeginobj(g, 2);
            self.copy_object(&gd);
            self.pdf_puts(b"\n");
            self.pdfendobj();
            self.pdfpagegroupval = 0;
        }

        self.write_refs();

        let st = &mut self.rt.images.pdf;
        let list = std::mem::take(&mut st.in_obj_list);
        let x = st.xref.clone();
        if let Some(di) = st.docs.iter().position(|d| d.id == id) {
            st.docs[di].in_obj_list = list;
            st.docs[di].xref = x;
        }
    }

    /// pdftoepdf.cc epdf_delete
    pub fn epdf_delete(&mut self) {
        let id = self.rt.images.epdf_doc;
        let Some(di) = self.epdf_doc_index(id) else { return };
        let st = &mut self.rt.images.pdf;
        st.xref = st.docs[di].xref.clone();
        if st.docs[di].occurences < 0 {
            self.delete_document(id);
        }
    }

    /// pdftoepdf.cc epdf_check_mem
    pub fn epdf_check_mem(&mut self) {
        if self.rt.images.pdf.is_init {
            self.rt.images.pdf.docs.clear();
        }
    }
}
