//! pdfTeX's C library (texk/web2c/pdftexdir/*.c), ported to Rust.

pub mod core;
pub mod fonts;
pub mod images;
pub mod parzip;
pub mod utils;
pub mod writezip;

use crate::gen::globals::Globals;

impl Globals {
    /// libpdffinish's per-module frees (fm_free, t1_free, enc_free, img_free, vf_free,
    /// epdf_free, ttf_free, sfd_free, glyph_unicode_free, zip_free).
    pub fn pdflib_free_all(&mut self) {
        crate::rt::phase::mark("pdflib_free_all");
        // This runs once, at the end of the job, right before the process exits. Freeing
        // the large font tables entry by entry (the glyph-to-Unicode tree from the format,
        // thousands of map entries) costs about a millisecond and has no effect on any
        // output, so they are released wholesale by leaking them to the process exit.
        {
            let f = &mut self.rt.fonts;
            std::mem::forget(f.tu.tree.take());
            std::mem::forget(std::mem::take(&mut f.map.tfm_tree));
            std::mem::forget(std::mem::take(&mut f.map.ps_tree));
            std::mem::forget(std::mem::take(&mut f.map.ff_tree));
            std::mem::forget(std::mem::take(&mut f.map.encname_tree));
            std::mem::forget(std::mem::take(&mut f.map.fm));
        }
        self.zip_free();
        self.fm_free();
        self.t1_free();
        self.enc_free();
        self.img_free();
        self.vf_free();
        self.epdf_free();
        self.ttf_free();
        self.sfd_free();
        self.glyph_unicode_free();
    }
}
