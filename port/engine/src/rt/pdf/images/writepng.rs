//! Port of pdftexdir/writepng.c: PNG images as /Image XObjects, decoded through the libpng
//! port, or copied without decoding ("PNG copy") when the IDAT data can be used as a
//! FlateDecode stream with the PNG predictors.

use super::libpng::*;
use super::{CFile, ImageStruct, IMAGE_COLOR_B, IMAGE_COLOR_C, IMAGE_COLOR_I, SEEK_CUR, SEEK_SET};
use crate::gen::globals::Globals;
use crate::rt::*;

/// writepng.c static state
#[derive(Default)]
pub struct PngState {
    transparent_page_group: i32,
    last_png_needs_page_group: bool,
    transparent_page_group_was_written: bool,
}

const SPNG_CHUNK_IDAT: i32 = 0x49444154;
const SPNG_CHUNK_IEND: i32 = 0x49454E44;

/// Which bytes of a decoded row go to the image stream and which to the soft mask.
#[derive(Clone, Copy, PartialEq)]
enum PixelSplit {
    Simple,
    Gray8,
    Gray16,
    Rgb8,
    Rgb16,
}

impl PixelSplit {
    /// true: pdfbuf, false: smask (j is the byte index within the current chunk)
    #[inline]
    fn to_image(self, j: usize) -> bool {
        match self {
            PixelSplit::Simple => true,
            PixelSplit::Gray16 => j % 4 == 0 || j % 4 == 1,
            PixelSplit::Gray8 => j % 2 == 0,
            PixelSplit::Rgb16 => !(j % 8 == 6 || j % 8 == 7),
            PixelSplit::Rgb8 => j % 4 != 3,
        }
    }
}

impl Globals {
    fn png_of(&mut self, img: i32) -> Box<Png> {
        match std::mem::take(&mut self.img(img).st) {
            ImageStruct::Png(p) => p,
            other => {
                self.img(img).st = other;
                texmfmp::fatal("PNG image without libpng state")
            }
        }
    }

    fn png_put(&mut self, img: i32, p: Box<Png>) {
        self.img(img).st = ImageStruct::Png(p);
    }

    /// The default libpng error handler ("libpng error: ...") followed by the longjmp
    /// to read_png_info's setjmp: pdftex_fail("libpng: internal error").
    fn libpng_error(&mut self, msg: &str) -> ! {
        self.rt.files.write(FileRef::STDERR, format!("libpng error: {}\n", msg).as_bytes());
        self.pdftex_fail("libpng: internal error")
    }

    fn png_check<T>(&mut self, r: PngResult<T>) -> T {
        match r {
            Ok(v) => v,
            Err(e) => self.libpng_error(&e),
        }
    }

    /// writepng.c read_png_info
    pub fn read_png_info(&mut self, img: i32) {
        let name = self.img(img).image_name.clone().unwrap_or_default();
        let f = self.img_xfopen(&name);
        let mut png = Box::new(Png::new(f.data));
        let r = png.read_info();
        self.png_check(r);
        let (w, h) = (png.info.width as i32, png.info.height as i32);
        self.img(img).width = w;
        self.img(img).height = h;
        if png.get_valid(PNG_INFO_pHYs) != 0 {
            let xr = zround(0.0254 * png.get_x_pixels_per_meter() as f64);
            let yr = zround(0.0254 * png.get_y_pixels_per_meter() as f64);
            self.img(img).x_res = xr;
            self.img(img).y_res = yr;
        }
        let ct = png.info.color_type;
        let color = match ct {
            PNG_COLOR_TYPE_PALETTE => IMAGE_COLOR_C | IMAGE_COLOR_I,
            PNG_COLOR_TYPE_GRAY | PNG_COLOR_TYPE_GRAY_ALPHA => IMAGE_COLOR_B,
            PNG_COLOR_TYPE_RGB | PNG_COLOR_TYPE_RGB_ALPHA => IMAGE_COLOR_C,
            _ => self.pdftex_fail(&format!("unsupported type of color_type <{}>", ct)),
        };
        self.img(img).color_type = color;
        if (self.fixedpdfmajorversion > 1 || self.fixedpdfminorversion >= 4)
            && (ct == PNG_COLOR_TYPE_GRAY_ALPHA || ct == PNG_COLOR_TYPE_RGB_ALPHA)
        {
            if self.rt.images.png.transparent_page_group == 0 {
                self.rt.images.png.transparent_page_group = self.pdfnewobjnum();
            }
            if self.pdfpagegroupval == 0 {
                self.pdfpagegroupval = self.rt.images.png.transparent_page_group;
            }
            let g = self.pdfpagegroupval;
            self.img(img).group_ref = g;
        }
        self.png_put(img, png);
    }

    fn png_colorspace(&mut self, img: i32, device: &[u8]) {
        let r = self.img(img).colorspace_ref;
        if r != 0 {
            self.pdf_printf(&format!("{} 0 R\n", r));
        } else {
            self.pdf_puts(device);
        }
    }

    /// write_noninterlaced(outmac) / write_interlaced(outmac): copy the decoded rows into
    /// pdfbuf (and the alpha bytes into smask).
    fn png_write_rows(&mut self, png: &mut Png, split: PixelSplit, smask: &mut Vec<u8>) {
        let height = png.info.height as usize;
        let rowbytes = png.info.rowbytes;
        if png.info.interlace_type == PNG_INTERLACE_NONE {
            let mut row = vec![0u8; rowbytes];
            for _ in 0..height {
                let r = png.read_row(&mut row);
                self.png_check(r);
                self.png_emit_row(&row, split, smask);
            }
        } else {
            if (png.info.height as u64) * (rowbytes as u64) >= 10240000 {
                self.pdftex_warn(
                    "large interlaced PNG might cause out of memory (use non-interlaced PNG to fix this)",
                );
            }
            let mut rows: Vec<Vec<u8>> = (0..height).map(|_| vec![0u8; rowbytes]).collect();
            let r = png.read_image(&mut rows);
            self.png_check(r);
            for row in rows {
                self.png_emit_row(&row, split, smask);
            }
        }
    }

    fn png_emit_row(&mut self, row: &[u8], split: PixelSplit, smask: &mut Vec<u8>) {
        let mut r = 0usize;
        let mut k = row.len();
        while k > 0 {
            let l = k.min(self.pdfbufsize as usize);
            self.pdfroom(l as i32);
            for j in 0..l {
                if split.to_image(j) {
                    *ix(self.pdfbuf, self.pdfptr) = row[r];
                    self.pdfptr += 1;
                } else {
                    smask.push(row[r]);
                }
                r += 1;
            }
            k -= l;
        }
    }

    /// The palette stream (write_png_palette and the copy path).
    fn png_write_palette_object(&mut self, palette_objnum: i32, palette: &[PngColor], num_palette: i32) {
        self.pdfbegindict(palette_objnum, 0);
        self.pdfbeginstream();
        for i in 0..num_palette.max(0) as usize {
            self.pdfroom(3);
            let c = palette.get(i).copied().unwrap_or_default();
            for b in [c.red, c.green, c.blue] {
                *ix(self.pdfbuf, self.pdfptr) = b;
                self.pdfptr += 1;
            }
        }
        self.pdfendstream();
    }

    fn write_png_palette(&mut self, img: i32, png: &mut Png) {
        let num_palette = png.get_plte().map(|(_, n)| n).unwrap_or(0);
        self.pdfcreateobj(0, 0);
        let palette_objnum = self.objptr;
        let r = self.img(img).colorspace_ref;
        if r != 0 {
            self.pdf_printf(&format!("{} 0 R\n", r));
        } else {
            self.pdf_printf(&format!("[/Indexed /DeviceRGB {} {} 0 R]\n", num_palette - 1, palette_objnum));
        }
        self.pdfbeginstream();
        let mut smask = Vec::new();
        self.png_write_rows(png, PixelSplit::Simple, &mut smask);
        self.pdfendstream();
        if palette_objnum > 0 {
            let pal = png.palette.clone();
            self.png_write_palette_object(palette_objnum, &pal, num_palette);
        }
    }

    fn write_png_simple(&mut self, img: i32, png: &mut Png, device: &[u8]) {
        self.png_colorspace(img, device);
        self.pdfbeginstream();
        let mut smask = Vec::new();
        self.png_write_rows(png, PixelSplit::Simple, &mut smask);
        self.pdfendstream();
    }

    /// write_png_gray_alpha / write_png_rgb_alpha
    fn write_png_alpha(&mut self, img: i32, png: &mut Png, rgb: bool) {
        self.png_colorspace(img, if rgb { b"/DeviceRGB\n" } else { b"/DeviceGray\n" });
        self.pdfcreateobj(0, 0);
        let smask_objnum = self.objptr;
        self.pdf_printf(&format!("/SMask {} 0 R\n", smask_objnum));
        let div = if rgb { 4 } else { 2 };
        let smask_size = (png.info.rowbytes / div) * png.info.height as usize;
        let mut smask: Vec<u8> = Vec::with_capacity(smask_size);
        self.pdfbeginstream();
        let hicolor16 = png.info.bit_depth == 16 && self.fixedimagehicolor;
        let split = match (rgb, hicolor16) {
            (false, true) => PixelSplit::Gray16,
            (false, false) => PixelSplit::Gray8,
            (true, true) => PixelSplit::Rgb16,
            (true, false) => PixelSplit::Rgb8,
        };
        self.png_write_rows(png, split, &mut smask);
        self.pdfendstream();
        self.pdfflush();
        if smask_objnum > 0 {
            let bitdepth = png.info.bit_depth as i32;
            self.pdfbegindict(smask_objnum, 0);
            self.pdf_puts(b"/Type /XObject\n/Subtype /Image\n");
            self.pdf_printf(&format!(
                "/Width {}\n/Height {}\n/BitsPerComponent {}\n",
                png.info.width as i32,
                png.info.height as i32,
                if bitdepth == 16 { 8 } else { bitdepth }
            ));
            self.pdf_puts(b"/ColorSpace /DeviceGray\n");
            self.pdfbeginstream();
            smask.resize(smask_size.max(smask.len()), 0);
            let mut i = 0usize;
            while i < smask_size {
                if i % 8 == 0 {
                    self.pdfroom(8);
                }
                *ix(self.pdfbuf, self.pdfptr) = smask[i];
                self.pdfptr += 1;
                if bitdepth == 16 {
                    i += 1;
                }
                i += 1;
            }
            self.pdfendstream();
        }
    }

    /// writepng.c spng_getint
    fn spng_getint(&mut self, fp: &mut CFile) -> i32 {
        let mut buf = [0u8; 4];
        if fp.read(&mut buf) != 4 {
            self.pdftex_fail("writepng: reading chunk type failed");
        }
        (((((buf[0] as i32) << 8).wrapping_add(buf[1] as i32) << 8).wrapping_add(buf[2] as i32)) << 8)
            .wrapping_add(buf[3] as i32)
    }

    /// writepng.c copy_png
    fn copy_png(&mut self, png: &Png) {
        let mut fp = CFile::new(png.file_data().to_vec());
        let mut streamlength: i32 = 0;
        if !fp.seek(8, SEEK_SET) {
            self.pdftex_fail("writepng: fseek in PNG file failed");
        }
        loop {
            let len = self.spng_getint(&mut fp);
            let t = self.spng_getint(&mut fp);
            if t == SPNG_CHUNK_IEND {
                break;
            }
            if t == SPNG_CHUNK_IDAT {
                streamlength = streamlength.wrapping_add(len);
            }
            if !fp.seek(len.wrapping_add(4) as i64, SEEK_CUR) {
                self.pdftex_fail("writepng: fseek in PNG file failed");
            }
        }
        self.pdf_printf(&format!(
            "/Length {}\n/Filter/FlateDecode\n/DecodeParms<</Colors {}/Columns {}/BitsPerComponent {}/Predictor 10>>\n>>\nstream\n",
            streamlength,
            if png.info.color_type == 2 { 3 } else { 1 },
            png.info.width as i32,
            png.info.bit_depth as i32
        ));
        let mut idat = 0;
        if !fp.seek(8, SEEK_SET) {
            self.pdftex_fail("writepng: fseek in PNG file failed");
        }
        loop {
            let mut len = self.spng_getint(&mut fp);
            let t = self.spng_getint(&mut fp);
            match t {
                SPNG_CHUNK_IDAT => {
                    if idat == 2 {
                        self.pdftex_fail("writepng: IDAT chunk sequence broken");
                    }
                    idat = 1;
                    while len > 0 {
                        let i = if len > self.pdfbufsize { self.pdfbufsize } else { len };
                        self.pdfroom(i);
                        let dst = slice(self.pdfbuf, self.pdfptr, i as usize);
                        fp.read(dst);
                        self.pdfptr += i;
                        len -= i;
                    }
                    if !fp.seek(4, SEEK_CUR) {
                        self.pdftex_fail("writepng: fseek in PNG file failed");
                    }
                }
                SPNG_CHUNK_IEND => {
                    self.pdfendstream();
                    break;
                }
                _ => {
                    if idat == 1 {
                        idat = 2;
                    }
                    if !fp.seek(len.wrapping_add(4) as i64, SEEK_CUR) {
                        self.pdftex_fail("writepng: fseek in PNG file failed");
                    }
                }
            }
        }
    }

    /// writepng.c write_additional_png_objects
    fn write_additional_png_objects(&mut self) {
        let st = &self.rt.images.png;
        if st.last_png_needs_page_group && !st.transparent_page_group_was_written && st.transparent_page_group > 0 {
            let g = st.transparent_page_group;
            self.rt.images.png.transparent_page_group_was_written = true;
            self.pdfbeginobj(g, 2);
            if self.getpdfcompresslevel() == 0 {
                self.pdf_puts(b"%PTEX Group needed for transparent pngs\n");
            }
            self.pdf_puts(b"<</Type/Group /S/Transparency /CS/DeviceRGB /I true>>\n");
            self.pdfendobj();
        }
    }

    /// writepng.c write_png
    pub fn write_png(&mut self, img: i32) {
        let mut png = self.png_of(img);
        let mut png_copy = true;
        let mut gamma = 0.0f64;
        let mut int_file_gamma = 0i32;
        self.rt.images.png.last_png_needs_page_group = false;
        let num_palette = png.get_plte().map(|(_, n)| n).unwrap_or(0);

        if self.fixedpdfmajorversion == 1 && self.fixedpdfminorversion < 5 {
            self.fixedimagehicolor = false;
        }
        self.pdf_puts(b"/Type /XObject\n/Subtype /Image\n");
        if png.get_valid(PNG_INFO_tRNS) != 0 {
            let r = png.set_trns_to_alpha();
            self.png_check(r);
            png_copy = false;
        }
        if self.fixedpdfmajorversion == 1
            && self.fixedpdfminorversion < 4
            && png.info.color_type & PNG_COLOR_MASK_ALPHA != 0
        {
            let r = png.set_strip_alpha();
            self.png_check(r);
            png_copy = false;
        }
        if self.fixedpdfmajorversion == 1 && self.fixedpdfminorversion < 5 {
            self.fixedimagehicolor = false;
        }
        if png.info.bit_depth == 16 && !self.fixedimagehicolor {
            let r = png.set_strip_16();
            self.png_check(r);
            png_copy = false;
        }
        if let Some(g) = png.get_gama_fixed() {
            gamma = g as f64 * 0.00001;
            int_file_gamma = g;
        }
        if self.fixedimageapplygamma != 0 {
            let scrn = self.fixedgamma as f64 / 1000.0;
            let r = if png.get_valid(PNG_INFO_gAMA) != 0 {
                png.set_gamma(scrn, gamma)
            } else {
                png.set_gamma(scrn, 1000.0 / self.fixedimagegamma as f64)
            };
            self.png_check(r);
            png_copy = false;
        }
        png.set_interlace_handling();
        let r = png.read_update_info();
        self.png_check(r);

        self.pdf_printf(&format!(
            "/Width {}\n/Height {}\n/BitsPerComponent {}\n",
            png.info.width as i32, png.info.height as i32, png.info.bit_depth as i32
        ));
        self.pdf_puts(b"/ColorSpace ");
        let ct = png.info.color_type;
        let gama_valid = png.get_valid(PNG_INFO_gAMA) != 0;
        let blocking = PNG_INFO_cHRM
            | PNG_INFO_iCCP
            | PNG_INFO_sBIT
            | PNG_INFO_sRGB
            | PNG_INFO_bKGD
            | PNG_INFO_hIST
            | PNG_INFO_tRNS
            | PNG_INFO_sPLT;
        if png_copy
            && (self.fixedpdfmajorversion > 1 || self.fixedpdfminorversion > 1)
            && png.info.interlace_type == PNG_INTERLACE_NONE
            && (ct == PNG_COLOR_TYPE_GRAY || ct == PNG_COLOR_TYPE_RGB)
            && self.fixedimageapplygamma == 0
            && (!gama_valid || int_file_gamma == PNG_FP_1)
            && png.get_valid(blocking) == 0
        {
            let mut palette_objnum = 0;
            let r = self.img(img).colorspace_ref;
            if r != 0 {
                self.pdf_printf(&format!("{} 0 R\n", r));
            } else {
                match ct {
                    PNG_COLOR_TYPE_PALETTE => {
                        self.pdfcreateobj(0, 0);
                        palette_objnum = self.objptr;
                        self.pdf_printf(&format!(
                            "[/Indexed /DeviceRGB {} {} 0 R]\n",
                            num_palette - 1,
                            palette_objnum
                        ));
                    }
                    PNG_COLOR_TYPE_GRAY => self.pdf_puts(b"/DeviceGray\n"),
                    _ => self.pdf_puts(b"/DeviceRGB\n"),
                }
            }
            self.tex_printf(" (PNG copy)");
            self.copy_png(&png);
            if palette_objnum > 0 {
                let pal = png.palette.clone();
                self.png_write_palette_object(palette_objnum, &pal, num_palette);
            }
        } else {
            if std::env::var("TEXMF_DEBUG_PNG_COPY").map(|v| v == "1").unwrap_or(false) {
                self.tex_printf(" *** PNG copy skipped because:");
                if !png_copy {
                    self.tex_printf(" !png_copy");
                }
                if self.fixedpdfmajorversion == 1 && self.fixedpdfminorversion <= 1 {
                    self.tex_printf(&format!(" minorversion={} (and majorversion=1)", self.fixedpdfminorversion));
                }
                if png.info.interlace_type != PNG_INTERLACE_NONE {
                    self.tex_printf(" interlaced");
                }
                if !(ct == PNG_COLOR_TYPE_GRAY || ct == PNG_COLOR_TYPE_RGB) {
                    self.tex_printf(" colortype");
                }
                if self.fixedimageapplygamma != 0 {
                    self.tex_printf(" apply gamma");
                }
                if !(!gama_valid || int_file_gamma == PNG_FP_1) {
                    self.tex_printf(" gamma");
                }
                for (flag, name) in [
                    (PNG_INFO_cHRM, " cHRM"),
                    (PNG_INFO_iCCP, " iCCP"),
                    (PNG_INFO_sBIT, " sBIT"),
                    (PNG_INFO_sRGB, " sRGB"),
                    (PNG_INFO_bKGD, " bKGD"),
                    (PNG_INFO_hIST, " hIST"),
                    (PNG_INFO_tRNS, " tRNS"),
                    (PNG_INFO_sPLT, " sPLT"),
                ] {
                    if png.get_valid(flag) != 0 {
                        self.tex_printf(name);
                    }
                }
            }
            let v14 = self.fixedpdfminorversion >= 4 || self.fixedpdfmajorversion > 1;
            match ct {
                PNG_COLOR_TYPE_PALETTE => self.write_png_palette(img, &mut png),
                PNG_COLOR_TYPE_GRAY => self.write_png_simple(img, &mut png, b"/DeviceGray\n"),
                PNG_COLOR_TYPE_GRAY_ALPHA => {
                    if v14 {
                        self.write_png_alpha(img, &mut png, false);
                        self.rt.images.png.last_png_needs_page_group = true;
                    } else {
                        self.write_png_simple(img, &mut png, b"/DeviceGray\n");
                    }
                }
                PNG_COLOR_TYPE_RGB => self.write_png_simple(img, &mut png, b"/DeviceRGB\n"),
                PNG_COLOR_TYPE_RGB_ALPHA => {
                    if v14 {
                        self.write_png_alpha(img, &mut png, true);
                        self.rt.images.png.last_png_needs_page_group = true;
                    } else {
                        self.write_png_simple(img, &mut png, b"/DeviceRGB\n");
                    }
                }
                _ => self.pdftex_fail(&format!("unsupported type of color_type <{}>", ct)),
            }
        }
        self.png_put(img, png);
        self.pdfflush();
        self.write_additional_png_objects();
    }
}
