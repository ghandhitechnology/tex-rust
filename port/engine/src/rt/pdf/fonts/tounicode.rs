//! Port of pdftexdir/tounicode.c: \pdfglyphtounicode and /ToUnicode CMaps.

use std::collections::BTreeMap;

use super::*;
use crate::gen::globals::Globals;

const UNI_UNDEF: i64 = -1;
const UNI_STRING: i64 = -2;
const UNI_EXTRA_STRING: i64 = -3;

/// glyph_unicode_entry without the name (the map key)
#[derive(Clone, Debug, Default)]
pub struct GuEntry {
    pub code: i64,
    pub unicode_seq: Option<Vec<u8>>,
}

#[derive(Default)]
pub struct ToUnicodeState {
    /// glyph_unicode_tree (None = NULL)
    pub tree: Option<BTreeMap<Vec<u8>, GuEntry>>,
}

fn is_xdigit(c: u8) -> bool {
    c.is_ascii_digit() || (b'A'..=b'F').contains(&c)
}

/// sscanf %lX (unsigned long, 32 bits on Windows) with an optional field width
fn scan_hex(s: &[u8], width: usize) -> Option<u32> {
    let mut i = skip_ws(s, 0);
    let mut v: u32 = 0;
    let st = i;
    if at(s, i) == b'0' && (at(s, i + 1) == b'x' || at(s, i + 1) == b'X') && width > 2 && at(s, i + 2).is_ascii_hexdigit() {
        i += 2;
    }
    while at(s, i).is_ascii_hexdigit() && i - st < width {
        v = v.wrapping_mul(16).wrapping_add((at(s, i) as char).to_digit(16).unwrap());
        i += 1;
    }
    if i == st {
        None
    } else {
        Some(v)
    }
}

/// tounicode.c check_unicode_value
fn check_unicode_value(s: &[u8], multiple_value: bool) -> i64 {
    let l = s.len();
    let mut code: i64 = UNI_UNDEF;
    if l == 0 {
        return UNI_UNDEF;
    }
    if multiple_value && l % 4 != 0 {
        return UNI_UNDEF;
    }
    if !multiple_value && !(4..=6).contains(&l) {
        return UNI_UNDEF;
    }
    for i in 0..l {
        if !is_xdigit(s[i]) {
            return UNI_UNDEF;
        }
        if multiple_value {
            if i % 4 == 3 {
                match scan_hex(&s[i - 3..], 4) {
                    Some(c) => code = c as i32 as i64,
                    None => return UNI_UNDEF,
                }
                if !((0x0000..=0xD7FF).contains(&code) || (0xE000..=0xFFFF).contains(&code)) {
                    return UNI_UNDEF;
                }
            }
        } else if i == l - 1 {
            match scan_hex(s, usize::MAX) {
                Some(c) => code = c as i32 as i64,
                None => return UNI_UNDEF,
            }
            if !((0x0000..=0xD7FF).contains(&code) || (0xE000..=0x10FFFF).contains(&code)) {
                return UNI_UNDEF;
            }
        }
    }
    code
}

/// tounicode.c utf16be_str
fn utf16be_str(code: i64) -> String {
    if code <= 0xFFFF {
        format!("{:04X}", code)
    } else {
        let v = code - 0x10000;
        let vh = v / 0x400 + 0xD800;
        let vl = v % 0x400 + 0xDC00;
        format!("{:04X}{:04X}", vh, vl)
    }
}

/// tounicode.c is_last_byte_valid
fn is_last_byte_valid(src_code1: i32, src_code2: i32, code: i64) -> bool {
    let s = utf16be_str(code);
    let l = i64::from_str_radix(&s[s.len() - 2..], 16).unwrap_or(0);
    l < 255 - (src_code2 - src_code1) as i64
}

impl Globals {
    /// tounicode.c deftounicode
    pub fn deftounicode(&mut self, glyph: i32, unistr: i32) {
        let mut buf = self.tex_string(glyph);
        if let Some(z) = buf.iter().position(|&c| c == 0) {
            buf.truncate(z);
        }
        let mut p = self.tex_string(unistr);
        if let Some(z) = p.iter().position(|&c| c == 0) {
            p.truncate(z);
        }
        let lead = p.iter().take_while(|&&c| c == b' ').count();
        let p = p[lead..].to_vec();
        let mut l = p.len();
        while l > 0 && p[l - 1] == b' ' {
            l -= 1;
        }
        let mut valid_unistr = 1;
        for &c in &p[..l] {
            if c == b' ' {
                valid_unistr = 2;
            } else if !is_xdigit(c) {
                valid_unistr = 0;
                break;
            }
        }
        if l == 0 || valid_unistr == 0 || buf.is_empty() || buf == NOTDEF {
            self.pdftex_warn(&format!("ToUnicode: invalid parameter(s): `{}' => `{}'", bs(&buf), bs(&p)));
            return;
        }
        let mut gu = GuEntry { code: UNI_UNDEF, unicode_seq: None };
        if let Some(t) = &self.rt.fonts.tu.tree {
            if let Some(e) = t.get(&buf) {
                gu = e.clone();
            }
        }
        if valid_unistr == 2 {
            let buf2: Vec<u8> = p.iter().copied().filter(|&c| c != b' ').collect();
            gu.code = UNI_STRING;
            gu.unicode_seq = Some(buf2);
        } else {
            let v = scan_hex(&p, usize::MAX).unwrap_or(0);
            if v > 0x10FFFF {
                self.pdftex_warn(&format!("ToUnicode: value out of range [0,10FFFF]: {:X}", v));
                gu.code = UNI_UNDEF;
            } else {
                gu.code = v as i64;
            }
        }
        self.rt.fonts.tu.tree.get_or_insert_with(BTreeMap::new).insert(buf, gu);
    }

    fn gu_lookup(&self, name: &[u8]) -> Option<GuEntry> {
        self.rt.fonts.tu.tree.as_ref().and_then(|t| t.get(name).cloned())
    }

    /// tounicode.c set_glyph_unicode (for a glyph name that is not the notdef pointer)
    fn set_glyph_unicode(&self, s0: &[u8], tfmname: &[u8], gp: &mut GuEntry) {
        let mut s = s0.to_vec();
        if let Some(p) = s.iter().position(|&c| c == b'.') {
            s.truncate(p);
        }
        if s.is_empty() {
            return;
        }
        if s.contains(&b'_') {
            let mut buf2: Vec<u8> = Vec::new();
            for comp in s.split(|&c| c == b'_') {
                let mut tmp = GuEntry { code: UNI_UNDEF, unicode_seq: None };
                self.set_glyph_unicode(comp, tfmname, &mut tmp);
                match tmp.code {
                    UNI_UNDEF => {}
                    UNI_STRING | UNI_EXTRA_STRING => buf2.extend_from_slice(tmp.unicode_seq.as_deref().unwrap_or(b"")),
                    c => buf2.extend_from_slice(utf16be_str(c).as_bytes()),
                }
            }
            gp.code = UNI_EXTRA_STRING;
            gp.unicode_seq = Some(buf2);
            return;
        }
        // lookup for glyph name in the tfm's namespace
        let mut buf2 = b"tfm:".to_vec();
        buf2.extend_from_slice(tfmname);
        buf2.push(b'/');
        buf2.extend_from_slice(&s);
        buf2.truncate(SMALL_BUF_SIZE - 1);
        if let Some(e) = self.gu_lookup(&buf2) {
            *gp = e;
            return;
        }
        // lookup for glyph name in the main database
        let mut buf2 = s.clone();
        buf2.truncate(SMALL_BUF_SIZE - 1);
        if let Some(e) = self.gu_lookup(&buf2) {
            *gp = e;
            return;
        }
        if str_prefix(&s, b"uni") {
            let p2 = &s[3..];
            let code = check_unicode_value(p2, true);
            if code != UNI_UNDEF {
                if p2.len() == 4 {
                    gp.code = code;
                } else {
                    gp.code = UNI_EXTRA_STRING;
                    gp.unicode_seq = Some(p2.to_vec());
                }
            }
            return;
        }
        if str_prefix(&s, b"u") {
            let p2 = &s[1..];
            let code = check_unicode_value(p2, false);
            if code != UNI_UNDEF {
                gp.code = code;
            }
        }
    }

    /// tounicode.c write_tounicode: tfmname without .tfm, encname with .enc (None = builtin)
    pub fn write_tounicode(&mut self, glyph_names: &GlyphNames, tfmname: &[u8], encname: Option<&[u8]>) -> i32 {
        if self.rt.fonts.tu.tree.is_none() {
            self.pdftex_warn("no GlyphToUnicode entry has been inserted yet!");
            self.fixedgentounicode = 0;
            return 0;
        }
        let mut buf = tfmname.to_vec();
        buf.push(b'-');
        match encname {
            Some(e) => {
                buf.extend_from_slice(e);
                match buf.iter().rposition(|&c| c == b'.') {
                    Some(p) if &buf[p..] == b".enc" => buf.truncate(p),
                    _ => self.pdftex_warn(&format!("Dubious encoding file name: `{}'", bs(e))),
                }
            }
            None => buf.extend_from_slice(b"builtin"),
        }
        let objnum = self.pdfnewobjnum();
        self.pdfbegindict(objnum, 0);
        self.pdfbeginstream();
        let b = bs(&buf);
        let head = format!(
            "%!PS-Adobe-3.0 Resource-CMap\n\
             %%DocumentNeededResources: ProcSet (CIDInit)\n\
             %%IncludeResource: ProcSet (CIDInit)\n\
             %%BeginResource: CMap (TeX-{0}-0)\n\
             %%Title: (TeX-{0}-0 TeX {0} 0)\n\
             %%Version: 1.000\n\
             %%EndComments\n\
             /CIDInit /ProcSet findresource begin\n\
             12 dict begin\n\
             begincmap\n\
             /CIDSystemInfo\n\
             << /Registry (TeX)\n\
             /Ordering ({0})\n\
             /Supplement 0\n\
             >> def\n\
             /CMapName /TeX-{0}-0 def\n\
             /CMapType 2 def\n\
             1 begincodespacerange\n\
             <00> <FF>\n\
             endcodespacerange\n",
            b
        );
        self.pdf_printf(&head);
        let mut gtab: Vec<GuEntry> = vec![GuEntry { code: UNI_UNDEF, unicode_seq: None }; 257];
        for i in 0..256 {
            if let Some(g) = &glyph_names[i] {
                let mut e = GuEntry { code: UNI_UNDEF, unicode_seq: None };
                self.set_glyph_unicode(g, tfmname, &mut e);
                gtab[i] = e;
            }
        }
        let mut range_size = [0i32; 257];
        let mut i = 0usize;
        while i < 256 {
            let c = gtab[i].code;
            if c == UNI_STRING || c == UNI_EXTRA_STRING {
                range_size[i] = 1;
                i += 1;
            } else if c == UNI_UNDEF {
                range_size[i] = 0;
                i += 1;
            } else {
                let j = i;
                while i < 256
                    && gtab[i + 1].code >= 0
                    && gtab[i].code + 1 == gtab[i + 1].code
                    && is_last_byte_valid(j as i32, i as i32, gtab[i].code)
                {
                    i += 1;
                }
                i += 1;
                range_size[j] = (i - j) as i32;
            }
        }
        let mut bfrange_count = 0;
        let mut bfchar_count = 0;
        let mut i = 0usize;
        while i < 256 {
            if range_size[i] == 1 {
                bfchar_count += 1;
                i += 1;
            } else if range_size[i] > 1 {
                bfrange_count += 1;
                i += range_size[i] as usize;
            } else {
                i += 1;
            }
        }
        // write out bfrange
        let mut i = 0usize;
        loop {
            let subrange_count = if bfrange_count > 100 { 100 } else { bfrange_count };
            bfrange_count -= subrange_count;
            self.pdf_printf(&format!("{} beginbfrange\n", subrange_count));
            for _ in 0..subrange_count {
                while range_size[i] <= 1 && i < 256 {
                    i += 1;
                }
                let s = format!("<{:02X}> <{:02X}> <{}>\n", i, i as i32 + range_size[i] - 1, utf16be_str(gtab[i].code));
                self.pdf_printf(&s);
                i += range_size[i] as usize;
            }
            self.pdf_printf("endbfrange\n");
            if bfrange_count <= 0 {
                break;
            }
        }
        // write out bfchar
        let mut i = 0usize;
        loop {
            let subrange_count = if bfchar_count > 100 { 100 } else { bfchar_count };
            bfchar_count -= subrange_count;
            self.pdf_printf(&format!("{} beginbfchar\n", subrange_count));
            for _ in 0..subrange_count {
                while i < 256 {
                    if range_size[i] > 1 {
                        i += range_size[i] as usize;
                    } else if range_size[i] == 0 {
                        i += 1;
                    } else {
                        break;
                    }
                }
                let c = gtab[i].code;
                let mut s = format!("<{:02X}> <", i).into_bytes();
                if c == UNI_STRING || c == UNI_EXTRA_STRING {
                    s.extend_from_slice(gtab[i].unicode_seq.as_deref().unwrap_or(b""));
                } else {
                    s.extend_from_slice(utf16be_str(c).as_bytes());
                }
                s.extend_from_slice(b">\n");
                self.pdf_printf_b(&s);
                i += 1;
            }
            self.pdf_printf("endbfchar\n");
            if bfchar_count <= 0 {
                break;
            }
        }
        self.pdf_printf("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n%%EndResource\n%%EOF\n");
        self.pdfendstream();
        objnum
    }

    fn dump_cstr(&mut self, s: &[u8]) {
        self.dump_int(s.len() as i32 + 1);
        let mut v = s.to_vec();
        v.push(0);
        self.dump_things(v.as_mut_ptr(), v.len() as i32);
    }

    fn undump_cstr(&mut self) -> Option<Vec<u8>> {
        let x = self.undump_int();
        if x > 0 {
            let mut v = vec![0u8; x as usize];
            self.undump_things(v.as_mut_ptr(), x);
            if let Some(z) = v.iter().position(|&c| c == 0) {
                v.truncate(z);
            }
            Some(v)
        } else {
            None
        }
    }

    /// tounicode.c dumptounicode
    pub fn dumptounicode(&mut self) {
        let entries: Vec<(Vec<u8>, GuEntry)> = match &self.rt.fonts.tu.tree {
            None => {
                self.dump_int(0);
                return;
            }
            Some(t) => t.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        };
        self.dump_int(entries.len() as i32);
        for (name, gu) in entries {
            self.dump_cstr(&name);
            self.dump_int(gu.code as i32);
            if gu.code == UNI_STRING {
                let s = gu.unicode_seq.unwrap_or_default();
                self.dump_cstr(&s);
            }
        }
    }

    /// tounicode.c undumptounicode
    pub fn undumptounicode(&mut self) {
        let mut remaining = self.undump_int();
        if remaining == 0 {
            return;
        }
        let mut t = BTreeMap::new();
        while remaining > 0 {
            remaining -= 1;
            let name = match self.undump_cstr() {
                Some(n) => n,
                None => self.pdftex_fail("undumpcharptr(gu->name) got NULL"),
            };
            let code = self.undump_int() as i64;
            let mut gu = GuEntry { code, unicode_seq: None };
            if code == UNI_STRING {
                match self.undump_cstr() {
                    Some(s) => gu.unicode_seq = Some(s),
                    None => self.pdftex_fail("undumpcharptr(gu->unicode_seq) got NULL"),
                }
            }
            t.entry(name).or_insert(gu);
        }
        self.rt.fonts.tu.tree = Some(t);
    }

    /// tounicode.c glyph_unicode_free
    pub fn glyph_unicode_free(&mut self) {
        self.rt.fonts.tu.tree = None;
    }
}
