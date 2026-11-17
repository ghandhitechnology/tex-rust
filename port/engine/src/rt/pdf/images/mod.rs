//! pdfTeX's image inclusion (texk/web2c/pdftexdir: writeimg.c, writepng.c, writejpg.c,
//! writejbig2.c, pdftoepdf.cc, epdf.c, image.h), ported to Rust.
//!
//! `image_array` is `ImageState::array` (image_ptr is its length); an image's
//! `image_struct` union is `ImageStruct`. PNG files are decoded by `libpng` (a port of
//! the libpng 1.6.55 read path), PDF files are read with the pdfread crate (xpdf 4.06).

pub mod epdf;
pub mod libpng;
pub mod pdftoepdf;
pub mod writejbig2;
pub mod writejpg;
pub mod writepng;

use crate::gen::globals::Globals;
use crate::rt::*;

// image.h
pub const IMAGE_TYPE_NONE: i32 = 0;
pub const IMAGE_TYPE_PDF: i32 = 1;
pub const IMAGE_TYPE_PNG: i32 = 2;
pub const IMAGE_TYPE_JPG: i32 = 3;
#[allow(dead_code)]
pub const IMAGE_TYPE_TIF: i32 = 4;
pub const IMAGE_TYPE_JBIG2: i32 = 5;

pub const IMAGE_COLOR_B: i32 = 1;
pub const IMAGE_COLOR_C: i32 = 2;
pub const IMAGE_COLOR_I: i32 = 4;

const SMALL_BUF_SIZE: i32 = 256;

/// image.h pdf_image_struct
#[derive(Default, Clone)]
pub struct PdfImage {
    pub orig_x: i32,
    pub orig_y: i32,
    pub selected_page: i32,
    pub page_box: i32,
    /// `void *doc`: id of the PdfDocument record (pdftoepdf.cc)
    pub doc: usize,
}

/// image.h JPG_IMAGE_INFO
pub struct JpgImageInfo {
    pub color_space: i32,
    pub bits_per_component: u8,
    /// JPG_UINT32 = unsigned long (32 bits on Windows)
    pub length: u32,
    pub file: CFile,
}

/// image.h JBIG2_IMAGE_INFO
#[derive(Default, Clone)]
pub struct Jbig2ImageInfo {
    pub selected_page: i32,
}

/// image.h image_entry::image_struct
#[derive(Default)]
pub enum ImageStruct {
    #[default]
    None,
    Pdf(PdfImage),
    Png(Box<libpng::Png>),
    Jpg(Box<JpgImageInfo>),
    Jbig2(Jbig2ImageInfo),
}

/// image.h image_entry
#[derive(Default)]
pub struct ImageEntry {
    /// the file name as found by kpathsea (bytes)
    pub image_name: Option<Vec<u8>>,
    pub image_type: i32,
    pub color_type: i32,
    pub width: i32,
    pub height: i32,
    pub rotate: i32,
    pub x_res: i32,
    pub y_res: i32,
    pub num_pages: i32,
    pub colorspace_ref: i32,
    pub group_ref: i32,
    pub st: ImageStruct,
}

/// writeimg.c's globals (image_array, image_limit, the epdf_* exchange variables) and the
/// static state of the other image C files.
#[derive(Default)]
pub struct ImageState {
    pub array: Vec<ImageEntry>,
    /// image_array != NULL
    pub allocated: bool,
    pub limit: i32,
    pub epdf_width: f32,
    pub epdf_height: f32,
    pub epdf_orig_x: f32,
    pub epdf_orig_y: f32,
    pub epdf_rotate: f32,
    pub epdf_selected_page: i32,
    pub epdf_num_pages: i32,
    pub epdf_page_box: i32,
    pub epdf_doc: usize,
    pub epdf_has_page_group: i32,
    pub png: writepng::PngState,
    pub jbig2: writejbig2::Jbig2State,
    pub pdf: pdftoepdf::EpdfState,
}

/// A C `FILE*` opened "rb" over the whole file: getc/feof/fseek/ftell/fread semantics.
#[derive(Default)]
pub struct CFile {
    pub data: Vec<u8>,
    pub pos: u64,
    pub eof: bool,
}

pub const SEEK_SET: i32 = 0;
pub const SEEK_CUR: i32 = 1;
pub const SEEK_END: i32 = 2;

impl CFile {
    pub fn new(data: Vec<u8>) -> CFile {
        CFile { data, pos: 0, eof: false }
    }
    pub fn getc(&mut self) -> i32 {
        if (self.pos as usize) < self.data.len() && self.pos < usize::MAX as u64 {
            let c = self.data[self.pos as usize];
            self.pos += 1;
            c as i32
        } else {
            self.eof = true;
            -1
        }
    }
    pub fn feof(&self) -> bool {
        self.eof
    }
    /// fseek: returns false (as fseek's -1) for a negative resulting position.
    pub fn seek(&mut self, off: i64, whence: i32) -> bool {
        let base = match whence {
            SEEK_SET => 0i64,
            SEEK_CUR => self.pos as i64,
            _ => self.data.len() as i64,
        };
        let p = base + off;
        if p < 0 {
            return false;
        }
        self.pos = p as u64;
        self.eof = false;
        true
    }
    pub fn tell(&self) -> i64 {
        self.pos as i64
    }
    /// fread(buf, 1, buf.len(), f): copies what is available, returns the count.
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let start = (self.pos as usize).min(self.data.len());
        let n = buf.len().min(self.data.len() - start);
        buf[..n].copy_from_slice(&self.data[start..start + n]);
        self.pos += n as u64;
        if n < buf.len() {
            self.eof = true;
        }
        n
    }
}

/// kpathsea xfseek/xfseeko: FATAL_PERROR when fseek fails.
pub fn xfseek(f: &mut CFile, off: i64, whence: i32, name: &str) {
    if !f.seek(off, whence) {
        texmfmp::fatal_perror(name);
    }
}

impl Globals {
    // ------------------------------------------------------------------ helpers
    pub fn img(&mut self, img: i32) -> &mut ImageEntry {
        &mut self.rt.images.array[img as usize]
    }

    /// kpathsea xfopen(name, FOPEN_RBIN_MODE): FATAL_PERROR on failure.
    pub fn img_xfopen(&mut self, name: &[u8]) -> CFile {
        let p = self.os_path(name);
        match std::fs::read(p) {
            Ok(d) => CFile::new(d),
            Err(_) => texmfmp::fatal_perror(&bytes_to_string(name)),
        }
    }

    fn img_name(&mut self, img: i32) -> Vec<u8> {
        self.img(img).image_name.clone().unwrap_or_default()
    }

    /// pdfout() for every byte of `b` (same buffer flushes as a byte loop).
    pub fn pdfout_bytes(&mut self, mut b: &[u8]) {
        while !b.is_empty() {
            self.pdfroom(1);
            let space = (self.pdfbufsize - self.pdfptr) as usize;
            let n = space.min(b.len());
            slice(self.pdfbuf, self.pdfptr, n).copy_from_slice(&b[..n]);
            self.pdfptr += n as i32;
            b = &b[n..];
        }
    }

    /// bp2int(p): round(p*(onehundredbp/100.0)), with web2c's round = zround.
    fn bp2int(&self, p: f32) -> i32 {
        zround(p as f64 * (self.onehundredbp as f64 / 100.0))
    }

    /// writeimg.c new_image_entry (alloc_array(image, 1, SMALL_BUF_SIZE))
    fn new_image_entry(&mut self) -> i32 {
        let st = &mut self.rt.images;
        if !st.allocated {
            st.limit = SMALL_BUF_SIZE;
            st.allocated = true;
        } else if (st.array.len() as u32).wrapping_add(1) > st.limit as u32 {
            st.limit = st.limit.wrapping_mul(2);
            if (st.array.len() as u32 + 1) > st.limit as u32 {
                st.limit = st.array.len() as i32 + 1;
            }
            if st.limit as u32 > i32::MAX as u32 {
                self.pdftex_fail("image_array exceeds size limit");
            }
        }
        let st = &mut self.rt.images;
        st.array.push(ImageEntry::default());
        (st.array.len() - 1) as i32
    }

    // ------------------------------------------------------------------ getters
    #[allow(dead_code)] // writeimg.c imagecolor: exported by the C file, unused by pdftex.web
    pub fn imagecolor(&mut self, img: i32) -> i32 {
        self.img(img).color_type
    }
    pub fn imagewidth(&mut self, img: i32) -> i32 {
        self.img(img).width
    }
    pub fn imageheight(&mut self, img: i32) -> i32 {
        self.img(img).height
    }
    pub fn imagerotate(&mut self, img: i32) -> i32 {
        self.img(img).rotate
    }
    pub fn imagexres(&mut self, img: i32) -> i32 {
        self.img(img).x_res
    }
    pub fn imageyres(&mut self, img: i32) -> i32 {
        self.img(img).y_res
    }
    pub fn ispdfimage(&mut self, img: i32) -> bool {
        self.img(img).image_type == IMAGE_TYPE_PDF
    }
    pub fn ispngimage(&mut self, img: i32) -> bool {
        self.img(img).image_type == IMAGE_TYPE_PNG
    }
    pub fn checkimageb(&mut self, procset: i32) -> bool {
        procset & IMAGE_COLOR_B != 0
    }
    pub fn checkimagec(&mut self, procset: i32) -> bool {
        procset & IMAGE_COLOR_C != 0
    }
    pub fn checkimagei(&mut self, procset: i32) -> bool {
        procset & IMAGE_COLOR_I != 0
    }
    pub fn updateimageprocset(&mut self, img: i32) {
        self.pdfimageprocset |= self.img(img).color_type;
    }
    pub fn epdforigx(&mut self, img: i32) -> i32 {
        match &self.img(img).st {
            ImageStruct::Pdf(p) => p.orig_x,
            _ => 0,
        }
    }
    pub fn epdforigy(&mut self, img: i32) -> i32 {
        match &self.img(img).st {
            ImageStruct::Pdf(p) => p.orig_y,
            _ => 0,
        }
    }
    pub fn imagepages(&mut self, img: i32) -> i32 {
        self.img(img).num_pages
    }
    pub fn imagecolordepth(&mut self, img: i32) -> i32 {
        let t = self.img(img).image_type;
        match t {
            IMAGE_TYPE_PNG => match &self.img(img).st {
                ImageStruct::Png(p) => p.info.bit_depth as i32,
                _ => 0,
            },
            IMAGE_TYPE_JPG => match &self.img(img).st {
                ImageStruct::Jpg(j) => j.bits_per_component as i32,
                _ => 0,
            },
            IMAGE_TYPE_JBIG2 | IMAGE_TYPE_PDF => 0,
            _ => self.pdftex_fail("unknown type of image"),
        }
    }
    pub fn getimagegroupref(&mut self, img: i32) -> i32 {
        self.img(img).group_ref
    }
    pub fn setimagegroupref(&mut self, img: i32, value: i32) {
        self.img(img).group_ref = value;
    }

    // ------------------------------------------------------------------ type detection
    /// writeimg.c checktypebyheader
    fn checktypebyheader(&mut self, img: i32) {
        if self.img(img).image_type != IMAGE_TYPE_NONE {
            return;
        }
        let name = self.img_name(img);
        let mut file = self.img_xfopen(&name);
        let mut header = [0u8; 8];
        for h in header.iter_mut() {
            *h = file.getc() as u8;
            if file.feof() {
                self.pdftex_fail("reading image file failed");
            }
        }
        let t = if header[..2] == *b"\xFF\xD8" {
            IMAGE_TYPE_JPG
        } else if header == *b"\x89PNG\r\n\x1A\n" {
            IMAGE_TYPE_PNG
        } else if header == *b"\x97\x4A\x42\x32\x0D\x0A\x1A\x0A" {
            IMAGE_TYPE_JBIG2
        } else if header[..7] == *b"%PDF-1." {
            IMAGE_TYPE_PDF
        } else {
            IMAGE_TYPE_NONE
        };
        self.img(img).image_type = t;
    }

    /// writeimg.c checktypebyextension (strrchr on the whole found path)
    fn checktypebyextension(&mut self, img: i32) {
        if self.img(img).image_type != IMAGE_TYPE_NONE {
            return;
        }
        let name = self.rt.pdfu.cur_file_name.clone().unwrap_or_default();
        let t = match name.rfind('.') {
            None => IMAGE_TYPE_NONE,
            Some(i) => {
                let suf = name[i..].to_ascii_lowercase();
                match suf.as_str() {
                    ".png" => IMAGE_TYPE_PNG,
                    ".jpg" | ".jpeg" => IMAGE_TYPE_JPG,
                    ".jbig2" | ".jb2" => IMAGE_TYPE_JBIG2,
                    ".pdf" => IMAGE_TYPE_PDF,
                    _ => IMAGE_TYPE_NONE,
                }
            }
        };
        self.img(img).image_type = t;
    }

    // ------------------------------------------------------------------ readimage
    #[allow(clippy::too_many_arguments)]
    pub fn readimage(
        &mut self,
        s: i32,
        mut page_num: i32,
        page_name: i32,
        colorspace: i32,
        pagebox: i32,
        pdfmajorversion: i32,
        pdfminorversion: i32,
        pdfinclusionerrorlevel: i32,
    ) -> i32 {
        let img = self.new_image_entry();
        self.img(img).colorspace_ref = colorspace;
        let dest = if page_name != 0 { Some(self.tex_string(page_name)) } else { None };
        let found = self.find_input_file(s);
        self.rt.pdfu.cur_file_name = found.as_deref().map(bytes_to_string);
        self.img(img).image_name = found.clone();
        let Some(name) = found else {
            let raw = bytes_to_string(&self.tex_string(s));
            self.pdftex_fail(&format!("cannot find image file {}", raw));
        };
        self.recorder_record_input(&name);
        self.checktypebyheader(img);
        self.checktypebyextension(img);
        match self.img(img).image_type {
            IMAGE_TYPE_PDF => {
                self.img(img).st = ImageStruct::Pdf(PdfImage { page_box: pagebox, ..Default::default() });
                page_num = self.read_pdf_info(
                    &name,
                    dest.as_deref(),
                    page_num,
                    pagebox,
                    pdfmajorversion,
                    pdfminorversion,
                    pdfinclusionerrorlevel,
                );
                let e = &self.rt.images;
                let (w, h, rot, np, ox, oy, doc, grp) = (
                    e.epdf_width,
                    e.epdf_height,
                    e.epdf_rotate,
                    e.epdf_num_pages,
                    e.epdf_orig_x,
                    e.epdf_orig_y,
                    e.epdf_doc,
                    e.epdf_has_page_group,
                );
                let (w, h, ox, oy) = (self.bp2int(w), self.bp2int(h), self.bp2int(ox), self.bp2int(oy));
                let ent = self.img(img);
                ent.width = w;
                ent.height = h;
                ent.rotate = rot as i32;
                ent.num_pages = np;
                if let ImageStruct::Pdf(p) = &mut ent.st {
                    p.orig_x = ox;
                    p.orig_y = oy;
                    p.selected_page = page_num;
                    p.doc = doc;
                }
                ent.group_ref = if grp == 1 { -1 } else { 0 };
            }
            IMAGE_TYPE_PNG => {
                self.img(img).num_pages = 1;
                self.read_png_info(img);
            }
            IMAGE_TYPE_JPG => {
                self.img(img).st = ImageStruct::Jpg(Box::new(JpgImageInfo {
                    color_space: 0,
                    bits_per_component: 0,
                    length: 0,
                    file: CFile::default(),
                }));
                self.img(img).num_pages = 1;
                self.read_jpg_info(img);
            }
            IMAGE_TYPE_JBIG2 => {
                if pdfmajorversion == 1 && pdfminorversion < 4 {
                    self.pdftex_fail(&format!(
                        "JBIG2 images only possible with at least PDF 1.4; you are generating PDF 1.{}",
                        pdfminorversion
                    ));
                }
                let ent = self.img(img);
                ent.st = ImageStruct::Jbig2(Jbig2ImageInfo { selected_page: page_num });
                ent.image_type = IMAGE_TYPE_JBIG2;
                self.read_jbig2_info(img);
            }
            _ => self.pdftex_fail("unknown type of image"),
        }
        self.rt.pdfu.cur_file_name = None;
        img
    }

    pub fn writeimage(&mut self, img: i32) {
        let name = self.img(img).image_name.clone();
        self.rt.pdfu.cur_file_name = name.as_deref().map(bytes_to_string);
        let mut msg = b" <".to_vec();
        msg.extend_from_slice(name.as_deref().unwrap_or(b"(null)"));
        self.tex_printf_b(&msg);
        match self.img(img).image_type {
            IMAGE_TYPE_PNG => self.write_png(img),
            IMAGE_TYPE_JPG => self.write_jpg(img),
            IMAGE_TYPE_JBIG2 => self.write_jbig2(img),
            IMAGE_TYPE_PDF => {
                let (doc, sel, pb) = match &self.img(img).st {
                    ImageStruct::Pdf(p) => (p.doc, p.selected_page, p.page_box),
                    _ => (0, 0, 0),
                };
                let e = &mut self.rt.images;
                e.epdf_doc = doc;
                e.epdf_selected_page = sel;
                e.epdf_page_box = pb;
                self.write_epdf();
            }
            _ => self.pdftex_fail("unknown type of image"),
        }
        self.tex_printf(">");
        self.rt.pdfu.cur_file_name = None;
    }

    pub fn deleteimage(&mut self, img: i32) {
        if self.iniversion {
            return; // The image may be \dump{}ed to a format
        }
        match self.img(img).image_type {
            IMAGE_TYPE_PDF => {
                let doc = match &self.img(img).st {
                    ImageStruct::Pdf(p) => p.doc,
                    _ => 0,
                };
                self.rt.images.epdf_doc = doc;
                self.epdf_delete();
            }
            IMAGE_TYPE_PNG | IMAGE_TYPE_JPG => {
                // xfclose / png_destroy_read_struct
                self.img(img).st = ImageStruct::None;
            }
            IMAGE_TYPE_JBIG2 => {}
            _ => self.pdftex_fail("unknown type of image"),
        }
        self.img(img).image_name = None;
    }

    /// writeimg.c img_free
    pub fn img_free(&mut self) {
        self.rt.images.array = Vec::new();
    }

    // ------------------------------------------------------------------ format (un)dumping
    fn dump_charptr(&mut self, s: Option<&[u8]>) {
        match s {
            Some(b) => {
                self.dump_int(b.len() as i32 + 1);
                let mut v = b.to_vec();
                v.push(0);
                self.dump_things(v.as_mut_ptr(), v.len() as i32);
            }
            None => self.dump_int(0),
        }
    }

    fn undump_charptr(&mut self) -> Option<Vec<u8>> {
        let x = self.undump_int();
        if x > 0 {
            let mut v = vec![0u8; x as usize];
            self.undump_things(v.as_mut_ptr(), x);
            let end = v.iter().position(|&c| c == 0).unwrap_or(v.len());
            v.truncate(end);
            Some(v)
        } else {
            None
        }
    }

    /// writeimg.c dumpimagemeta
    pub fn dumpimagemeta(&mut self) {
        let lim = self.rt.images.limit;
        self.dump_int(lim);
        let cur_image = self.rt.images.array.len() as i32;
        self.dump_int(cur_image);
        for img in 0..cur_image {
            let name = self.img(img).image_name.clone();
            self.dump_charptr(name.as_deref());
            let e = self.img(img);
            let vals = [
                e.image_type,
                e.color_type,
                e.width,
                e.height,
                e.x_res,
                e.y_res,
                e.num_pages,
                e.colorspace_ref,
                e.group_ref,
            ];
            let extra: Vec<i32> = match &e.st {
                ImageStruct::Pdf(p) if e.image_type == IMAGE_TYPE_PDF => vec![p.page_box, p.selected_page],
                ImageStruct::Jbig2(j) if e.image_type == IMAGE_TYPE_JBIG2 => vec![j.selected_page],
                _ => vec![],
            };
            for v in vals {
                self.dump_int(v);
            }
            for v in extra {
                self.dump_int(v);
            }
        }
    }

    /// writeimg.c undumpimagemeta
    pub fn undumpimagemeta(&mut self, pdfmajorversion: i32, pdfminorversion: i32, pdfinclusionerrorlevel: i32) {
        let limit = self.undump_int();
        let st = &mut self.rt.images;
        st.limit = limit;
        st.allocated = true;
        st.array = Vec::new();
        let cur_image = self.undump_int();
        for _ in 0..cur_image {
            self.rt.images.array.push(ImageEntry::default());
        }
        for img in 0..cur_image {
            let name = self.undump_charptr();
            self.img(img).image_name = name.clone();
            let mut v = [0i32; 9];
            for x in v.iter_mut() {
                *x = self.undump_int();
            }
            let e = self.img(img);
            e.image_type = v[0];
            e.color_type = v[1];
            e.width = v[2];
            e.height = v[3];
            e.x_res = v[4];
            e.y_res = v[5];
            e.num_pages = v[6];
            e.colorspace_ref = v[7];
            e.group_ref = v[8];
            let name = name.unwrap_or_default();
            if self.rt.kpse().find_file(&name, kpse::TEX_FORMAT, true).is_none() {
                self.pdftex_fail(&format!("cannot find image file {}", bytes_to_string(&name)));
            }
            match self.img(img).image_type {
                IMAGE_TYPE_PDF => {
                    let page_box = self.undump_int();
                    let selected_page = self.undump_int();
                    self.img(img).st = ImageStruct::Pdf(PdfImage { page_box, selected_page, ..Default::default() });
                    self.read_pdf_info(
                        &name,
                        None,
                        selected_page,
                        page_box,
                        pdfmajorversion,
                        pdfminorversion,
                        pdfinclusionerrorlevel,
                    );
                    let e = &self.rt.images;
                    let (w, h, np, ox, oy, doc) =
                        (e.epdf_width, e.epdf_height, e.epdf_num_pages, e.epdf_orig_x, e.epdf_orig_y, e.epdf_doc);
                    let (w, h, ox, oy) = (self.bp2int(w), self.bp2int(h), self.bp2int(ox), self.bp2int(oy));
                    let ent = self.img(img);
                    ent.width = w;
                    ent.height = h;
                    ent.num_pages = np;
                    if let ImageStruct::Pdf(p) = &mut ent.st {
                        p.orig_x = ox;
                        p.orig_y = oy;
                        p.doc = doc;
                    }
                }
                IMAGE_TYPE_PNG => {
                    self.img(img).num_pages = 1;
                    self.read_png_info(img);
                }
                IMAGE_TYPE_JPG => {
                    self.img(img).st = ImageStruct::Jpg(Box::new(JpgImageInfo {
                        color_space: 0,
                        bits_per_component: 0,
                        length: 0,
                        file: CFile::default(),
                    }));
                    self.img(img).num_pages = 1;
                    self.read_jpg_info(img);
                }
                IMAGE_TYPE_JBIG2 => {
                    if pdfmajorversion == 1 && pdfminorversion < 4 {
                        self.pdftex_fail(&format!(
                            "JBIG2 images only possible with at least PDF 1.4; you are generating PDF 1.{}",
                            pdfminorversion
                        ));
                    }
                    let selected_page = self.undump_int();
                    let ent = self.img(img);
                    ent.st = ImageStruct::Jbig2(Jbig2ImageInfo { selected_page });
                    ent.image_type = IMAGE_TYPE_JBIG2;
                    self.read_jbig2_info(img);
                }
                _ => self.pdftex_fail("unknown type of image"),
            }
        }
    }
}
