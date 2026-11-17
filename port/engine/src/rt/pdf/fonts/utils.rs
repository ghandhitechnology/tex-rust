//! The font part of pdftexdir/utils.c: make_subset_tag.

use crate::gen::globals::Globals;
use crate::rt::md5::Md5;

const SUBSET_TAG_LENGTH: usize = 6;

impl Globals {
    /// utils.c make_subset_tag: a 6-letter tag from the MD5 of the glyph names, the font
    /// name and the round counter j (hashed as the 4 little-endian bytes of an int).
    pub fn make_subset_tag(&mut self, fd: usize) {
        let (glyphs, fontname) = {
            let e = &self.rt.fonts.wf.fd[fd];
            assert!(e.subset_tag.is_none());
            let g: Vec<Vec<u8>> = e.gl_tree.as_ref().expect("gl_tree").iter().cloned().collect();
            (g, e.fontname.clone().expect("fontname"))
        };
        let mut j: i32 = 0;
        let mut tag;
        loop {
            let mut pms = Md5::new();
            for g in &glyphs {
                pms.update(g);
                pms.update(b" ");
            }
            pms.update(&fontname);
            pms.update(&j.to_le_bytes());
            let digest = pms.finish();
            let mut a = [0i32; SUBSET_TAG_LENGTH];
            for i in 0..13 {
                a[0] += digest[i] as i32;
            }
            for i in 1..SUBSET_TAG_LENGTH {
                a[i] = a[i - 1] - digest[i - 1] as i32 + digest[(i + 12) % 16] as i32;
            }
            tag = (0..SUBSET_TAG_LENGTH).map(|i| (a[i] % 26) as u8 + b'A').collect::<Vec<u8>>();
            j += 1;
            assert!(j < 100);
            if !self.rt.fonts.st_tree.contains(&tag) {
                break;
            }
        }
        self.rt.fonts.st_tree.insert(tag.clone());
        self.rt.fonts.wf.fd[fd].subset_tag = Some(tag);
        if j > 2 {
            self.pdftex_warn(&format!("\nmake_subset_tag(): subset-tag collision, resolved in round {}.\n", j));
        }
    }
}
