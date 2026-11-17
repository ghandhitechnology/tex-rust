//! Port of pdftexdir/writet3.c: Type 3 fonts from PK bitmaps (or .pgc files).

use std::collections::BTreeSet;

use super::pkin::CharDesc;
use super::*;
use crate::gen::globals::Globals;
use crate::rt::kpse;

const T3_BUF_SIZE: usize = 1024;

/// writet3.c static state
pub struct T3State {
    line: Vec<u8>,
    image_used: bool,
    char_procs: [i32; 256],
    char_widths: [f32; 256],
    glyph_num: i32,
    font_scale: f32,
    b0: i32,
    b1: i32,
    b2: i32,
    b3: i32,
    is_pk_font: bool,
    pub file: CFile,
}

impl Default for T3State {
    fn default() -> Self {
        T3State {
            line: vec![0],
            image_used: false,
            char_procs: [0; 256],
            char_widths: [0.0; 256],
            glyph_num: 0,
            font_scale: 0.0,
            b0: 0,
            b1: 0,
            b2: 0,
            b3: 0,
            is_pk_font: false,
            file: CFile::default(),
        }
    }
}

impl Globals {
    fn t3(&mut self) -> &mut T3State {
        &mut self.rt.fonts.t3
    }

    fn t3_prefix(&self, s: &[u8]) -> bool {
        str_prefix(&self.rt.fonts.t3.line, s)
    }

    /// writet3.c t3_getline (line kept without the NUL; length = t3_line_ptr)
    fn t3_getline(&mut self) {
        loop {
            let mut p: Vec<u8> = Vec::new();
            let mut c = self.t3().file.getc();
            while !self.t3().file.feof() {
                append_char_to_buf(&mut c, &mut p, usize::MAX);
                if c == 10 {
                    break;
                }
                c = self.t3().file.getc();
            }
            if !append_eol(&mut p, T3_BUF_SIZE) {
                self.check_buf_fail("writet3.c", 66);
            }
            let short = p.len() < 2 || p.first() == Some(&b'%');
            self.t3().line = p;
            if short && !self.t3().file.feof() {
                continue;
            }
            return;
        }
    }

    fn t3_putline(&mut self) {
        let l = self.t3().line.clone();
        for b in l {
            self.pdfout(b);
        }
    }

    /// writet3.c update_bbox
    fn update_bbox(&mut self, llx: i32, lly: i32, urx: i32, ury: i32, is_first_glyph: bool) {
        let t = self.t3();
        if is_first_glyph {
            t.b0 = llx;
            t.b1 = lly;
            t.b2 = urx;
            t.b3 = ury;
        } else {
            if llx < t.b0 {
                t.b0 = llx;
            }
            if lly < t.b1 {
                t.b1 = lly;
            }
            if urx > t.b2 {
                t.b2 = urx;
            }
            if ury > t.b3 {
                t.b3 = ury;
            }
        }
    }

    /// writet3.c t3_write_glyph
    fn t3_write_glyph(&mut self, f: i32) {
        const BEGIN: &[u8] = b"\\pdfglyph";
        const END: &[u8] = b"\\endglyph";
        self.t3_getline();
        if !self.t3_prefix(BEGIN) {
            return;
        }
        let mut line = self.t3().line.clone();
        line.push(0);
        let mut v = [0i64; 8];
        let mut pos = BEGIN.len() + 1;
        let mut ok = pos <= line.len();
        for x in v.iter_mut() {
            if !ok {
                break;
            }
            match scan_int(&line, pos, 0) {
                Some((n, e)) => {
                    *x = n as i32 as i64;
                    pos = e;
                }
                None => ok = false,
            }
        }
        if !ok {
            let mut l = self.t3().line.clone();
            if l.last() == Some(&10) {
                l.pop();
            }
            self.pdftex_fail(&format!("invalid glyph preamble: `{}'", bs(&l)));
        }
        let [glyph_index, width, _height, _depth, llx, lly, urx, ury] = v.map(|x| x as i32);
        if glyph_index < *ix(self.fontbc, f) as i32 || glyph_index > *ix(self.fontec, f) as i32 {
            return;
        }
        if !self.pdfcharmarked(f, glyph_index) {
            while !self.t3_prefix(END) {
                if self.t3().file.feof() {
                    self.pdftex_fail("unexpected end of file");
                }
                self.t3_getline();
            }
            return;
        }
        let first = self.t3().glyph_num == 0;
        self.update_bbox(llx, lly, urx, ury, first);
        self.t3().glyph_num += 1;
        self.pdfnewdict(0, 0, 0);
        let gi = glyph_index as usize;
        self.rt.fonts.t3.char_procs[gi] = self.objptr;
        if width == 0 {
            let cw = self.getcharwidth(f, glyph_index as u8);
            let size = *ix(self.pdffontsize, f);
            let t = self.t3();
            t.char_widths[gi] = (cw as f32 / t.font_scale) / size as f32;
        } else {
            self.t3().char_widths[gi] = width as f32;
        }
        self.pdfbeginstream();
        self.t3_getline();
        let w = self.t3().char_widths[gi] as i32;
        self.pdf_printf(&format!("{} 0 {} {} {} {} d1\nq\n", w, llx, lly, urx, ury));
        while !self.t3_prefix(END) {
            if self.t3().file.feof() {
                self.pdftex_fail("unexpected end of file");
            }
            if self.t3_prefix(b"BI") {
                self.t3().image_used = true;
            }
            self.t3_putline();
            self.t3_getline();
        }
        self.pdf_puts(b"Q\n");
        self.pdfendstream();
    }

    /// writet3.c get_pk_font_scale
    fn get_pk_font_scale(&mut self, f: i32) -> i32 {
        let a = self.dividescaled(*ix(self.pdffontsize, f), self.onehundredbp, self.fixeddecimaldigits + 2);
        self.dividescaled(self.pkscalefactor, a, 0)
    }

    /// writet3.c pk_char_width
    fn pk_char_width(&mut self, f: i32, w: i32) -> i32 {
        let a = self.dividescaled(w, *ix(self.pdffontsize, f), 7);
        let s = self.get_pk_font_scale(f);
        self.dividescaled(a, s, 0)
    }

    /// writet3.c getpkcharwidth
    pub fn getpkcharwidth(&mut self, f: i32, w: i32) -> i32 {
        let s = self.get_pk_font_scale(f) as f64;
        let c = self.pk_char_width(f, w) as f64;
        d2i((s / 100000.0) * (c / 100.0) * *ix(self.pdffontsize, f) as f64)
    }

    /// writet3.c writepk
    fn writepk(&mut self, f: i32) -> bool {
        let res = self.fixedpkresolution;
        let r = res as f32 * (*ix(self.pdffontsize, f) as f32 / *ix(self.fontdsize, f) as f32);
        let dpi0 = (r as f64).round() as i64 as u32;
        let (dpi, _) = texrs_kpathsea::Kpse::magstep_fix(dpi0, res as u32);
        let mut name = self.tex_string(*ix(self.fontname, f));
        if let Some(z) = name.iter().position(|&c| c == 0) {
            name.truncate(z);
        }
        self.rt.pdfu.cur_file_name = Some(bs(&name));
        // kpse_find_pk(cur_file_name, dpi, &font_ret), FILESTRCASEEQ, kpse_bitmap_tolerance
        let found = self.rt.kpse().find_glyph(&name, dpi, kpse::PK_FORMAT);
        let path = match found {
            Some((p, info))
                if info.name.eq_ignore_ascii_case(&name)
                    && texrs_kpathsea::Kpse::bitmap_tolerance(info.dpi as f64, dpi as f64) =>
            {
                p
            }
            _ => {
                self.pdftex_fail(&format!("Font {} at {} not found", bs(&name), dpi));
            }
        };
        let file = self.font_xfopen(&path);
        self.t3().file = file;
        self.recorder_record_input(&path);
        self.t3().image_used = true;
        self.t3().is_pk_font = true;
        let mut m = b" <".to_vec();
        m.extend_from_slice(&path);
        self.tex_printf_b(&m);
        let mut cd = CharDesc { rastersize: 256, raster: vec![0; 256], ..Default::default() };
        let mut check_preamble = true;
        while self.readchar(check_preamble, &mut cd) != 0 {
            check_preamble = false;
            if !(0..256).contains(&cd.charcode) || !self.pdfcharmarked(f, cd.charcode) {
                continue;
            }
            let cc = cd.charcode as usize;
            let cw = self.getcharwidth(f, cd.charcode as u8);
            let pw = self.pk_char_width(f, cw);
            self.t3().char_widths[cc] = pw as f32;
            let is_null_glyph;
            if cd.cwidth < 1 || cd.cheight < 1 {
                let w = d2i((self.t3().char_widths[cc] as f64 / 100.0).round());
                cd.cwidth = w;
                cd.xescape = w;
                cd.cheight = 1;
                cd.xoff = 0;
                cd.yoff = 0;
                is_null_glyph = true;
            } else {
                is_null_glyph = false;
            }
            let llx = -cd.xoff;
            let lly = cd.yoff - cd.cheight + 1;
            let urx = cd.cwidth + llx + 1;
            let ury = cd.cheight + lly;
            let first = self.t3().glyph_num == 0;
            self.update_bbox(llx, lly, urx, ury, first);
            self.t3().glyph_num += 1;
            self.pdfnewdict(0, 0, 0);
            self.rt.fonts.t3.char_procs[cc] = self.objptr;
            self.pdfbeginstream();
            let w = self.t3().char_widths[cc] as i32;
            self.pdfprintreal(w, 2);
            self.pdf_printf(&format!(" 0 {} {} {} {} d1\n", llx, lly, urx, ury));
            if !is_null_glyph {
                self.pdf_printf(&format!("q\n{} 0 0 {} {} {} cm\nBI\n", cd.cwidth, cd.cheight, llx, lly));
                self.pdf_printf(&format!("/W {}\n/H {}\n", cd.cwidth, cd.cheight));
                self.pdf_puts(b"/IM true\n/BPC 1\n/D [1 0]\nID ");
                let cw = (cd.cwidth + 7) / 8;
                let rw = (cd.cwidth + 15) / 16;
                let mut row = 0usize;
                let get = |cd: &CharDesc, i: usize| cd.raster.get(i).copied().unwrap_or(0);
                for _ in 0..cd.cheight {
                    for _ in 0..rw - 1 {
                        let v = get(&cd, row);
                        self.pdfout((v / 256) as u8);
                        self.pdfout((v % 256) as u8);
                        row += 1;
                    }
                    let v = get(&cd, row);
                    self.pdfout((v / 256) as u8);
                    if 2 * rw == cw {
                        self.pdfout((v % 256) as u8);
                    }
                    row += 1;
                }
                self.pdf_puts(b"\nEI\nQ\n");
            }
            self.pdfendstream();
        }
        self.set_cur_file_name_none();
        true
    }

    /// writet3.c remove_duplicate_glyph_names
    fn remove_duplicate_glyph_names(&mut self, fe: usize, encname: &[u8]) {
        let mut seen: BTreeSet<Vec<u8>> = BTreeSet::new();
        for i in 0..256 {
            let Some(g) = self.rt.fonts.enc.fe[fe].glyph_names[i].clone() else { continue };
            if seen.contains(&g) {
                self.pdftex_warn(&format!("{}: duplicate glyph name at position {}: {}", bs(encname), i, bs(&g)));
                self.rt.fonts.enc.fe[fe].glyph_names[i] = None;
            } else {
                seen.insert(g);
            }
        }
    }

    /// writet3.c writet3
    pub fn writet3(&mut self, fm: Option<usize>, objnum: i32, f: i32) {
        const SCALE_STR: &[u8] = b"\\pdffontscale";
        {
            let t = self.t3();
            t.glyph_num = 0;
            t.image_used = false;
            t.char_procs = [0; 256];
            t.char_widths = [0.0; 256];
        }
        let encname = fm.and_then(|m| self.fm(m).encname.clone());
        let fe = match &encname {
            Some(e) => self.get_fe_entry(e),
            None => None,
        };
        if let Some(fe) = fe {
            self.remove_duplicate_glyph_names(fe, encname.as_deref().unwrap());
        }
        let n = *ix(self.fontname, f);
        let e = self.getnullstr();
        let pgc = self.maketexstring_u(b".pgc");
        self.packfilename(n, e, pgc);
        let s = self.makenamestring();
        let mut cn = self.tex_string(s);
        if let Some(z) = cn.iter().position(|&c| c == 0) {
            cn.truncate(z);
        }
        self.rt.pdfu.cur_file_name = Some(bs(&cn));
        self.t3().is_pk_font = false;
        match self.font_open_input(kpse::MISCFONTS_FORMAT) {
            None => {
                if !self.writepk(f) {
                    self.set_cur_file_name_none();
                    return;
                }
            }
            Some(file) => {
                self.t3().file = file;
                let nf = self.name_of_file();
                let mut m = b"<".to_vec();
                m.extend_from_slice(&nf);
                self.tex_printf_b(&m);
                self.t3_getline();
                let mut line = self.t3().line.clone();
                line.push(0);
                let sc = if self.t3_prefix(SCALE_STR) { scan_float(&line, SCALE_STR.len() + 1).map(|(v, _)| v) } else { None };
                match sc {
                    Some(v) if v > 0.0 && v <= 1000.0 => self.t3().font_scale = v,
                    _ => self.pdftex_fail("missing or invalid font scale"),
                }
                while !self.t3().file.feof() {
                    self.t3_write_glyph(f);
                }
            }
        }
        // write_font_dict:
        let bc = *ix(self.fontbc, f) as i32;
        let ec = *ix(self.fontec, f) as i32;
        let mut i = bc;
        while i <= ec {
            if self.pdfcharmarked(f, i) {
                break;
            }
            i += 1;
        }
        let first_char = i;
        let mut i = ec;
        while i > first_char {
            if self.pdfcharmarked(f, i) {
                break;
            }
            i -= 1;
        }
        let last_char = i;
        let glyph_names: Option<GlyphNames> = fe.map(|fe| self.rt.fonts.enc.fe[fe].glyph_names.clone());
        let tounicode_objnum = if self.fixedgentounicode > 0 && !*ix(self.pdffontnobuiltintounicode, f) && fe.is_some() {
            let tfm = self.fm(fm.unwrap()).tfm().to_vec();
            let name = self.rt.fonts.enc.fe[fe.unwrap()].name.clone();
            self.write_tounicode(glyph_names.as_ref().unwrap(), &tfm, Some(&name))
        } else {
            0
        };
        self.pdfbegindict(objnum, 1);
        self.pdf_puts(b"/Type /Font\n/Subtype /Type3\n");
        self.pdf_printf(&format!("/Name /F{}\n", f));
        let attr = *ix(self.pdffontattr, f);
        if attr != self.getnullstr() {
            self.pdfprint(attr);
            self.pdf_puts(b"\n");
        }
        if self.t3().is_pk_font {
            let s = self.get_pk_font_scale(f);
            self.pdf_puts(b"/FontMatrix [");
            self.pdfprintreal(s, 5);
            self.pdf_puts(b" 0 0 ");
            self.pdfprintreal(s, 5);
            self.pdf_puts(b" 0 0]\n");
        } else {
            let s = fmt_g(self.t3().font_scale as f64);
            self.pdf_printf(&format!("/FontMatrix [{} 0 0 {} 0 0]\n", s, s));
        }
        let t = &self.rt.fonts.t3;
        let s = format!("/{} [ {} {} {} {} ]\n", FONT_KEY[FONTBBOX1_CODE].0, t.b0, t.b1, t.b2, t.b3);
        self.pdf_printf(&s);
        let s = format!("/Resources << /ProcSet [ /PDF {}] >>\n", if self.t3().image_used { "/ImageB " } else { "" });
        self.pdf_printf(&s);
        self.pdf_printf(&format!("/FirstChar {}\n/LastChar {}\n", first_char, last_char));
        let wptr = self.pdfnewobjnum();
        let eptr = self.pdfnewobjnum();
        let cptr = self.pdfnewobjnum();
        self.pdf_printf(&format!("/Widths {} 0 R\n/Encoding {} 0 R\n/CharProcs {} 0 R\n", wptr, eptr, cptr));
        if tounicode_objnum != 0 {
            self.pdf_printf(&format!("/ToUnicode {} 0 R\n", tounicode_objnum));
        }
        self.pdfenddict();
        self.pdfbeginobj(wptr, 1);
        self.pdf_puts(b"[");
        if self.t3().is_pk_font {
            for i in first_char..=last_char {
                let w = self.t3().char_widths[i as usize] as i32;
                self.pdfprintreal(w, 2);
                self.pdf_puts(b" ");
            }
        } else {
            for i in first_char..=last_char {
                let w = self.t3().char_widths[i as usize] as i32;
                self.pdf_printf(&format!("{} ", w));
            }
        }
        self.pdf_puts(b"]\n");
        self.pdfendobj();
        let gn = |i: i32| -> Option<Vec<u8>> { glyph_names.as_ref().and_then(|g| g[i as usize].clone()) };
        self.pdfbegindict(eptr, 1);
        self.pdf_printf(&format!("/Type /Encoding\n/Differences [{}", first_char));
        let mut is_notdef;
        if self.t3().char_procs[first_char as usize] == 0 {
            self.pdf_printf("/.notdef");
            is_notdef = true;
        } else {
            match gn(first_char) {
                Some(g) => {
                    let mut s = b"/".to_vec();
                    s.extend_from_slice(&g);
                    self.pdf_printf_b(&s);
                }
                None => self.pdf_printf(&format!("/a{}", first_char)),
            }
            is_notdef = false;
        }
        for i in first_char + 1..=last_char {
            if self.t3().char_procs[i as usize] == 0 {
                if !is_notdef {
                    self.pdf_printf(&format!(" {}/.notdef", i));
                    is_notdef = true;
                }
            } else {
                if is_notdef {
                    self.pdf_printf(&format!(" {}", i));
                    is_notdef = false;
                }
                match gn(i) {
                    Some(g) => {
                        let mut s = b"/".to_vec();
                        s.extend_from_slice(&g);
                        self.pdf_printf_b(&s);
                    }
                    None => self.pdf_printf(&format!("/a{}", i)),
                }
            }
        }
        self.pdf_puts(b"]\n");
        self.pdfenddict();
        self.pdfbegindict(cptr, 1);
        for i in first_char..=last_char {
            let p = self.t3().char_procs[i as usize];
            if p != 0 {
                match gn(i) {
                    Some(g) => {
                        let mut s = b"/".to_vec();
                        s.extend_from_slice(&g);
                        s.extend_from_slice(format!(" {} 0 R\n", p).as_bytes());
                        self.pdf_printf_b(&s);
                    }
                    None => self.pdf_printf(&format!("/a{} {} 0 R\n", i, p)),
                }
            }
        }
        self.pdfenddict();
        self.t3().file = CFile::default();
        self.tex_printf(">");
        self.set_cur_file_name_none();
    }
}
