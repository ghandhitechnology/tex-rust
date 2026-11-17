//! Port of pdftexdir/writefont.c: font dictionaries, font descriptors, /Widths arrays.

use std::collections::{BTreeMap, BTreeSet};

use super::mapfile::check_std_t1font;
use super::*;
use crate::gen::globals::Globals;

/// writefont.c static state: fo_tree (by tfm_name) and fd_tree (by ff_name, slant,
/// extend) over the fo and fd arenas.
#[derive(Default)]
pub struct WriteFontState {
    pub fo: Vec<FoEntry>,
    pub fd: Vec<FdEntry>,
    pub fo_tree: BTreeMap<Vec<u8>, usize>,
    pub fd_tree: BTreeMap<(Vec<u8>, i32, i32), usize>,
}

const M_PI: f64 = 3.14159265358979323846;

impl Globals {
    pub fn fd(&self, i: usize) -> &FdEntry {
        &self.rt.fonts.wf.fd[i]
    }
    pub fn fd_mut(&mut self, i: usize) -> &mut FdEntry {
        &mut self.rt.fonts.wf.fd[i]
    }
    fn fo(&self, i: usize) -> &FoEntry {
        &self.rt.fonts.wf.fo[i]
    }
    fn fo_mut(&mut self, i: usize) -> &mut FoEntry {
        &mut self.rt.fonts.wf.fo[i]
    }

    /// writefont.c new_fd_entry (the entry is put into the fd arena)
    pub fn new_fd_entry(&mut self) -> usize {
        self.rt.fonts.wf.fd.push(FdEntry::default());
        self.rt.fonts.wf.fd.len() - 1
    }

    /// writefont.c preset_fontmetrics
    fn preset_fontmetrics(&mut self, fd: usize, f: i32) {
        let size = *ix(self.pdffontsize, f);
        let ia = d2i(-(self.getslant(f) as f64 / 65536.0).atan() * (180.0 / M_PI));
        let mut d = [IntParm::default(); FONT_KEYS_NUM];
        d[ITALIC_ANGLE_CODE].val = self.dividescaled(ia, size, 3);
        let h = self.getcharheight(f, b'h');
        d[ASCENT_CODE].val = self.dividescaled(h, size, 3);
        let hh = self.getcharheight(f, b'H');
        d[CAPHEIGHT_CODE].val = self.dividescaled(hh, size, 3);
        let y = self.getchardepth(f, b'y');
        let i = -self.dividescaled(y, size, 3);
        d[DESCENT_CODE].val = if i < 0 { i } else { 0 };
        let w = self.getcharwidth(f, b'.') / 3;
        d[STEMV_CODE].val = self.dividescaled(w, size, 3);
        let x = self.getxheight(f);
        d[XHEIGHT_CODE].val = self.dividescaled(x, size, 3);
        d[FONTBBOX1_CODE].val = 0;
        d[FONTBBOX2_CODE].val = d[DESCENT_CODE].val;
        let q = self.getquad(f);
        d[FONTBBOX3_CODE].val = self.dividescaled(q, size, 3);
        d[FONTBBOX4_CODE].val = if d[CAPHEIGHT_CODE].val > d[ASCENT_CODE].val { d[CAPHEIGHT_CODE].val } else { d[ASCENT_CODE].val };
        for e in d.iter_mut().take(INT_KEYS_NUM) {
            e.set = true;
        }
        let fdm = &mut self.fd_mut(fd).font_dim;
        for k in 0..INT_KEYS_NUM {
            fdm[k] = d[k];
        }
    }

    /// writefont.c fix_fontmetrics
    fn fix_fontmetrics(&mut self, fd: usize) {
        let p = self.fd(fd).font_dim;
        if !p[FONTBBOX1_CODE].set || !p[FONTBBOX2_CODE].set || !p[FONTBBOX3_CODE].set || !p[FONTBBOX4_CODE].set {
            let ff = bs(self.fm(self.fd(fd).fm).ff());
            self.pdftex_warn(&format!("font `{}' doesn't have a BoundingBox", ff));
            return;
        }
        let p = &mut self.fd_mut(fd).font_dim;
        if !p[ASCENT_CODE].set {
            p[ASCENT_CODE].val = p[FONTBBOX4_CODE].val;
            p[ASCENT_CODE].set = true;
        }
        if !p[DESCENT_CODE].set {
            p[DESCENT_CODE].val = p[FONTBBOX2_CODE].val;
            p[DESCENT_CODE].set = true;
        }
        if !p[CAPHEIGHT_CODE].set {
            p[CAPHEIGHT_CODE].val = p[FONTBBOX4_CODE].val;
            p[CAPHEIGHT_CODE].set = true;
        }
    }

    /// writefont.c write_fontmetrics
    fn write_fontmetrics(&mut self, fd: usize) {
        self.fix_fontmetrics(fd);
        let d = self.fd(fd).font_dim;
        if d[FONTBBOX1_CODE].set && d[FONTBBOX2_CODE].set && d[FONTBBOX3_CODE].set && d[FONTBBOX4_CODE].set {
            self.pdf_printf(&format!(
                "/{} [{} {} {} {}]\n",
                FONT_KEY[FONTBBOX1_CODE].0,
                d[FONTBBOX1_CODE].val,
                d[FONTBBOX2_CODE].val,
                d[FONTBBOX3_CODE].val,
                d[FONTBBOX4_CODE].val
            ));
        }
        for i in 0..GEN_KEY_NUM {
            if d[i].set {
                self.pdf_printf(&format!("/{} {}\n", FONT_KEY[i].0, d[i].val));
            }
        }
    }

    /// writefont.c preset_fontname
    fn preset_fontname(&mut self, fo: usize) {
        let fm = self.fm(self.fo(fo).fm);
        let name = match &fm.ps_name {
            Some(p) => p.clone(),
            None => fm.tfm().to_vec(),
        };
        let fd = self.fo(fo).fd.unwrap();
        self.fd_mut(fd).fontname = Some(name);
    }

    /// writefont.c write_fontname
    pub fn write_fontname(&mut self, fd: usize, key: Option<&str>) {
        self.pdf_puts(b"/");
        if let Some(k) = key {
            self.pdf_printf(&format!("{} /", k));
        }
        if let Some(t) = self.fd(fd).subset_tag.clone() {
            let mut s = t;
            s.push(b'+');
            self.pdf_printf_b(&s);
        }
        let mut s = self.fd(fd).fontname.clone().expect("fontname");
        s.push(b'\n');
        self.pdf_printf_b(&s);
    }

    /// writefont.c write_fontname_object
    fn write_fontname_object(&mut self, fd: usize) {
        let n = self.fd(fd).fn_objnum;
        self.pdfbeginobj(n, 1);
        self.write_fontname(fd, None);
        self.pdfendobj();
    }

    /// writefont.c lookup_fd_entry
    pub fn lookup_fd_entry(&mut self, s: &[u8], slant: i32, extend: i32) -> Option<usize> {
        self.rt.fonts.wf.fd_tree.get(&(s.to_vec(), slant, extend)).copied()
    }

    /// writefont.c register_fd_entry
    pub fn register_fd_entry(&mut self, fd: usize) {
        let fm = self.fm(self.fd(fd).fm);
        let key = (fm.ff().to_vec(), fm.slant, fm.extend);
        self.rt.fonts.wf.fd_tree.entry(key).or_insert(fd);
    }

    /// writefont.c create_fontdescriptor
    fn create_fontdescriptor(&mut self, fo: usize, f: i32) {
        let fd = self.new_fd_entry();
        self.fo_mut(fo).fd = Some(fd);
        self.preset_fontname(fo);
        self.preset_fontmetrics(fd, f);
        let (fe, fm) = (self.fo(fo).fe, self.fo(fo).fm);
        let e = self.fd_mut(fd);
        e.fe = fe;
        e.fm = fm;
        e.gl_tree = Some(BTreeSet::new());
    }

    /// writefont.c mark_reenc_glyphs
    fn mark_reenc_glyphs(&mut self, fo: usize, f: i32) {
        let fm = self.fo(fo).fm;
        if self.fm(fm).is_subsetted() {
            let fe = self.fo(fo).fe.unwrap();
            let fd = self.fo(fo).fd.unwrap();
            let (first, last) = (self.fo(fo).first_char, self.fo(fo).last_char);
            for i in first..=last {
                if self.pdfcharmarked(f, i) {
                    if let Some(g) = self.rt.fonts.enc.fe[fe].glyph_names[i as usize].clone() {
                        self.fd_mut(fd).gl_tree.as_mut().unwrap().insert(g);
                    }
                }
            }
        }
    }

    /// writefont.c mark_chars
    fn mark_chars(&mut self, fo: usize, tx_tree: Option<BTreeSet<i32>>, f: i32) -> BTreeSet<i32> {
        let mut t = tx_tree.unwrap_or_default();
        let (first, last) = (self.fo(fo).first_char, self.fo(fo).last_char);
        for i in first..=last {
            if self.pdfcharmarked(f, i) {
                t.insert(i);
            }
        }
        t
    }

    /// writefont.c get_char_range
    fn get_char_range(&mut self, fo: usize, f: i32) {
        let bc = *ix(self.fontbc, f) as i32;
        let ec = *ix(self.fontec, f) as i32;
        let mut i = bc;
        while i <= ec {
            if self.pdfcharmarked(f, i) {
                break;
            }
            i += 1;
        }
        let first = i;
        let mut i = ec;
        while i >= bc {
            if self.pdfcharmarked(f, i) {
                break;
            }
            i -= 1;
        }
        let last = i;
        let e = self.fo_mut(fo);
        e.first_char = first;
        e.last_char = last;
        if first > last || !self.pdfcharmarked(f, first) {
            let e = self.fo_mut(fo);
            e.last_char = 0;
            e.first_char = 1;
        }
    }

    /// writefont.c create_charwidth_array
    fn create_charwidth_array(&mut self, fo: usize, f: i32) {
        let mut w = vec![0i32; 256];
        let (first, last) = (self.fo(fo).first_char, self.fo(fo).last_char);
        let size = *ix(self.pdffontsize, f);
        for i in first..=last {
            let cw = self.getcharwidth(f, i as u8);
            w[i as usize] = self.dividescaled(cw, size, 4);
        }
        self.fo_mut(fo).cw_width = Some(w);
    }

    /// writefont.c write_charwidth_array
    fn write_charwidth_array(&mut self, fo: usize) {
        let n = self.pdfnewobjnum();
        self.fo_mut(fo).cw_objnum = n;
        self.pdfbeginobj(n, 1);
        self.pdf_puts(b"[");
        let (first, last) = (self.fo(fo).first_char, self.fo(fo).last_char);
        let w = self.fo(fo).cw_width.clone().unwrap();
        for i in first..=last {
            let wi = w[i as usize];
            self.pdf_printf(&format!("{}", wi / 10));
            let j = wi % 10;
            if j != 0 {
                self.pdf_printf(&format!(".{}", j));
            }
            if i != last {
                self.pdf_puts(b" ");
            }
        }
        self.pdf_puts(b"]\n");
        self.pdfendobj();
    }

    /// writefont.c lookup_fo_entry
    fn lookup_fo_entry(&self, s: &[u8]) -> Option<usize> {
        self.rt.fonts.wf.fo_tree.get(s).copied()
    }

    /// writefont.c register_fo_entry
    fn register_fo_entry(&mut self, fo: usize) {
        let tfm = self.fm(self.fo(fo).fm).tfm().to_vec();
        debug_assert!(self.lookup_fo_entry(&tfm).is_none());
        self.rt.fonts.wf.fo_tree.entry(tfm).or_insert(fo);
    }

    /// writefont.c write_fontfile
    fn write_fontfile(&mut self, fd: usize) {
        let fm = self.fd(fd).fm;
        let (t1, tt, otf) = (self.fm(fm).is_type1(), self.fm(fm).is_truetype(), self.fm(fm).is_opentype());
        if t1 {
            self.writet1(fd);
        } else if tt {
            self.writettf(fd);
        } else if otf {
            self.writeotf(fd);
        } else {
            unreachable!();
        }
        if !self.fd(fd).ff_found {
            return;
        }
        let n = self.pdfnewobjnum();
        self.fd_mut(fd).ff_objnum = n;
        self.pdfbegindict(n, 0);
        if t1 {
            let t = &self.rt.fonts.t1;
            let s = format!("/Length1 {}\n/Length2 {}\n/Length3 {}\n", t.length1, t.length2, t.length3);
            self.pdf_printf(&s);
        } else if tt {
            let s = format!("/Length1 {}\n", self.rt.fonts.ttf.ttf_length);
            self.pdf_printf(&s);
        } else {
            self.pdf_puts(b"/Subtype /Type1C\n");
        }
        self.pdfbeginstream();
        self.fb_flush();
        self.pdfendstream();
    }

    /// writefont.c write_fontdescriptor
    pub fn write_fontdescriptor(&mut self, fd: usize) {
        const STD_FLAGS: [i32; 14] = [
            1 + 2 + (1 << 5),
            1 + 2 + (1 << 5) + (1 << 18),
            1 + 2 + (1 << 5) + (1 << 6),
            1 + 2 + (1 << 5) + (1 << 6) + (1 << 18),
            1 << 5,
            (1 << 5) + (1 << 18),
            (1 << 5) + (1 << 6),
            (1 << 5) + (1 << 6) + (1 << 18),
            4,
            2 + (1 << 5),
            2 + (1 << 5) + (1 << 18),
            2 + (1 << 5) + (1 << 6),
            2 + (1 << 5) + (1 << 6) + (1 << 18),
            4,
        ];
        let fm = self.fd(fd).fm;
        if self.fm(fm).is_fontfile() {
            self.write_fontfile(fd);
        }
        if self.fd(fd).fn_objnum != 0 {
            self.write_fontname_object(fd);
        }
        if self.fd(fd).fd_objnum == 0 {
            let n = self.pdfnewobjnum();
            self.fd_mut(fd).fd_objnum = n;
        }
        let n = self.fd(fd).fd_objnum;
        self.pdfbegindict(n, 1);
        self.pdf_puts(b"/Type /FontDescriptor\n");
        self.write_fontname(fd, Some("FontName"));
        let e = self.fm(fm).clone();
        let fd_flags = if e.fd_flags != FD_FLAGS_NOT_SET_IN_MAPLINE {
            e.fd_flags
        } else if self.fd(fd).ff_found {
            FD_FLAGS_DEFAULT_EMBED
        } else {
            let fl = if e.is_std_t1font() {
                STD_FLAGS[check_std_t1font(e.ps_name.as_deref().unwrap()) as usize]
            } else {
                FD_FLAGS_DEFAULT_NON_EMBED
            };
            let ps = match &e.ps_name {
                Some(p) => bs(p),
                None => "No name given".to_string(),
            };
            self.pdftex_warn(&format!(
                "No flags specified for non-embedded font `{}' ({}) (I'm using {}): fix your map entry.",
                ps,
                bs(e.tfm()),
                fl
            ));
            fl
        };
        self.pdf_printf(&format!("/Flags {}\n", fd_flags));
        self.write_fontmetrics(fd);
        if self.fd(fd).ff_found {
            if self.getpdfomitcharset() == 0 && e.is_subsetted() && e.is_type1() {
                let glyphs: Vec<Vec<u8>> = self.fd(fd).gl_tree.as_ref().unwrap().iter().cloned().collect();
                self.pdf_puts(b"/CharSet (");
                for g in glyphs {
                    let mut s = b"/".to_vec();
                    s.extend_from_slice(&g);
                    self.pdf_printf_b(&s);
                }
                self.pdf_puts(b")\n");
            }
            let ff = self.fd(fd).ff_objnum;
            if e.is_type1() {
                self.pdf_printf(&format!("/FontFile {} 0 R\n", ff));
            } else if e.is_truetype() {
                self.pdf_printf(&format!("/FontFile2 {} 0 R\n", ff));
            } else if e.is_opentype() {
                self.pdf_printf(&format!("/FontFile3 {} 0 R\n", ff));
            }
        }
        self.pdfenddict();
    }

    /// writefont.c write_fontdescriptors
    fn write_fontdescriptors(&mut self) {
        let list: Vec<usize> = self.rt.fonts.wf.fd_tree.values().copied().collect();
        for fd in list {
            self.write_fontdescriptor(fd);
        }
    }

    /// writefont.c write_fontdictionary
    fn write_fontdictionary(&mut self, fo: usize) {
        let fm = self.fo(fo).fm;
        let e = self.fm(fm).clone();
        let fd = self.fo(fo).fd;
        let tex_font = self.fo(fo).tex_font;
        let a = self.fixedgentounicode > 0 && fd.is_some() && !*ix(self.pdffontnobuiltintounicode, tex_font);
        let b = !(fd.is_some() && e.tfm_name.is_some() && e.tfm() != b"dummy-space");
        if a || b {
            if let Some(fe) = self.fo(fo).fe {
                let gl = self.rt.fonts.enc.fe[fe].glyph_names.clone();
                let name = self.rt.fonts.enc.fe[fe].name.clone();
                let n = self.write_tounicode(&gl, e.tfm(), Some(&name));
                self.fo_mut(fo).tounicode_objnum = n;
            } else if e.is_type1() {
                let fd = fd.unwrap();
                let gl = match self.fd(fd).builtin_glyph_names.clone() {
                    Some(g) => g,
                    None => self.pdftex_fail("builtin glyph names is empty"),
                };
                let n = self.write_tounicode(&gl, e.tfm(), None);
                self.fo_mut(fo).tounicode_objnum = n;
            }
        }
        let n = self.fo(fo).fo_objnum;
        self.pdfbegindict(n, 1);
        self.pdf_puts(b"/Type /Font\n");
        self.pdf_puts(b"/Subtype /");
        if e.is_type1() {
            self.pdf_printf("Type1\n");
        } else if e.is_truetype() {
            self.pdf_printf("TrueType\n");
        } else if e.is_opentype() {
            self.pdf_printf("Type1\n");
        }
        let fd = fd.unwrap();
        self.write_fontname(fd, Some("BaseFont"));
        let s = format!("/FontDescriptor {} 0 R\n", self.fd(fd).fd_objnum);
        self.pdf_printf(&s);
        let o = self.fo(fo);
        let s = format!("/FirstChar {}\n/LastChar {}\n/Widths {} 0 R\n", o.first_char, o.last_char, o.cw_objnum);
        self.pdf_printf(&s);
        if e.is_type1() || e.is_opentype() {
            if let Some(fe) = self.fo(fo).fe {
                let fe_obj = self.rt.fonts.enc.fe[fe].fe_objnum;
                if fe_obj != 0 {
                    self.pdf_printf(&format!("/Encoding {} 0 R\n", fe_obj));
                }
            }
        }
        let tu = self.fo(fo).tounicode_objnum;
        if tu != 0 {
            self.pdf_printf(&format!("/ToUnicode {} 0 R\n", tu));
        }
        let attr = *ix(self.pdffontattr, tex_font);
        if attr != self.getnullstr() {
            self.pdfprint(attr);
            self.pdf_puts(b"\n");
        }
        self.pdfenddict();
    }

    /// writefont.c write_fontdictionaries
    fn write_fontdictionaries(&mut self) {
        let list: Vec<usize> = self.rt.fonts.wf.fo_tree.values().copied().collect();
        for fo in list {
            self.write_fontdictionary(fo);
        }
    }

    /// writefont.c writefontstuff: final flush of all font related stuff.
    pub fn writefontstuff(&mut self) {
        crate::rt::phase::mark("writefontstuff");
        self.write_fontdescriptors();
        self.write_fontencodings();
        self.write_fontdictionaries();
    }

    /// writefont.c create_fontdictionary
    fn create_fontdictionary(&mut self, fm: usize, font_objnum: i32, f: i32) {
        self.rt.fonts.wf.fo.push(FoEntry {
            fo_objnum: 0,
            tex_font: 0,
            fm: 0,
            fd: None,
            fe: None,
            cw_objnum: 0,
            cw_width: None,
            first_char: 1,
            last_char: 0,
            tounicode_objnum: 0,
        });
        let fo = self.rt.fonts.wf.fo.len() - 1;
        self.get_char_range(fo, f);
        {
            let o = self.fo_mut(fo);
            o.fm = fm;
            o.fo_objnum = font_objnum;
            o.tex_font = f;
        }
        let e = self.fm(fm).clone();
        if let Some(enc) = &e.encname {
            let fe = self.get_fe_entry(enc);
            self.fo_mut(fo).fe = fe;
            if let Some(fe) = fe {
                if e.is_type1() || e.is_opentype() {
                    if self.rt.fonts.enc.fe[fe].fe_objnum == 0 {
                        let n = self.pdfnewobjnum();
                        self.rt.fonts.enc.fe[fe].fe_objnum = n;
                    }
                    let t = self.rt.fonts.enc.fe[fe].tx_tree.take();
                    let t = self.mark_chars(fo, t, f);
                    self.rt.fonts.enc.fe[fe].tx_tree = Some(t);
                }
            }
        }
        if !e.is_builtin() {
            if e.is_type1() {
                match self.lookup_fd_entry(e.ff(), e.slant, e.extend) {
                    Some(fd) => self.fo_mut(fo).fd = Some(fd),
                    None => {
                        self.create_fontdescriptor(fo, f);
                        let fd = self.fo(fo).fd.unwrap();
                        self.register_fd_entry(fd);
                    }
                }
            } else {
                self.create_fontdescriptor(fo, f);
            }
            self.create_charwidth_array(fo, f);
            self.write_charwidth_array(fo);
            let fd = self.fo(fo).fd.unwrap();
            if self.fo(fo).fe.is_some() {
                self.mark_reenc_glyphs(fo, f);
                if !e.is_type1() {
                    let t = self.fd_mut(fd).tx_tree.take();
                    let t = self.mark_chars(fo, t, f);
                    self.fd_mut(fd).tx_tree = Some(t);
                    if e.is_truetype() {
                        self.fd_mut(fd).write_ttf_glyph_names = true;
                    }
                }
            } else {
                let t = self.fd_mut(fd).tx_tree.take();
                let t = self.mark_chars(fo, t, f);
                self.fd_mut(fd).tx_tree = Some(t);
            }
            if !e.is_type1() {
                self.write_fontdescriptor(fd);
            }
        } else {
            self.create_charwidth_array(fo, f);
            self.write_charwidth_array(fo);
            self.create_fontdescriptor(fo, f);
            let fd = self.fo(fo).fd.unwrap();
            self.write_fontdescriptor(fd);
            if !e.is_std_t1font() {
                let ps = e.ps_name.as_deref().map(bs).unwrap_or_else(|| "(null)".into());
                self.pdftex_warn(&format!("font `{}' is not a standard font; I suppose it is available to your PDF viewer then", ps));
            }
        }
        if e.is_type1() {
            self.register_fo_entry(fo);
        } else {
            self.write_fontdictionary(fo);
        }
    }

    /// writefont.c font_is_used
    fn font_is_used(&mut self, f: i32) -> bool {
        let bc = *ix(self.fontbc, f) as i32;
        let ec = *ix(self.fontec, f) as i32;
        let mut i = bc;
        while i <= ec {
            if self.pdfcharmarked(f, i) {
                break;
            }
            i += 1;
        }
        let s = i;
        let mut i = ec;
        while i >= bc {
            if self.pdfcharmarked(f, i) {
                break;
            }
            i -= 1;
        }
        s <= i
    }

    /// writefont.c dopdffont
    pub fn dopdffont(&mut self, font_objnum: i32, f: i32) {
        if !self.font_is_used(f) {
            return;
        }
        let fm = if self.hasfmentry(f) { Some(self.font_fm(f)) } else { None };
        match fm {
            Some(fm) if !self.fm(fm).is_pk() => self.create_fontdictionary(fm, font_objnum, f),
            _ => self.writet3(fm, font_objnum, f),
        }
    }
}
