//! Port of pdftexdir/epdf.c: the font-descriptor side of PDF inclusion with
//! \pdfinclusioncopyfonts=0 (embedded Type 1 fonts of an included PDF are replaced by
//! pdfTeX's own copy of the font from the map file).
//!
//! These functions work on the font subsystem's structures (fm_entry/fd_entry arenas,
//! the map file lookup and writeenc.c's epdf_write_enc) in `rt/pdf/fonts`.

use crate::gen::globals::Globals;

mod with_fonts {
    use super::*;

    const STEMV_CODE: usize = 4;

    fn is_generic_space(c: u8) -> bool {
        c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' || c == 0x0c
    }

    impl Globals {
        /// mapfile.c lookup_fontmap
        pub fn epdf_lookup_fontmap(&mut self, ps_name: &[u8]) -> Option<usize> {
            self.lookup_fontmap(ps_name)
        }

        /// epdf.c is_subsetable
        pub fn is_subsetable(&mut self, fm: usize) -> bool {
            assert!(self.fm(fm).is_included());
            self.fm(fm).is_subsetted()
        }

        /// epdf.c epdf_create_fontdescriptor
        pub fn epdf_create_fontdescriptor(&mut self, fm: usize, stemv: i32) -> usize {
            let (ff, slant, extend) = {
                let e = self.fm(fm);
                (e.ff().to_vec(), e.slant, e.extend)
            };
            if let Some(fd) = self.lookup_fd_entry(&ff, slant, extend) {
                return fd;
            }
            self.fm_mut(fm).in_use = true;
            let fd = self.new_fd_entry();
            self.fd_mut(fd).fm = fm;
            self.register_fd_entry(fd);
            let n = self.pdfnewobjnum();
            let ps_name = self.fm(fm).ps_name.clone();
            assert!(ps_name.is_some());
            let e = self.fd_mut(fd);
            e.fd_objnum = n;
            e.fontname = ps_name;
            e.font_dim[STEMV_CODE].val = stemv;
            e.font_dim[STEMV_CODE].set = true;
            e.gl_tree = Some(Default::default());
            fd
        }

        /// epdf.c get_fd_objnum
        pub fn get_fd_objnum(&mut self, fd: usize) -> i32 {
            let n = self.fd(fd).fd_objnum;
            assert!(n != 0);
            n
        }

        /// epdf.c get_fn_objnum
        pub fn get_fn_objnum(&mut self, fd: usize) -> i32 {
            if self.fd(fd).fn_objnum == 0 {
                let n = self.pdfnewobjnum();
                self.fd_mut(fd).fn_objnum = n;
            }
            self.fd(fd).fn_objnum
        }

        /// epdf.c epdf_mark_glyphs: charset is "/a/b /c ..." (the CharSet string up to
        /// its first NUL)
        pub fn epdf_mark_glyphs(&mut self, fd: usize, charset: &[u8]) {
            let mut cs: Vec<u8> = charset.to_vec();
            cs.push(0);
            let mut start = 0usize;
            while is_generic_space(cs[start]) {
                start += 1;
            }
            let q = start + cs[start..].iter().position(|&c| c == 0).unwrap();
            let mut s = start + 1;
            while s < q {
                let mut p = s;
                while cs[p] != 0 && cs[p] != b'/' && !is_generic_space(cs[p]) {
                    p += 1;
                }
                let end = p;
                if is_generic_space(cs[p]) {
                    cs[p] = 0;
                    p += 1;
                    while is_generic_space(cs[p]) {
                        p += 1;
                    }
                }
                cs[p] = 0;
                let glyph = cs[s..end].to_vec();
                let tree = self.fd_mut(fd).gl_tree.get_or_insert_with(Default::default);
                tree.insert(glyph);
                s = p + 1;
            }
        }

        /// epdf.c embed_whole_font
        pub fn embed_whole_font(&mut self, fd: usize) {
            self.fd_mut(fd).all_glyphs = true;
        }

        /// writeenc.c epdf_write_enc (None = the `notdef` pointer, not written)
        pub fn epdf_write_encoding(&mut self, glyph_names: &[Option<Vec<u8>>], objnum: i32) {
            let names: Vec<Option<Vec<u8>>> = glyph_names.to_vec();
            self.epdf_write_enc(&names, objnum);
        }
    }
}

impl Globals {
    /// epdf.c epdf_free
    pub fn epdf_free(&mut self) {
        self.epdf_check_mem();
    }
}
