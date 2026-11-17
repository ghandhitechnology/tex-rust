//! Port of pdftexdir/writeenc.c: encoding vectors (fe_entry) and /Encoding objects.

use std::collections::BTreeMap;

use super::*;
use crate::gen::globals::Globals;

/// writeenc.c static state: fe_tree (by name) over the fe arena.
#[derive(Default)]
pub struct EncState {
    pub fe: Vec<FeEntry>,
    pub fe_tree: BTreeMap<Vec<u8>, usize>,
}

impl Globals {
    /// writeenc.c get_fe_entry (load_enc_file fails hard if the file is missing).
    pub fn get_fe_entry(&mut self, s: &[u8]) -> Option<usize> {
        if let Some(&fe) = self.rt.fonts.enc.fe_tree.get(s) {
            return Some(fe);
        }
        let gl = self.load_enc_file(s)?;
        let e = &mut self.rt.fonts.enc;
        e.fe.push(FeEntry { fe_objnum: 0, name: s.to_vec(), glyph_names: gl, tx_tree: None });
        let i = e.fe.len() - 1;
        e.fe_tree.insert(s.to_vec(), i);
        Some(i)
    }

    /// writeenc.c epdf_write_enc (for the epdf/pdftoepdf port)
    #[allow(dead_code)]
    pub fn epdf_write_enc(&mut self, glyph_names: &GlyphNames, fe_objnum: i32) {
        self.pdfbegindict(fe_objnum, 1);
        self.pdf_puts(b"/Type /Encoding\n");
        self.pdf_puts(b"/Differences [");
        let mut i_old: i32 = -2;
        for i in 0..256i32 {
            if let Some(g) = &glyph_names[i as usize] {
                let mut s = Vec::new();
                if i == i_old + 1 {
                    s.push(b'/');
                } else if i_old == -2 {
                    s.extend_from_slice(format!("{}/", i).as_bytes());
                } else {
                    s.extend_from_slice(format!(" {}/", i).as_bytes());
                }
                s.extend_from_slice(g);
                self.pdf_printf_b(&s);
                i_old = i;
            }
        }
        self.pdf_puts(b"]\n");
        self.pdfenddict();
    }

    /// writeenc.c write_enc
    fn write_enc(&mut self, fe: usize) {
        let objnum = self.rt.fonts.enc.fe[fe].fe_objnum;
        let codes: Vec<i32> = self.rt.fonts.enc.fe[fe].tx_tree.as_ref().expect("tx_tree").iter().copied().collect();
        self.pdfbegindict(objnum, 1);
        self.pdf_puts(b"/Type /Encoding\n");
        self.pdf_puts(b"/Differences [");
        let mut i_old: i32 = -2;
        for p in codes {
            let g = gname(&self.rt.fonts.enc.fe[fe].glyph_names[p as usize]).to_vec();
            let mut s = Vec::new();
            if p == i_old + 1 {
                s.push(b'/');
            } else if i_old == -2 {
                s.extend_from_slice(format!("{}/", p).as_bytes());
            } else {
                s.extend_from_slice(format!(" {}/", p).as_bytes());
            }
            s.extend_from_slice(&g);
            self.pdf_printf_b(&s);
            i_old = p;
        }
        self.pdf_puts(b"]\n");
        self.pdfenddict();
    }

    /// writeenc.c write_fontencodings
    pub fn write_fontencodings(&mut self) {
        let list: Vec<usize> = self.rt.fonts.enc.fe_tree.values().copied().collect();
        for fe in list {
            if self.rt.fonts.enc.fe[fe].fe_objnum != 0 {
                self.write_enc(fe);
            }
        }
    }

    /// writeenc.c enc_free
    pub fn enc_free(&mut self) {
        self.rt.fonts.enc.fe_tree.clear();
    }
}
