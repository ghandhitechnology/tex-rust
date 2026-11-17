//! Port of pdftexdir/writejpg.c: JPEG files are embedded as they are (/DCTDecode); only
//! the SOFn header, JFIF density and Exif resolution tags are read.

use super::{xfseek, CFile, ImageStruct, IMAGE_COLOR_B, IMAGE_COLOR_C, SEEK_CUR, SEEK_END, SEEK_SET};
use crate::gen::globals::Globals;

const JPG_GRAY: i32 = 1;
const JPG_RGB: i32 = 3;
const JPG_CMYK: i32 = 4;

const M_SOF0: i32 = 0xc0;
const M_SOF1: i32 = 0xc1;
const M_SOF2: i32 = 0xc2;
const M_SOF3: i32 = 0xc3;
const M_SOF5: i32 = 0xc5;
const M_SOF6: i32 = 0xc6;
const M_SOF7: i32 = 0xc7;
const M_SOF9: i32 = 0xc9;
const M_SOF10: i32 = 0xca;
const M_SOF11: i32 = 0xcb;
const M_SOF13: i32 = 0xcd;
const M_SOF14: i32 = 0xce;
const M_SOF15: i32 = 0xcf;
const M_RST0: i32 = 0xd0;
const M_RST7: i32 = 0xd7;
const M_SOI: i32 = 0xd8;
const M_EOI: i32 = 0xd9;
const M_TEM: i32 = 0x01;

/// writejpg.c read2bytes: (c << 8) + xgetc(f) as JPG_UINT16 (unsigned int)
fn read2bytes(f: &mut CFile) -> u32 {
    let c = f.getc();
    (c << 8).wrapping_add(f.getc()) as u32
}

/// get_unsigned_byte: (unsigned char) fgetc
fn get_unsigned_byte(f: &mut CFile) -> u8 {
    f.getc() as u8
}

fn get_unsigned_pair(f: &mut CFile) -> u16 {
    let mut pair = get_unsigned_byte(f) as u16;
    pair = (pair << 8) | get_unsigned_byte(f) as u16;
    pair
}

/// Byte at an offset of the Exif buffer; reads outside it (the C code does not check)
/// give 0.
fn at(buf: &[u8], i: i64) -> u32 {
    if i >= 0 && (i as usize) < buf.len() {
        buf[i as usize] as u32
    } else {
        0
    }
}

/// writejpg.c read_exif_bytes
fn read_exif_bytes(buf: &[u8], p: &mut i64, n: i32, b: bool) -> u32 {
    let mut rval: u32 = 0;
    let mut pp = *p;
    if b {
        if n == 4 {
            rval = rval.wrapping_add(at(buf, pp));
            pp += 1;
            rval <<= 8;
            rval = rval.wrapping_add(at(buf, pp));
            pp += 1;
            rval <<= 8;
        }
        if n == 4 || n == 2 {
            rval = rval.wrapping_add(at(buf, pp));
            pp += 1;
            rval <<= 8;
            rval = rval.wrapping_add(at(buf, pp));
        }
    } else {
        pp += n as i64;
        if n == 4 {
            pp -= 1;
            rval = rval.wrapping_add(at(buf, pp));
            rval <<= 8;
            pp -= 1;
            rval = rval.wrapping_add(at(buf, pp));
            rval <<= 8;
        }
        if n == 4 || n == 2 {
            pp -= 1;
            rval = rval.wrapping_add(at(buf, pp));
            rval <<= 8;
            pp -= 1;
            rval = rval.wrapping_add(at(buf, pp));
        }
    }
    *p += n as i64;
    rval
}

/// writejpg.c read_APP1_Exif: returns (xx, yy) when the TIFF header is valid.
fn read_app1_exif(fp: &mut CFile, length: u16) -> Option<(i32, i32)> {
    let mut buffer = vec![0u8; length as usize];
    fp.read(&mut buffer);
    let len = length as i64;
    let mut p: i64 = 0;
    while p < len && at(&buffer, p) == 0 {
        p += 1;
    }
    let tiff_header = p;
    let bigendian = if at(&buffer, p) == b'M' as u32 && at(&buffer, p + 1) == b'M' as u32 {
        true
    } else if at(&buffer, p) == b'I' as u32 && at(&buffer, p + 1) == b'I' as u32 {
        false
    } else {
        return None;
    };
    p += 2;
    let i = read_exif_bytes(&buffer, &mut p, 2, bigendian) as i32;
    if i != 42 {
        return None;
    }
    let i = read_exif_bytes(&buffer, &mut p, 4, bigendian) as i32;
    p = tiff_header + i as i64;
    let mut num_fields = read_exif_bytes(&buffer, &mut p, 2, bigendian) as i32;
    let (mut value, mut num, mut den) = (0i32, 0i32, 0i32);
    let (mut xres, mut yres, mut res_unit) = (72.0f64, 72.0f64, 1.0f64);
    while num_fields > 0 {
        num_fields -= 1;
        let tag = read_exif_bytes(&buffer, &mut p, 2, bigendian) as i32;
        let typ = read_exif_bytes(&buffer, &mut p, 2, bigendian) as i32;
        read_exif_bytes(&buffer, &mut p, 4, bigendian);
        match typ {
            1 | 7 => {
                value = at(&buffer, p) as i32;
                p += 4;
            }
            3 => {
                value = read_exif_bytes(&buffer, &mut p, 2, bigendian) as i32;
                p += 2;
            }
            4 | 9 => {
                value = read_exif_bytes(&buffer, &mut p, 4, bigendian) as i32;
            }
            5 | 10 => {
                value = read_exif_bytes(&buffer, &mut p, 4, bigendian) as i32;
                let mut rp = tiff_header + value as i64;
                num = read_exif_bytes(&buffer, &mut rp, 4, bigendian) as i32;
                den = read_exif_bytes(&buffer, &mut rp, 4, bigendian) as i32;
            }
            _ => {
                p += 4;
            }
        }
        match tag {
            282 => {
                if den != 0 {
                    xres = num.wrapping_div(den) as f64;
                }
            }
            283 => {
                if den != 0 {
                    yres = num.wrapping_div(den) as f64;
                }
            }
            296 => match value {
                2 => res_unit = 1.0,
                3 => res_unit = 2.54,
                _ => {}
            },
            _ => {}
        }
    }
    Some(((xres * res_unit) as i32, (yres * res_unit) as i32))
}

impl Globals {
    fn jpg_file(&mut self, img: i32) -> &mut super::JpgImageInfo {
        match &mut self.rt.images.array[img as usize].st {
            ImageStruct::Jpg(j) => j,
            _ => unreachable!("JPEG image without JPG_IMAGE_INFO"),
        }
    }

    /// writejpg.c read_jpg_info
    pub fn read_jpg_info(&mut self, img: i32) {
        self.img(img).x_res = 0;
        self.img(img).y_res = 0;
        let name = self.img(img).image_name.clone().unwrap_or_default();
        let cur = self.rt.pdfu.cur_file_name.clone().unwrap_or_default();
        let mut f = self.img_xfopen(&name);
        xfseek(&mut f, 0, SEEK_END, &cur);
        let length = f.tell() as u32;
        xfseek(&mut f, 0, SEEK_SET, &cur);
        if read2bytes(&mut f) != 0xFFD8 {
            self.jpg_file(img).file = f;
            self.pdftex_fail("reading JPEG image failed (no JPEG header found)");
        }
        let appmk = read2bytes(&mut f);
        let mut xres: i32 = 0;
        let mut yres: i32 = 0;
        if appmk == 0xFFE0 {
            read2bytes(&mut f);
            let jpg_id = b"JFIF\0";
            let mut i = 0;
            while i < 5 {
                if f.getc() != jpg_id[i] as i32 {
                    break;
                }
                i += 1;
            }
            if i == 5 {
                read2bytes(&mut f);
                let units = f.getc();
                xres = read2bytes(&mut f) as i32;
                yres = read2bytes(&mut f) as i32;
                match units {
                    1 => {}
                    2 => {
                        xres = (xres as f64 * 2.54) as i32;
                        yres = (yres as f64 * 2.54) as i32;
                    }
                    _ => {
                        xres = 0;
                        yres = 0;
                    }
                }
            }
            if xres == 0 && yres != 0 {
                xres = yres;
            }
            if yres == 0 && xres != 0 {
                yres = xres;
            }
        } else if appmk == 0xFFE1 {
            let mut xx = 0;
            let mut yy = 0;
            let mut length = get_unsigned_pair(&mut f).wrapping_sub(2);
            if length > 5 {
                let mut app_sig = [0u8; 5];
                if f.read(&mut app_sig) != 5 {
                    // `return` from read_jpg_info: no SOF is looked for
                    self.img(img).x_res = 0;
                    self.img(img).y_res = 0;
                    let j = self.jpg_file(img);
                    j.length = f.data.len() as u32;
                    j.file = f;
                    return;
                }
                length -= 5;
                if &app_sig == b"Exif\0" {
                    if let Some((a, b)) = read_app1_exif(&mut f, length) {
                        xx = a;
                        yy = b;
                    }
                }
            }
            xres = xx;
            yres = yy;
        }
        self.img(img).x_res = xres;
        self.img(img).y_res = yres;
        xfseek(&mut f, 0, SEEK_SET, &cur);
        loop {
            if f.feof() {
                self.pdftex_fail("reading JPEG image failed (premature file end)");
            }
            if f.getc() != 0xFF {
                self.pdftex_fail("reading JPEG image failed (no marker found)");
            }
            let m = f.getc();
            match m {
                M_SOF5 | M_SOF6 | M_SOF7 | M_SOF9 | M_SOF10 | M_SOF11 | M_SOF13 | M_SOF14 | M_SOF15 => {
                    self.pdftex_fail("unsupported type of compression");
                }
                M_SOF2 | M_SOF0 | M_SOF1 | M_SOF3 => {
                    if m == M_SOF2 && self.fixedpdfmajorversion == 1 && self.fixedpdfminorversion <= 2 {
                        self.pdftex_fail("cannot use progressive DCT with PDF-1.2");
                    }
                    read2bytes(&mut f);
                    let bpc = f.getc() as u8;
                    let h = read2bytes(&mut f) as i32;
                    let w = read2bytes(&mut f) as i32;
                    let cs = f.getc();
                    xfseek(&mut f, 0, SEEK_SET, &cur);
                    self.img(img).height = h;
                    self.img(img).width = w;
                    let j = self.jpg_file(img);
                    j.bits_per_component = bpc;
                    j.color_space = cs;
                    j.length = length;
                    j.file = f;
                    let color = match cs {
                        JPG_GRAY => IMAGE_COLOR_B,
                        JPG_RGB | JPG_CMYK => IMAGE_COLOR_C,
                        _ => self.pdftex_fail(&format!("Unsupported color space {}", cs)),
                    };
                    self.img(img).color_type = color;
                    return;
                }
                M_SOI | M_EOI | M_TEM => {}
                _ if (M_RST0..=M_RST7).contains(&m) => {}
                _ => {
                    // xfseek(file, read2bytes(file) - 2, SEEK_CUR): an unsigned int converted
                    // to a 32-bit long
                    let off = read2bytes(&mut f).wrapping_sub(2) as i32 as i64;
                    xfseek(&mut f, off, SEEK_CUR, &cur);
                }
            }
        }
    }

    /// writejpg.c write_jpg
    pub fn write_jpg(&mut self, img: i32) {
        let (w, h) = (self.img(img).width, self.img(img).height);
        let (bpc, length, cs) = {
            let j = self.jpg_file(img);
            (j.bits_per_component as i32, j.length, j.color_space)
        };
        self.pdf_puts(b"/Type /XObject\n/Subtype /Image\n");
        self.pdf_printf(&format!(
            "/Width {}\n/Height {}\n/BitsPerComponent {}\n/Length {}\n",
            w, h, bpc, length as i32
        ));
        self.pdf_puts(b"/ColorSpace ");
        let r = self.img(img).colorspace_ref;
        if r != 0 {
            self.pdf_printf(&format!("{} 0 R\n", r));
        } else {
            match cs {
                JPG_GRAY => self.pdf_puts(b"/DeviceGray\n"),
                JPG_RGB => self.pdf_puts(b"/DeviceRGB\n"),
                JPG_CMYK => self.pdf_puts(b"/DeviceCMYK\n/Decode [1 0 1 0 1 0 1 0]\n"),
                _ => self.pdftex_fail(&format!("Unsupported color space {}", cs)),
            }
        }
        self.pdf_puts(b"/Filter /DCTDecode\n>>\nstream\n");
        // for (l = length; l > 0; l--) pdfout(xgetc(f)): EOF (-1) is written as 0xFF
        let j = self.jpg_file(img);
        let start = (j.file.pos as usize).min(j.file.data.len());
        let avail = (j.file.data.len() - start).min(length as usize);
        let mut bytes = j.file.data[start..start + avail].to_vec();
        j.file.pos += length as u64;
        bytes.resize(length as usize, 0xFF);
        self.pdfout_bytes(&bytes);
        self.pdfendstream();
    }
}
