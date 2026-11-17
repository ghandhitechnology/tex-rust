//! Small pieces of pdfTeX's C side: pdftex.h macros, avlstuff.c object trees, and the
//! kpathsea entry points the Pascal calls directly.

use std::collections::BTreeMap;

use crate::gen::globals::Globals;
use crate::rt::*;

#[derive(Default)]
pub struct CoreState {
    /// avlstuff.c PdfObjTree[t]: int0 -> objptr, first insertion wins (avl_probe).
    pub obj_tree: Vec<Option<BTreeMap<ObjKey, i32>>>,
}

/// avlstuff.c compare_info: objects named by a string (negative int0 = -string number)
/// compare by string length, then by content; all other int0 values compare as integers,
/// and every named object sorts before the numbered ones.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ObjKey {
    Named(usize, Vec<u8>),
    Num(i32),
}

impl Globals {
    fn obj_key(&self, int0: i32) -> ObjKey {
        if int0 < 0 {
            let b = self.tex_string(-int0);
            ObjKey::Named(b.len(), b)
        } else {
            ObjKey::Num(int0)
        }
    }

    #[inline]
    fn code_from(&self, base: Ptr<i32>, f: i32, c: i32, dflt: i32) -> i32 {
        let b = *ix(base, f);
        if b == 0 {
            dflt
        } else {
            *ix(self.pdfmem, b + c)
        }
    }

    // pdftex.h
    pub fn getlpcode(&mut self, f: i32, c: i32) -> i32 {
        self.code_from(self.pdffontlpbase, f, c, 0)
    }
    pub fn getrpcode(&mut self, f: i32, c: i32) -> i32 {
        self.code_from(self.pdffontrpbase, f, c, 0)
    }
    pub fn getefcode(&mut self, f: i32, c: i32) -> i32 {
        self.code_from(self.pdffontefbase, f, c, 1000)
    }
    pub fn getknbscode(&mut self, f: i32, c: i32) -> i32 {
        self.code_from(self.pdffontknbsbase, f, c, 0)
    }
    pub fn getstbscode(&mut self, f: i32, c: i32) -> i32 {
        self.code_from(self.pdffontstbsbase, f, c, 0)
    }
    pub fn getshbscode(&mut self, f: i32, c: i32) -> i32 {
        self.code_from(self.pdffontshbsbase, f, c, 0)
    }
    pub fn getknbccode(&mut self, f: i32, c: i32) -> i32 {
        self.code_from(self.pdffontknbcbase, f, c, 0)
    }
    pub fn getknaccode(&mut self, f: i32, c: i32) -> i32 {
        self.code_from(self.pdffontknacbase, f, c, 0)
    }

    /// pdftex.h pdfmarkchar: pdfcharused[f][c/8] |= (1<<(c%8))
    pub fn pdfmarkchar(&mut self, f: i32, c: i32) {
        let row = ix(self.pdfcharused, f);
        row[(c / 8) as usize] |= (1 << (c % 8)) as u8;
    }
    pub fn pdfcharmarked(&mut self, f: i32, c: i32) -> bool {
        let row = ix(self.pdfcharused, f);
        row[(c / 8) as usize] & (1 << (c % 8)) as u8 != 0
    }

    /// pdftex.h writepdf: fwrite(&pdfbuf[a], 1, b-a+1, pdffile)
    pub fn writepdf(&mut self, a: i32, b: i32) {
        let n = (b - a + 1).max(0) as usize;
        let data = slice(self.pdfbuf, a, n).to_vec();
        if self.parzip_write(&data) {
            return;
        }
        let f = self.pdffile;
        self.rt.files.write(f, &data);
    }

    pub fn pdfassert(&mut self, cond: bool) {
        if !cond {
            texmfmp::fatal("pdfTeX assertion failed");
        }
    }

    // ------------------------------------------------------------------ avlstuff.c
    pub fn avlputobj(&mut self, objptr: i32, t: i32) {
        let int0 = ix(self.objtab, objptr).int0;
        let key = self.obj_key(int0);
        let trees = &mut self.rt.core.obj_tree;
        if trees.len() <= t as usize {
            trees.resize(t as usize + 1, None);
        }
        trees[t as usize].get_or_insert_with(BTreeMap::new).entry(key).or_insert(objptr);
    }

    pub fn avlfindobj(&mut self, t: i32, i: i32, byname: i32) -> i32 {
        let key = self.obj_key(if byname > 0 { -i } else { i });
        match self.rt.core.obj_tree.get(t as usize) {
            Some(Some(tree)) => *tree.get(&key).unwrap_or(&0),
            _ => 0,
        }
    }

    // ------------------------------------------------------------------ kpathsea entry points
    pub fn ISDIRSEP(&mut self, c: i32) -> bool {
        c == b'/' as i32 || c == b'\\' as i32
    }
    pub fn kpseinnameok(&mut self, name: Ptr<u8>) -> bool {
        let n = bytes_to_string(name.cstr_bytes());
        self.rt.kpse().in_name_ok(&n)
    }
    pub fn kpseoutnameok(&mut self, name: Ptr<u8>) -> bool {
        let n = bytes_to_string(name.cstr_bytes());
        self.rt.kpse().out_name_ok(&n)
    }
    pub fn kpseinitprog(&mut self, prefix: &[u8], dpi: i32, mode: Ptr<u8>, fallback: Ptr<u8>) {
        let p = bytes_to_string(prefix);
        let m = if mode.is_null() { None } else { Some(bytes_to_string(mode.cstr_bytes())) };
        let f = if fallback.is_null() { None } else { Some(bytes_to_string(fallback.cstr_bytes())) };
        self.rt.kpse().init_prog(&p, dpi, m.as_deref(), f.as_deref());
    }
    pub fn kpsesetprogramenabled(&mut self, fmt: i32, value: i32, level: i32) {
        self.rt.kpse().set_program_enabled(fmt, value != 0, level);
    }

    // ------------------------------------------------------------------ C string helpers
    pub fn strlen(&mut self, s: Ptr<u8>) -> i32 {
        s.cstr_bytes().len() as i32
    }
    pub fn strcmp(&mut self, a: Ptr<u8>, b: Ptr<u8>) -> i32 {
        match a.cstr_bytes().cmp(b.cstr_bytes()) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }

    // ------------------------------------------------------------------ format dump hooks
    // writeimg.c dumpimagemeta/undumpimagemeta: rt/pdf/images/mod.rs
}


impl Globals {
    /// tex.ch sets kpse_make_tex_discard_errors (batch mode); kpathsea reads it.
    pub fn sync_kpse_discard_errors(&mut self) {
        let v = self.kpsemaketexdiscarderrors != 0;
        self.rt.kpse().k.set_make_tex_discard_errors(v);
    }
}
