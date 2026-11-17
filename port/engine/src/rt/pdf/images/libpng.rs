//! The libpng 1.6.55 read path (TeX Live 2026, `libs/libpng/libpng-src`) as pdfTeX's
//! writepng.c uses it, ported to Rust.
//!
//! Covered: `png_create_read_struct`/`png_read_info` (signature, chunk header and CRC
//! handling, the 1.6.47+ table driven chunk checks, every chunk handler whose result
//! pdfTeX can observe: IHDR, PLTE, IEND, tRNS, gAMA, sBIT, cHRM, sRGB, iCCP with the ICC
//! header/tag table checks, sPLT, bKGD, hIST, pHYs; the other known chunks are checked
//! and skipped exactly like their handlers would leave them), the transformations
//! writepng.c requests (`png_set_tRNS_to_alpha`, `png_set_strip_alpha`,
//! `png_set_strip_16`, `png_set_gamma`, `png_set_interlace_handling`) with
//! `png_init_read_transformations`, `png_read_transform_info`, the gamma tables of png.c
//! (floating point arithmetic, as in the TL build), and `png_read_row`/`png_read_image`
//! (IDAT inflate through the zlib port, row unfiltering, Adam7 combination).
//!
//! Errors are `Err(message)` with the text libpng's default error handler would print.
//! Warnings are dropped: pdfTeX installs a warning handler that ignores them, and in
//! release builds benign errors are warnings.
//!
//! Not reproduced: the bits after the last pixel of a row with fewer than 8 bits per
//! pixel. libpng keeps them from the caller's row buffer, which pdfTeX allocates with
//! malloc and never initializes; here they are zero.

#![allow(non_upper_case_globals)]
// the full set of png.h/pngpriv.h constants is kept even where pdfTeX needs no test
#![allow(dead_code)]

use texrs_zlib::{Flush, Inflate, Status};

// ------------------------------------------------------------------------------- png.h
pub const PNG_COLOR_MASK_PALETTE: u8 = 1;
pub const PNG_COLOR_MASK_COLOR: u8 = 2;
pub const PNG_COLOR_MASK_ALPHA: u8 = 4;
pub const PNG_COLOR_TYPE_GRAY: u8 = 0;
pub const PNG_COLOR_TYPE_PALETTE: u8 = PNG_COLOR_MASK_COLOR | PNG_COLOR_MASK_PALETTE;
pub const PNG_COLOR_TYPE_RGB: u8 = PNG_COLOR_MASK_COLOR;
pub const PNG_COLOR_TYPE_RGB_ALPHA: u8 = PNG_COLOR_MASK_COLOR | PNG_COLOR_MASK_ALPHA;
pub const PNG_COLOR_TYPE_GRAY_ALPHA: u8 = PNG_COLOR_MASK_ALPHA;
pub const PNG_INTERLACE_NONE: u8 = 0;
pub const PNG_RESOLUTION_METER: u8 = 1;

pub const PNG_INFO_gAMA: u32 = 0x0001;
pub const PNG_INFO_sBIT: u32 = 0x0002;
pub const PNG_INFO_cHRM: u32 = 0x0004;
pub const PNG_INFO_PLTE: u32 = 0x0008;
pub const PNG_INFO_tRNS: u32 = 0x0010;
pub const PNG_INFO_bKGD: u32 = 0x0020;
pub const PNG_INFO_hIST: u32 = 0x0040;
pub const PNG_INFO_pHYs: u32 = 0x0080;
pub const PNG_INFO_oFFs: u32 = 0x0100;
pub const PNG_INFO_tIME: u32 = 0x0200;
pub const PNG_INFO_pCAL: u32 = 0x0400;
pub const PNG_INFO_sRGB: u32 = 0x0800;
pub const PNG_INFO_iCCP: u32 = 0x1000;
pub const PNG_INFO_sPLT: u32 = 0x2000;
pub const PNG_INFO_sCAL: u32 = 0x4000;
pub const PNG_INFO_eXIf: u32 = 0x10000;
pub const PNG_INFO_cICP: u32 = 0x20000;
pub const PNG_INFO_cLLI: u32 = 0x40000;
pub const PNG_INFO_mDCV: u32 = 0x80000;

pub const PNG_FP_1: i32 = 100000;
const PNG_FP_MAX: f64 = 2147483647.0;
const PNG_FP_MIN: f64 = -2147483647.0;
const PNG_UINT_31_MAX: u32 = 0x7fff_ffff;
const PNG_MAX_PALETTE_LENGTH: usize = 256;
const PNG_DEFAULT_sRGB: i32 = -1;
const PNG_GAMMA_MAC_18: i32 = -2;
const PNG_GAMMA_sRGB: i32 = 220000;
const PNG_sRGB_INTENT_LAST: u32 = 4;

// ------------------------------------------------------------------------- pngpriv.h
const PNG_HAVE_IHDR: u32 = 0x01;
const PNG_HAVE_PLTE: u32 = 0x02;
const PNG_HAVE_IDAT: u32 = 0x04;
const PNG_AFTER_IDAT: u32 = 0x08;
const PNG_HAVE_IEND: u32 = 0x10;
const PNG_HAVE_PNG_SIGNATURE: u32 = 0x1000;
const PNG_HAVE_CHUNK_AFTER_IDAT: u32 = 0x2000;
const PNG_IS_READ_STRUCT: u32 = 0x8000;

const PNG_INTERLACE: u32 = 0x0002;
const PNG_COMPOSE: u32 = 0x0080;
const PNG_BACKGROUND_EXPAND: u32 = 0x0100;
const PNG_16_TO_8: u32 = 0x0400;
const PNG_EXPAND: u32 = 0x1000;
const PNG_GAMMA: u32 = 0x2000;
const PNG_STRIP_ALPHA: u32 = 0x40000;
const PNG_ENCODE_ALPHA: u32 = 0x800000;
const PNG_EXPAND_tRNS: u32 = 0x2000000;
const PNG_SCALE_16_TO_8: u32 = 0x4000000;

const PNG_GAMMA_sRGB_INVERSE: i32 = 45455;
const PNG_GAMMA_MAC_OLD: i32 = 151724;
const PNG_GAMMA_MAC_INVERSE: i32 = 65909;
const PNG_LIB_GAMMA_MIN: i32 = 1000;
const PNG_LIB_GAMMA_MAX: i32 = 10000000;

// pnglibconf.h
const PNG_GAMMA_THRESHOLD_FIXED: i32 = 5000;
const PNG_MAX_GAMMA_8: u32 = 11;
const PNG_INFLATE_BUF_SIZE: usize = 1024;
const PNG_IDAT_READ_SIZE: u32 = 8192; // PNG_ZBUF_SIZE
const PNG_USER_CHUNK_CACHE_MAX: u32 = 1000;
const PNG_USER_CHUNK_MALLOC_MAX: u32 = 8000000;
const PNG_USER_WIDTH_MAX: u32 = 1000000;
const PNG_USER_HEIGHT_MAX: u32 = 1000000;
const ZLIB_IO_MAX: usize = u32::MAX as usize;

const LZ77_MIN: u32 = 2 + 5 + 4;

const PASS_START: [u32; 7] = [0, 4, 0, 2, 0, 1, 0];
const PASS_INC: [u32; 7] = [8, 8, 4, 4, 2, 2, 1];
const PASS_YSTART: [u32; 7] = [0, 0, 4, 0, 2, 0, 1];
const PASS_YINC: [u32; 7] = [8, 8, 8, 4, 4, 2, 2];

const fn chunk(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | s[3] as u32
}
const png_IHDR: u32 = chunk(b"IHDR");
const png_PLTE: u32 = chunk(b"PLTE");
const png_IDAT: u32 = chunk(b"IDAT");
const png_IEND: u32 = chunk(b"IEND");
const png_iCCP: u32 = chunk(b"iCCP");

fn chunk_ancillary(c: u32) -> bool {
    (c >> 29) & 1 != 0
}

#[inline]
pub fn rowbytes(pixel_bits: u32, width: u32) -> usize {
    if pixel_bits >= 8 {
        width as usize * (pixel_bits as usize >> 3)
    } else {
        (width as usize * pixel_bits as usize + 7) >> 3
    }
}

fn get_uint_32(b: &[u8]) -> u32 {
    ((b[0] as u32) << 24) + ((b[1] as u32) << 16) + ((b[2] as u32) << 8) + b[3] as u32
}
fn get_uint_16(b: &[u8]) -> u16 {
    (((b[0] as u32) << 8) + b[1] as u32) as u16
}
fn get_int_32_checked(b: &[u8], error: &mut bool) -> i32 {
    let mut u = get_uint_32(b);
    if u & 0x8000_0000 == 0 {
        return u as i32;
    }
    u = (u ^ 0xffff_ffff).wrapping_add(1);
    if u & 0x8000_0000 == 0 {
        return -(u as i32);
    }
    *error = true;
    0
}

#[derive(Clone, Copy, Default, Debug)]
pub struct PngColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct PngColor16 {
    pub index: u8,
    pub red: u16,
    pub green: u16,
    pub blue: u16,
    pub gray: u16,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct PngColor8 {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub gray: u8,
    pub alpha: u8,
}

/// png_info: the parts pdfTeX reads back.
#[derive(Default, Debug)]
pub struct PngInfo {
    pub valid: u32,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
    pub interlace_type: u8,
    pub compression_type: u8,
    pub filter_type: u8,
    pub channels: u8,
    pub pixel_depth: u8,
    pub rowbytes: usize,
    pub num_palette: u16,
    pub num_trans: u16,
    pub gamma: i32,
    pub x_pixels_per_unit: u32,
    pub y_pixels_per_unit: u32,
    pub phys_unit_type: u8,
    pub sig_bit: PngColor8,
    pub background: PngColor16,
    pub trans_color: PngColor16,
    pub rendering_intent: i32,
}

/// png_row_info
#[derive(Clone, Copy, Default)]
struct RowInfo {
    width: u32,
    rowbytes: usize,
    color_type: u8,
    bit_depth: u8,
    channels: u8,
    pixel_depth: u8,
}

pub type PngResult<T> = Result<T, String>;

#[derive(PartialEq, PartialOrd, Clone, Copy)]
enum Handled {
    Error = 0,
    Discarded = 1,
    Saved = 2,
    Ok = 3,
}

/// The chunk table of pngrutil.c (`read_chunks`): handler, length limits, position rules.
#[derive(Clone, Copy, PartialEq)]
enum MaxLen {
    NoCheck,
    Limit,
    Max(u32),
}
struct ChunkRule {
    name: u32,
    has_handler: bool,
    max_length: MaxLen,
    min_length: u32,
    pos_before: u32,
    pos_after: u32,
    multiple: bool,
}

const H_COL: u32 = PNG_HAVE_PLTE | PNG_HAVE_IDAT;
const fn rule(n: &[u8; 4], h: bool, max: MaxLen, min: u32, before: u32, after: u32, multiple: bool) -> ChunkRule {
    ChunkRule { name: chunk(n), has_handler: h, max_length: max, min_length: min, pos_before: before, pos_after: after, multiple }
}
const LK_MIN: u32 = 3 + LZ77_MIN;
static READ_CHUNKS: [ChunkRule; 28] = [
    rule(b"IHDR", true, MaxLen::Max(13), 13, PNG_HAVE_IHDR, 0, false),
    rule(b"PLTE", true, MaxLen::NoCheck, 0, 0, PNG_HAVE_IHDR, true),
    rule(b"IDAT", false, MaxLen::NoCheck, 0, PNG_AFTER_IDAT, PNG_HAVE_IHDR, true),
    rule(b"IEND", true, MaxLen::NoCheck, 0, 0, PNG_AFTER_IDAT, false),
    rule(b"acTL", false, MaxLen::Max(8), 8, PNG_HAVE_IDAT, PNG_HAVE_IHDR, false),
    rule(b"bKGD", true, MaxLen::Max(6), 1, PNG_HAVE_IDAT, PNG_HAVE_IHDR, false),
    rule(b"cHRM", true, MaxLen::Max(32), 32, H_COL, PNG_HAVE_IHDR, false),
    rule(b"cICP", true, MaxLen::Max(4), 4, H_COL, PNG_HAVE_IHDR, false),
    rule(b"cLLI", true, MaxLen::Max(8), 8, H_COL, PNG_HAVE_IHDR, false),
    rule(b"eXIf", true, MaxLen::Limit, 4, 0, PNG_HAVE_IHDR, false),
    rule(b"fcTL", false, MaxLen::Max(25), 26, 0, PNG_HAVE_IHDR, true),
    rule(b"fdAT", false, MaxLen::Limit, 4, PNG_HAVE_IDAT, PNG_HAVE_IHDR, true),
    rule(b"gAMA", true, MaxLen::Max(4), 4, H_COL, PNG_HAVE_IHDR, false),
    rule(b"hIST", true, MaxLen::Max(1024), 0, PNG_HAVE_PLTE, PNG_HAVE_IHDR, false),
    rule(b"iCCP", true, MaxLen::NoCheck, LK_MIN, H_COL, PNG_HAVE_IHDR, false),
    rule(b"iTXt", true, MaxLen::NoCheck, 6, 0, PNG_HAVE_IHDR, true),
    rule(b"mDCV", true, MaxLen::Max(24), 24, H_COL, PNG_HAVE_IHDR, false),
    rule(b"oFFs", true, MaxLen::Max(9), 9, PNG_HAVE_IDAT, PNG_HAVE_IHDR, false),
    rule(b"pCAL", true, MaxLen::NoCheck, 14, PNG_HAVE_IDAT, PNG_HAVE_IHDR, false),
    rule(b"pHYs", true, MaxLen::Max(9), 9, PNG_HAVE_IDAT, PNG_HAVE_IHDR, false),
    rule(b"sBIT", true, MaxLen::Max(4), 1, H_COL, PNG_HAVE_IHDR, false),
    rule(b"sCAL", true, MaxLen::Limit, 4, PNG_HAVE_IDAT, PNG_HAVE_IHDR, false),
    rule(b"sPLT", true, MaxLen::NoCheck, 3, PNG_HAVE_IDAT, PNG_HAVE_IHDR, true),
    rule(b"sRGB", true, MaxLen::Max(1), 1, H_COL, PNG_HAVE_IHDR, false),
    rule(b"tEXt", true, MaxLen::NoCheck, 2, 0, PNG_HAVE_IHDR, true),
    rule(b"tIME", true, MaxLen::Max(7), 7, 0, PNG_HAVE_IHDR, false),
    rule(b"tRNS", true, MaxLen::Max(256), 0, PNG_HAVE_IDAT, PNG_HAVE_IHDR, false),
    rule(b"zTXt", true, MaxLen::Limit, LK_MIN, 0, PNG_HAVE_IHDR, true),
];

/// png_struct + png_info for one image being read.
pub struct Png {
    data: Vec<u8>,
    pos: usize,

    mode: u32,
    transformations: u32,
    row_init: bool,
    zstream_initialized: bool,
    zstream_ended: bool,
    max_inflate_window: bool,

    pub width: u32,
    pub height: u32,
    num_rows: u32,
    iwidth: u32,
    row_number: u32,
    pass: u8,
    rowbytes: usize,
    pub bit_depth: u8,
    pub color_type: u8,
    pub interlaced: u8,
    compression_type: u8,
    filter_type: u8,
    pub channels: u8,
    pub pixel_depth: u8,
    maximum_pixel_depth: u8,
    transformed_pixel_depth: u8,
    info_rowbytes: usize,

    chunk_name: u32,
    crc: u32,
    chunks_seen: u32,

    /// png_struct::palette (shared with png_info::palette in libpng 1.6)
    pub palette: Vec<PngColor>,
    pub num_palette: u16,
    trans_alpha: [u8; 256],
    pub num_trans: u16,
    trans_color: PngColor16,
    sig_bit: PngColor8,

    file_gamma: i32,
    screen_gamma: i32,
    chunk_gamma: i32,
    default_gamma: i32,
    gamma_table: Option<Vec<u8>>,
    gamma_16_table: Option<Vec<Vec<u16>>>,
    gamma_shift: u32,

    zstream: Option<Inflate>,
    zowner: u32,
    zstream_start: bool,
    /// zstream.next_in/avail_in over `zbuf`
    zbuf: Vec<u8>,
    zpos: usize,
    idat_size: u32,

    row_buf: Vec<u8>,
    prev_row: Vec<u8>,

    user_chunk_cache_max: u32,
    user_chunk_malloc_max: u32,

    pub info: PngInfo,
}

impl Png {
    /// png_create_read_struct + png_create_info_struct +
    /// png_set_option(PNG_MAXIMUM_INFLATE_WINDOW, ON) + png_init_io, over the file contents.
    pub fn new(data: Vec<u8>) -> Png {
        Png {
            data,
            pos: 0,
            mode: PNG_IS_READ_STRUCT,
            transformations: 0,
            row_init: false,
            zstream_initialized: false,
            zstream_ended: false,
            max_inflate_window: true,
            width: 0,
            height: 0,
            num_rows: 0,
            iwidth: 0,
            row_number: 0,
            pass: 0,
            rowbytes: 0,
            bit_depth: 0,
            color_type: 0,
            interlaced: 0,
            compression_type: 0,
            filter_type: 0,
            channels: 0,
            pixel_depth: 0,
            maximum_pixel_depth: 0,
            transformed_pixel_depth: 0,
            info_rowbytes: 0,
            chunk_name: 0,
            crc: 0,
            chunks_seen: 0,
            palette: Vec::new(),
            num_palette: 0,
            trans_alpha: [0; 256],
            num_trans: 0,
            trans_color: PngColor16::default(),
            sig_bit: PngColor8::default(),
            file_gamma: 0,
            screen_gamma: 0,
            chunk_gamma: 0,
            default_gamma: 0,
            gamma_table: None,
            gamma_16_table: None,
            gamma_shift: 0,
            zstream: None,
            zowner: 0,
            zstream_start: false,
            zbuf: Vec::new(),
            zpos: 0,
            idat_size: 0,
            row_buf: Vec::new(),
            prev_row: Vec::new(),
            user_chunk_cache_max: PNG_USER_CHUNK_CACHE_MAX,
            user_chunk_malloc_max: PNG_USER_CHUNK_MALLOC_MAX,
            info: PngInfo::default(),
        }
    }

    /// The underlying file (writepng.c copy_png reads it directly).
    pub fn file_data(&self) -> &[u8] {
        &self.data
    }

    // ---------------------------------------------------------------- errors
    /// png_format_buffer: "cHNK: message", with non-letters as [hh].
    fn format_chunk_msg(&self, msg: &str) -> String {
        let mut s = String::new();
        for shift in [24, 16, 8, 0] {
            let c = ((self.chunk_name >> shift) & 0xff) as u8;
            if c < 65 || c > 122 || (c > 90 && c < 97) {
                s.push_str(&format!("[{:02X}]", c));
            } else {
                s.push(c as char);
            }
        }
        s.push_str(": ");
        let m: String = msg.chars().take(195).collect();
        s.push_str(&m);
        s
    }
    fn chunk_error<T>(&self, msg: &str) -> PngResult<T> {
        Err(self.format_chunk_msg(msg))
    }

    // ---------------------------------------------------------------- I/O
    /// png_default_read_data: fread, "Read Error" on a short read.
    fn read_data(&mut self, buf: &mut [u8]) -> PngResult<()> {
        let n = buf.len();
        if self.pos + n > self.data.len() {
            return Err("Read Error".into());
        }
        buf.copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(())
    }

    fn crc_read(&mut self, buf: &mut [u8]) -> PngResult<()> {
        self.read_data(buf)?;
        self.crc = texrs_zlib::crc32(self.crc, buf);
        Ok(())
    }

    /// png_crc_error (default flags: the CRC is always checked)
    fn crc_error(&mut self) -> PngResult<bool> {
        let mut b = [0u8; 4];
        self.read_data(&mut b)?;
        Ok(get_uint_32(&b) != self.crc)
    }

    /// png_crc_finish_critical
    fn crc_finish_critical(&mut self, mut skip: u32, handle_as_ancillary: bool) -> PngResult<bool> {
        let mut tmp = [0u8; PNG_INFLATE_BUF_SIZE];
        while skip > 0 {
            let len = (PNG_INFLATE_BUF_SIZE as u32).min(skip);
            skip -= len;
            self.crc_read(&mut tmp[..len as usize])?;
        }
        if self.crc_error()? {
            if handle_as_ancillary || chunk_ancillary(self.chunk_name) {
                // png_chunk_warning
            } else {
                return self.chunk_error("CRC error");
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn crc_finish(&mut self, skip: u32) -> PngResult<bool> {
        self.crc_finish_critical(skip, false)
    }

    /// png_read_sig
    fn read_sig(&mut self) -> PngResult<()> {
        let mut sig = [0u8; 8];
        self.read_data(&mut sig)?;
        const SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
        if sig != SIG {
            if sig[..4] != SIG[..4] {
                return Err("Not a PNG file".into());
            } else {
                return Err("PNG file corrupted by ASCII conversion".into());
            }
        }
        self.mode |= PNG_HAVE_PNG_SIGNATURE;
        Ok(())
    }

    /// png_read_chunk_header
    fn read_chunk_header(&mut self) -> PngResult<u32> {
        let mut buf = [0u8; 8];
        self.read_data(&mut buf)?;
        let length = get_uint_32(&buf);
        if length > PNG_UINT_31_MAX {
            return Err("PNG unsigned integer out of range".into());
        }
        self.chunk_name = get_uint_32(&buf[4..]);
        self.crc = texrs_zlib::crc32(0, &buf[4..8]);
        if !check_chunk_name(self.chunk_name) {
            return self.chunk_error("bad header (invalid type)");
        }
        Ok(length)
    }

    fn has_chunk(&self, name: &[u8; 4]) -> bool {
        match READ_CHUNKS.iter().position(|r| r.name == chunk(name)) {
            Some(i) => self.chunks_seen & (1 << i) != 0,
            None => false,
        }
    }

    // ---------------------------------------------------------------- png_read_info
    pub fn read_info(&mut self) -> PngResult<()> {
        self.read_sig()?;
        loop {
            let length = self.read_chunk_header()?;
            let name = self.chunk_name;
            if name == png_IDAT {
                if self.mode & PNG_HAVE_IHDR == 0 {
                    return self.chunk_error("Missing IHDR before IDAT");
                } else if self.color_type == PNG_COLOR_TYPE_PALETTE && self.mode & PNG_HAVE_PLTE == 0 {
                    return self.chunk_error("Missing PLTE before IDAT");
                }
                // else if AFTER_IDAT: benign "Too many IDATs found"
                self.mode |= PNG_HAVE_IDAT;
            } else if self.mode & PNG_HAVE_IDAT != 0 {
                self.mode |= PNG_HAVE_CHUNK_AFTER_IDAT | PNG_AFTER_IDAT;
            }
            if name == png_IHDR || name == png_IEND {
                self.handle_chunk(length)?;
            } else if name == png_IDAT {
                self.idat_size = length;
                break;
            } else {
                self.handle_chunk(length)?;
            }
        }
        Ok(())
    }

    /// png_handle_unknown with keep = AS_DEFAULT and no user callback
    fn handle_unknown(&mut self, length: u32) -> PngResult<Handled> {
        self.crc_finish(length)?;
        if !chunk_ancillary(self.chunk_name) {
            return self.chunk_error("unhandled critical chunk");
        }
        Ok(Handled::Discarded)
    }

    /// png_handle_chunk
    fn handle_chunk(&mut self, length: u32) -> PngResult<Handled> {
        let name = self.chunk_name;
        let idx = READ_CHUNKS.iter().position(|r| r.name == name);
        let mut handled = Handled::Error;
        let mut errmsg: Option<&str> = None;
        match idx {
            None => handled = self.handle_unknown(length)?,
            Some(i) if !READ_CHUNKS[i].has_handler => handled = self.handle_unknown(length)?,
            Some(i) => {
                let r = &READ_CHUNKS[i];
                if r.name != png_IHDR && self.mode & PNG_HAVE_IHDR == 0 {
                    return self.chunk_error("missing IHDR");
                } else if self.mode & r.pos_before != 0 || self.mode & r.pos_after != r.pos_after {
                    errmsg = Some("out of place");
                } else if !r.multiple && self.chunks_seen & (1 << i) != 0 {
                    errmsg = Some("duplicate");
                } else if length < r.min_length {
                    errmsg = Some("too short");
                } else {
                    let ok = match r.max_length {
                        MaxLen::Limit => {
                            if length <= self.user_chunk_malloc_max {
                                true
                            } else {
                                errmsg = Some("length exceeds libpng limit");
                                false
                            }
                        }
                        MaxLen::Max(m) => {
                            if length <= m {
                                true
                            } else {
                                errmsg = Some("too long");
                                false
                            }
                        }
                        MaxLen::NoCheck => true,
                    };
                    if ok {
                        handled = self.run_handler(&r.name.to_be_bytes(), length)?;
                    }
                }
                if let Some(m) = errmsg {
                    if !chunk_ancillary(name) {
                        return self.chunk_error(m);
                    }
                    self.crc_finish(length)?;
                    // png_chunk_benign_error -> warning
                } else if handled >= Handled::Saved {
                    self.chunks_seen |= 1 << i;
                }
            }
        }
        Ok(handled)
    }

    fn run_handler(&mut self, name: &[u8; 4], length: u32) -> PngResult<Handled> {
        match name {
            b"IHDR" => self.handle_ihdr(length),
            b"PLTE" => self.handle_plte(length),
            b"IEND" => self.handle_iend(length),
            b"gAMA" => self.handle_gama(length),
            b"sBIT" => self.handle_sbit(length),
            b"cHRM" => self.handle_chrm(length),
            b"sRGB" => self.handle_srgb(length),
            b"iCCP" => self.handle_iccp(length),
            b"sPLT" => self.handle_splt(length),
            b"tRNS" => self.handle_trns(length),
            b"bKGD" => self.handle_bkgd(length),
            b"cICP" => self.handle_fixed_valid(length, 4, PNG_INFO_cICP),
            b"cLLI" => self.handle_fixed_valid(length, 8, PNG_INFO_cLLI),
            b"mDCV" => self.handle_fixed_valid(length, 24, PNG_INFO_mDCV),
            b"eXIf" => self.handle_exif(length),
            b"hIST" => self.handle_hist(length),
            b"pHYs" => self.handle_phys(length),
            _ => self.handle_skipped(length),
        }
    }

    /// Handlers of chunks whose content pdfTeX never looks at (oFFs, pCAL, sCAL, tIME,
    /// tEXt, zTXt, iTXt): the data is consumed and CRC-checked. Their exact acceptance
    /// rules only decide whether a later duplicate is accepted, which nothing observes.
    fn handle_skipped(&mut self, length: u32) -> PngResult<Handled> {
        if self.crc_finish(length)? {
            return Ok(Handled::Error);
        }
        Ok(Handled::Ok)
    }

    /// cICP, cLLI, mDCV: fixed-length chunks stored as they are.
    fn handle_fixed_valid(&mut self, length: u32, n: usize, flag: u32) -> PngResult<Handled> {
        let _ = length;
        let mut buf = vec![0u8; n];
        self.crc_read(&mut buf)?;
        if self.crc_finish(0)? {
            return Ok(Handled::Error);
        }
        self.info.valid |= flag;
        Ok(Handled::Ok)
    }

    fn handle_ihdr(&mut self, _length: u32) -> PngResult<Handled> {
        self.mode |= PNG_HAVE_IHDR;
        let mut buf = [0u8; 13];
        self.crc_read(&mut buf)?;
        self.crc_finish(0)?;
        let width = get_uint_32(&buf);
        if width > PNG_UINT_31_MAX {
            return Err("PNG unsigned integer out of range".into());
        }
        let height = get_uint_32(&buf[4..]);
        if height > PNG_UINT_31_MAX {
            return Err("PNG unsigned integer out of range".into());
        }
        let bit_depth = buf[8];
        let color_type = buf[9];
        let compression_type = buf[10];
        let filter_type = buf[11];
        let interlace_type = buf[12];
        self.width = width;
        self.height = height;
        self.bit_depth = bit_depth;
        self.interlaced = interlace_type;
        self.color_type = color_type;
        self.filter_type = filter_type;
        self.compression_type = compression_type;
        self.channels = match color_type {
            PNG_COLOR_TYPE_RGB => 3,
            PNG_COLOR_TYPE_GRAY_ALPHA => 2,
            PNG_COLOR_TYPE_RGB_ALPHA => 4,
            _ => 1,
        };
        self.pixel_depth = bit_depth.wrapping_mul(self.channels);
        self.rowbytes = rowbytes(self.pixel_depth as u32, width);
        // png_set_IHDR
        let i = &mut self.info;
        i.width = width;
        i.height = height;
        i.bit_depth = bit_depth;
        i.color_type = color_type;
        i.compression_type = compression_type;
        i.filter_type = filter_type;
        i.interlace_type = interlace_type;
        self.check_ihdr()?;
        let i = &mut self.info;
        i.channels = if i.color_type == PNG_COLOR_TYPE_PALETTE {
            1
        } else if i.color_type & PNG_COLOR_MASK_COLOR != 0 {
            3
        } else {
            1
        };
        if i.color_type & PNG_COLOR_MASK_ALPHA != 0 {
            i.channels += 1;
        }
        i.pixel_depth = i.channels.wrapping_mul(i.bit_depth);
        i.rowbytes = rowbytes(i.pixel_depth as u32, width);
        Ok(Handled::Ok)
    }

    /// png_check_IHDR (the warnings are silent; any problem is "Invalid IHDR data")
    fn check_ihdr(&self) -> PngResult<()> {
        let i = &self.info;
        let mut error = false;
        if i.width == 0 || i.width > PNG_UINT_31_MAX || i.width > PNG_USER_WIDTH_MAX {
            error = true;
        }
        if i.height == 0 || i.height > PNG_UINT_31_MAX || i.height > PNG_USER_HEIGHT_MAX {
            error = true;
        }
        let bd = i.bit_depth;
        if bd != 1 && bd != 2 && bd != 4 && bd != 8 && bd != 16 {
            error = true;
        }
        let ct = i.color_type;
        if ct == 1 || ct == 5 || ct > 6 {
            error = true;
        }
        if (ct == PNG_COLOR_TYPE_PALETTE && bd > 8)
            || ((ct == PNG_COLOR_TYPE_RGB || ct == PNG_COLOR_TYPE_GRAY_ALPHA || ct == PNG_COLOR_TYPE_RGB_ALPHA) && bd < 8)
        {
            error = true;
        }
        if i.interlace_type >= 2 {
            error = true;
        }
        if i.compression_type != 0 {
            error = true;
        }
        if i.filter_type != 0 {
            error = true;
        }
        if error {
            return Err("Invalid IHDR data".into());
        }
        Ok(())
    }

    fn handle_plte(&mut self, length: u32) -> PngResult<Handled> {
        let errmsg = if self.mode & PNG_HAVE_PLTE != 0 {
            "duplicate"
        } else if self.mode & PNG_HAVE_IDAT != 0 {
            "out of place"
        } else if self.color_type & PNG_COLOR_MASK_COLOR == 0 {
            "ignored in grayscale PNG"
        } else if length > 3 * PNG_MAX_PALETTE_LENGTH as u32 || length % 3 != 0 {
            "invalid"
        } else if self.color_type != PNG_COLOR_TYPE_PALETTE && (self.has_chunk(b"tRNS") || self.has_chunk(b"bKGD")) {
            "out of place"
        } else {
            let max_palette_length: u32 =
                if self.color_type == PNG_COLOR_TYPE_PALETTE { 1 << self.bit_depth } else { PNG_MAX_PALETTE_LENGTH as u32 };
            let num = if length > 3 * max_palette_length { max_palette_length } else { length / 3 };
            let mut buf = vec![0u8; (num * 3) as usize];
            self.crc_read(&mut buf)?;
            self.crc_finish_critical(length - 3 * num, self.color_type != PNG_COLOR_TYPE_PALETTE)?;
            let pal: Vec<PngColor> =
                buf.chunks(3).map(|c| PngColor { red: c[0], green: c[1], blue: c[2] }).collect();
            self.mode |= PNG_HAVE_PLTE;
            self.set_plte(&pal)?;
            return Ok(Handled::Ok);
        };
        let m = errmsg;
        if self.color_type == PNG_COLOR_TYPE_PALETTE {
            self.crc_finish(length)?;
            return self.chunk_error(m);
        }
        self.crc_finish_critical(length, true)?;
        Ok(Handled::Error)
    }

    /// png_set_PLTE
    fn set_plte(&mut self, pal: &[PngColor]) -> PngResult<()> {
        let num_palette = pal.len();
        let max_palette_length =
            if self.info.color_type == PNG_COLOR_TYPE_PALETTE { 1usize << self.info.bit_depth } else { PNG_MAX_PALETTE_LENGTH };
        if num_palette > max_palette_length {
            if self.info.color_type == PNG_COLOR_TYPE_PALETTE {
                return Err("Invalid palette length".into());
            }
            return Ok(());
        }
        if num_palette == 0 {
            return Err("Invalid palette".into());
        }
        let mut p = vec![PngColor::default(); PNG_MAX_PALETTE_LENGTH];
        p[..num_palette].copy_from_slice(pal);
        self.palette = p;
        self.num_palette = num_palette as u16;
        self.info.num_palette = num_palette as u16;
        self.info.valid |= PNG_INFO_PLTE;
        Ok(())
    }

    fn handle_iend(&mut self, length: u32) -> PngResult<Handled> {
        self.mode |= PNG_AFTER_IDAT | PNG_HAVE_IEND;
        self.crc_finish_critical(length, true)?;
        Ok(Handled::Ok)
    }

    fn handle_gama(&mut self, _length: u32) -> PngResult<Handled> {
        let mut buf = [0u8; 4];
        self.crc_read(&mut buf)?;
        if self.crc_finish(0)? {
            return Ok(Handled::Error);
        }
        let ugamma = get_uint_32(&buf);
        if ugamma > PNG_UINT_31_MAX {
            return Ok(Handled::Error);
        }
        self.info.gamma = ugamma as i32;
        self.info.valid |= PNG_INFO_gAMA;
        if self.chunk_gamma == 0 {
            self.chunk_gamma = ugamma as i32;
        }
        Ok(Handled::Ok)
    }

    fn handle_sbit(&mut self, length: u32) -> PngResult<Handled> {
        let (truelen, sample_depth) = if self.color_type == PNG_COLOR_TYPE_PALETTE {
            (3u32, 8u8)
        } else {
            (self.channels as u32, self.bit_depth)
        };
        if length != truelen {
            self.crc_finish(length)?;
            return Ok(Handled::Error);
        }
        let mut buf = [sample_depth; 4];
        self.crc_read(&mut buf[..truelen as usize])?;
        if self.crc_finish(0)? {
            return Ok(Handled::Error);
        }
        for &b in &buf[..truelen as usize] {
            if b == 0 || b > sample_depth {
                return Ok(Handled::Error);
            }
        }
        if self.color_type & PNG_COLOR_MASK_COLOR != 0 {
            self.sig_bit.red = buf[0];
            self.sig_bit.green = buf[1];
            self.sig_bit.blue = buf[2];
            self.sig_bit.alpha = buf[3];
        } else {
            self.sig_bit.gray = buf[0];
            self.sig_bit.red = buf[0];
            self.sig_bit.green = buf[0];
            self.sig_bit.blue = buf[0];
            self.sig_bit.alpha = buf[1];
        }
        self.info.sig_bit = self.sig_bit;
        self.info.valid |= PNG_INFO_sBIT;
        Ok(Handled::Ok)
    }

    fn handle_chrm(&mut self, _length: u32) -> PngResult<Handled> {
        let mut buf = [0u8; 32];
        self.crc_read(&mut buf)?;
        if self.crc_finish(0)? {
            return Ok(Handled::Error);
        }
        let mut error = false;
        for k in 0..8 {
            get_int_32_checked(&buf[4 * k..], &mut error);
        }
        if error {
            return Ok(Handled::Error);
        }
        self.info.valid |= PNG_INFO_cHRM;
        Ok(Handled::Ok)
    }

    fn handle_srgb(&mut self, _length: u32) -> PngResult<Handled> {
        let mut intent = [0u8; 1];
        self.crc_read(&mut intent)?;
        if self.crc_finish(0)? {
            return Ok(Handled::Error);
        }
        if intent[0] > 3 {
            return Ok(Handled::Error);
        }
        self.info.rendering_intent = intent[0] as i32;
        self.info.valid |= PNG_INFO_sRGB;
        if !self.has_chunk(b"cICP") || self.chunk_gamma == 0 {
            self.chunk_gamma = PNG_GAMMA_sRGB_INVERSE;
        }
        Ok(Handled::Ok)
    }

    /// png_inflate_claim
    fn inflate_claim(&mut self, owner: u32) -> PngResult<bool> {
        if self.zowner != 0 {
            self.zowner = 0; // release build: warning
        }
        let window_bits = if self.max_inflate_window {
            self.zstream_start = false;
            15
        } else {
            self.zstream_start = true;
            0
        };
        self.zbuf.clear();
        self.zpos = 0;
        let ok = if self.zstream_initialized {
            let s = self.zstream.as_mut().unwrap().reset2(window_bits);
            s == Status::Ok
        } else {
            match Inflate::with_window_bits(window_bits) {
                Ok(z) => {
                    self.zstream = Some(z);
                    self.zstream_initialized = true;
                    true
                }
                Err(_) => false,
            }
        };
        if ok {
            self.zowner = owner;
        }
        Ok(ok)
    }

    /// One `inflate()` call (png_zlib_inflate) over zbuf[zpos..] into out; returns
    /// (status, produced).
    fn zinflate(&mut self, out: &mut [u8], flush: Flush) -> (Status, usize) {
        if self.zstream_start && self.zpos < self.zbuf.len() {
            if (self.zbuf[self.zpos] >> 4) > 7 {
                return (Status::DataError, 0);
            }
            self.zstream_start = false;
        }
        let z = self.zstream.as_mut().unwrap();
        let r = z.inflate(&self.zbuf[self.zpos..], out, flush);
        self.zpos += r.consumed;
        (r.status, r.produced)
    }

    /// png_inflate_read: fill out[..] (reading more chunk data as needed); returns the
    /// zlib status and how much of `out` is still unfilled.
    fn inflate_read(&mut self, chunk_bytes: &mut u32, out: &mut [u8], finish: bool) -> PngResult<(Status, usize)> {
        let mut produced_total = 0usize;
        let mut ret;
        loop {
            if self.zpos >= self.zbuf.len() {
                let read_size = (PNG_INFLATE_BUF_SIZE as u32).min(*chunk_bytes);
                *chunk_bytes -= read_size;
                let mut b = vec![0u8; read_size as usize];
                if read_size > 0 {
                    self.crc_read(&mut b)?;
                }
                self.zbuf = b;
                self.zpos = 0;
            }
            let flush = if *chunk_bytes > 0 {
                Flush::NoFlush
            } else if finish {
                Flush::Finish
            } else {
                Flush::SyncFlush
            };
            let (st, n) = self.zinflate(&mut out[produced_total..], flush);
            produced_total += n;
            ret = st;
            if !(ret == Status::Ok && produced_total < out.len()) {
                break;
            }
        }
        Ok((ret, out.len() - produced_total))
    }

    fn handle_iccp(&mut self, length: u32) -> PngResult<Handled> {
        let mut length = length;
        let read_length = 81u32.min(length);
        let mut keyword = vec![0u8; read_length as usize];
        self.crc_read(&mut keyword)?;
        length -= read_length;
        if length < LZ77_MIN {
            self.crc_finish(length)?;
            return Ok(Handled::Error);
        }
        let mut kl = 0usize;
        while kl < 80 && kl < read_length as usize && keyword[kl] != 0 {
            kl += 1;
        }
        let mut finished = false;
        let mut ok = false;
        if (1..=79).contains(&kl) && kl + 1 < read_length as usize && keyword[kl + 1] == 0 {
            if self.inflate_claim(png_iCCP)? {
                self.zbuf = keyword[kl + 2..].to_vec();
                self.zpos = 0;
                let mut header = [0u8; 132];
                let (_, left) = self.inflate_read(&mut length, &mut header, false)?;
                if left == 0 {
                    let profile_length = get_uint_32(&header);
                    if profile_length >= 132 && profile_length <= self.user_chunk_malloc_max {
                        if icc_check_header(profile_length, &header, self.color_type) {
                            let tag_count = get_uint_32(&header[128..]);
                            let mut tags = vec![0u8; 12 * tag_count as usize];
                            let (_, left) = self.inflate_read(&mut length, &mut tags, false)?;
                            if left == 0 {
                                let mut profile = header.to_vec();
                                profile.extend_from_slice(&tags);
                                if icc_check_tag_table(profile_length, &profile) {
                                    let rest = profile_length as usize - 132 - 12 * tag_count as usize;
                                    let mut body = vec![0u8; rest];
                                    let (_, left) = self.inflate_read(&mut length, &mut body, true)?;
                                    if left == 0 {
                                        self.crc_finish(length)?;
                                        finished = true;
                                        self.info.valid |= PNG_INFO_iCCP;
                                        ok = true;
                                    }
                                }
                            }
                        }
                    }
                }
                self.zowner = 0;
            }
        }
        if ok {
            return Ok(Handled::Ok);
        }
        if !finished {
            self.crc_finish(length)?;
        }
        Ok(Handled::Error)
    }

    fn handle_splt(&mut self, length: u32) -> PngResult<Handled> {
        if self.user_chunk_cache_max != 0 {
            if self.user_chunk_cache_max == 1 {
                self.crc_finish(length)?;
                return Ok(Handled::Error);
            }
            self.user_chunk_cache_max -= 1;
            if self.user_chunk_cache_max == 1 {
                self.crc_finish(length)?;
                return Ok(Handled::Error);
            }
        }
        if length as u64 + 1 > self.user_chunk_malloc_max as u64 {
            self.crc_finish(length)?;
            return Ok(Handled::Error);
        }
        let mut buffer = vec![0u8; length as usize + 1];
        self.crc_read(&mut buffer[..length as usize])?;
        if self.crc_finish(0)? {
            return Ok(Handled::Error);
        }
        buffer[length as usize] = 0;
        let mut e = 0usize;
        while buffer[e] != 0 {
            e += 1;
        }
        e += 1;
        if length < 2 || e > (length - 2) as usize {
            return Ok(Handled::Error);
        }
        let depth = buffer[e];
        e += 1;
        let entry_size = if depth == 8 { 6 } else { 10 };
        let data_length = length as usize - e;
        if data_length % entry_size != 0 {
            return Ok(Handled::Error);
        }
        let nentries = data_length / entry_size;
        if nentries == 0 {
            // png_malloc_warn(0) fails: "sPLT chunk requires too much memory"
            return Ok(Handled::Error);
        }
        self.info.valid |= PNG_INFO_sPLT;
        Ok(Handled::Ok)
    }

    fn handle_trns(&mut self, length: u32) -> PngResult<Handled> {
        let mut readbuf = [0u8; PNG_MAX_PALETTE_LENGTH];
        if self.color_type == PNG_COLOR_TYPE_GRAY {
            if length != 2 {
                self.crc_finish(length)?;
                return Ok(Handled::Error);
            }
            let mut buf = [0u8; 2];
            self.crc_read(&mut buf)?;
            self.num_trans = 1;
            self.trans_color.gray = get_uint_16(&buf);
        } else if self.color_type == PNG_COLOR_TYPE_RGB {
            if length != 6 {
                self.crc_finish(length)?;
                return Ok(Handled::Error);
            }
            let mut buf = [0u8; 6];
            self.crc_read(&mut buf)?;
            self.num_trans = 1;
            self.trans_color.red = get_uint_16(&buf);
            self.trans_color.green = get_uint_16(&buf[2..]);
            self.trans_color.blue = get_uint_16(&buf[4..]);
        } else if self.color_type == PNG_COLOR_TYPE_PALETTE {
            if self.mode & PNG_HAVE_PLTE == 0 {
                self.crc_finish(length)?;
                return Ok(Handled::Error);
            }
            if length > self.num_palette as u32 || length > PNG_MAX_PALETTE_LENGTH as u32 || length == 0 {
                self.crc_finish(length)?;
                return Ok(Handled::Error);
            }
            self.crc_read(&mut readbuf[..length as usize])?;
            self.num_trans = length as u16;
        } else {
            self.crc_finish(length)?;
            return Ok(Handled::Error);
        }
        if self.crc_finish(0)? {
            self.num_trans = 0;
            return Ok(Handled::Error);
        }
        // png_set_tRNS(png_ptr, info_ptr, readbuf, num_trans, &trans_color)
        let mut num_trans = self.num_trans as usize;
        if num_trans > 0 && num_trans <= PNG_MAX_PALETTE_LENGTH {
            let mut t = [0u8; 256];
            t[..num_trans].copy_from_slice(&readbuf[..num_trans]);
            self.trans_alpha = t;
            self.info.valid |= PNG_INFO_tRNS;
        }
        self.info.trans_color = self.trans_color;
        if num_trans == 0 {
            num_trans = 1;
        }
        self.info.num_trans = num_trans as u16;
        self.info.valid |= PNG_INFO_tRNS;
        Ok(Handled::Ok)
    }

    fn handle_bkgd(&mut self, length: u32) -> PngResult<Handled> {
        let truelen = if self.color_type == PNG_COLOR_TYPE_PALETTE {
            if self.mode & PNG_HAVE_PLTE == 0 {
                self.crc_finish(length)?;
                return Ok(Handled::Error);
            }
            1
        } else if self.color_type & PNG_COLOR_MASK_COLOR != 0 {
            6
        } else {
            2
        };
        if length != truelen {
            self.crc_finish(length)?;
            return Ok(Handled::Error);
        }
        let mut buf = [0u8; 6];
        self.crc_read(&mut buf[..truelen as usize])?;
        if self.crc_finish(0)? {
            return Ok(Handled::Error);
        }
        let mut bg = PngColor16::default();
        if self.color_type == PNG_COLOR_TYPE_PALETTE {
            bg.index = buf[0];
            if self.info.num_palette != 0 {
                if buf[0] as u16 >= self.info.num_palette {
                    return Ok(Handled::Error);
                }
                let c = self.palette[buf[0] as usize];
                bg.red = c.red as u16;
                bg.green = c.green as u16;
                bg.blue = c.blue as u16;
            }
        } else if self.color_type & PNG_COLOR_MASK_COLOR == 0 {
            if self.bit_depth <= 8 && (buf[0] != 0 || buf[1] as u32 >= (1u32 << self.bit_depth)) {
                return Ok(Handled::Error);
            }
            let g = get_uint_16(&buf);
            bg.red = g;
            bg.green = g;
            bg.blue = g;
            bg.gray = g;
        } else {
            if self.bit_depth <= 8 && (buf[0] != 0 || buf[2] != 0 || buf[4] != 0) {
                return Ok(Handled::Error);
            }
            bg.red = get_uint_16(&buf);
            bg.green = get_uint_16(&buf[2..]);
            bg.blue = get_uint_16(&buf[4..]);
        }
        self.info.background = bg;
        self.info.valid |= PNG_INFO_bKGD;
        Ok(Handled::Ok)
    }

    fn handle_exif(&mut self, length: u32) -> PngResult<Handled> {
        let mut buffer = vec![0u8; length as usize];
        self.crc_read(&mut buffer)?;
        if self.crc_finish(0)? {
            return Ok(Handled::Error);
        }
        let header = get_uint_32(&buffer);
        if header != 0x4949_2A00 && header != 0x4D4D_002A {
            return Ok(Handled::Error);
        }
        self.info.valid |= PNG_INFO_eXIf;
        Ok(Handled::Ok)
    }

    fn handle_hist(&mut self, length: u32) -> PngResult<Handled> {
        let num = length / 2;
        if length != num * 2 || num != self.num_palette as u32 || num > PNG_MAX_PALETTE_LENGTH as u32 {
            self.crc_finish(length)?;
            return Ok(Handled::Error);
        }
        let mut buf = vec![0u8; length as usize];
        self.crc_read(&mut buf)?;
        if self.crc_finish(0)? {
            return Ok(Handled::Error);
        }
        // png_set_hIST
        if self.info.num_palette == 0 || self.info.num_palette as usize > PNG_MAX_PALETTE_LENGTH {
            return Ok(Handled::Ok);
        }
        self.info.valid |= PNG_INFO_hIST;
        Ok(Handled::Ok)
    }

    fn handle_phys(&mut self, _length: u32) -> PngResult<Handled> {
        let mut buf = [0u8; 9];
        self.crc_read(&mut buf)?;
        if self.crc_finish(0)? {
            return Ok(Handled::Error);
        }
        self.info.x_pixels_per_unit = get_uint_32(&buf);
        self.info.y_pixels_per_unit = get_uint_32(&buf[4..]);
        self.info.phys_unit_type = buf[8];
        self.info.valid |= PNG_INFO_pHYs;
        Ok(Handled::Ok)
    }

    // ---------------------------------------------------------------- getters (pngget.c)
    pub fn get_valid(&self, flag: u32) -> u32 {
        if flag == PNG_INFO_tRNS && self.num_trans == 0 {
            return 0;
        }
        self.info.valid & flag
    }
    pub fn get_x_pixels_per_meter(&self) -> u32 {
        if self.info.valid & PNG_INFO_pHYs != 0 && self.info.phys_unit_type == PNG_RESOLUTION_METER {
            return self.info.x_pixels_per_unit;
        }
        0
    }
    pub fn get_y_pixels_per_meter(&self) -> u32 {
        if self.info.valid & PNG_INFO_pHYs != 0 && self.info.phys_unit_type == PNG_RESOLUTION_METER {
            return self.info.y_pixels_per_unit;
        }
        0
    }
    /// png_get_gAMA_fixed / png_get_gAMA (png_float: value * .00001)
    pub fn get_gama_fixed(&self) -> Option<i32> {
        if self.info.valid & PNG_INFO_gAMA != 0 {
            Some(self.info.gamma)
        } else {
            None
        }
    }
    /// png_get_PLTE: (palette, num_palette) when PLTE is valid.
    pub fn get_plte(&self) -> Option<(&[PngColor], i32)> {
        if self.info.valid & PNG_INFO_PLTE != 0 {
            Some((&self.palette, self.info.num_palette as i32))
        } else {
            None
        }
    }

    // ---------------------------------------------------------------- transformations setup
    fn rtran_ok(&self) -> PngResult<bool> {
        if self.row_init {
            return Err("invalid after png_start_read_image or png_read_update_info".into());
        }
        Ok(true)
    }
    pub fn set_trns_to_alpha(&mut self) -> PngResult<()> {
        if self.rtran_ok()? {
            self.transformations |= PNG_EXPAND | PNG_EXPAND_tRNS;
        }
        Ok(())
    }
    pub fn set_strip_alpha(&mut self) -> PngResult<()> {
        if self.rtran_ok()? {
            self.transformations |= PNG_STRIP_ALPHA;
        }
        Ok(())
    }
    pub fn set_strip_16(&mut self) -> PngResult<()> {
        if self.rtran_ok()? {
            self.transformations |= PNG_16_TO_8;
        }
        Ok(())
    }
    /// png_set_gamma (floating point API) -> png_set_gamma_fixed
    pub fn set_gamma(&mut self, scrn_gamma: f64, file_gamma: f64) -> PngResult<()> {
        let s = convert_gamma_value(scrn_gamma)?;
        let f = convert_gamma_value(file_gamma)?;
        if !self.rtran_ok()? {
            return Ok(());
        }
        let s = translate_gamma_flags(s, true);
        let f = translate_gamma_flags(f, false);
        if f <= 0 {
            return Err("invalid file gamma in png_set_gamma".into());
        }
        if s <= 0 {
            return Err("invalid screen gamma in png_set_gamma".into());
        }
        if unsupported_gamma(f) || unsupported_gamma(s) {
            return Ok(());
        }
        self.file_gamma = f;
        self.screen_gamma = s;
        Ok(())
    }
    pub fn set_interlace_handling(&mut self) -> i32 {
        if self.interlaced != 0 {
            self.transformations |= PNG_INTERLACE;
            return 7;
        }
        1
    }

    /// png_read_update_info
    pub fn read_update_info(&mut self) -> PngResult<()> {
        if !self.row_init {
            self.read_start_row()?;
            self.read_transform_info()?;
        } else {
            return Err("png_read_update_info/png_start_read_image: duplicate call".into());
        }
        Ok(())
    }

    /// png_resolve_file_gamma + png_init_gamma_values
    fn init_gamma_values(&mut self) -> bool {
        let mut file_gamma = self.file_gamma;
        if file_gamma == 0 {
            file_gamma = self.chunk_gamma;
        }
        if file_gamma == 0 {
            file_gamma = self.default_gamma;
        }
        if file_gamma == 0 && self.screen_gamma != 0 {
            file_gamma = png_reciprocal(self.screen_gamma);
        }
        let mut screen_gamma = self.screen_gamma;
        let mut gamma_correction = false;
        if file_gamma > 0 {
            if screen_gamma > 0 {
                gamma_correction = png_gamma_threshold(file_gamma, screen_gamma);
            } else {
                screen_gamma = png_reciprocal(file_gamma);
            }
        } else {
            file_gamma = PNG_FP_1;
            screen_gamma = PNG_FP_1;
        }
        self.file_gamma = file_gamma;
        self.screen_gamma = screen_gamma;
        gamma_correction
    }

    /// png_init_read_transformations, restricted to the transformations pdfTeX can set
    /// (no background, alpha mode, rgb-to-gray, shift, quantize).
    fn init_read_transformations(&mut self) {
        if self.init_gamma_values() {
            self.transformations |= PNG_GAMMA;
        } else {
            self.transformations &= !PNG_GAMMA;
        }
        if self.transformations & PNG_STRIP_ALPHA != 0 && self.transformations & PNG_COMPOSE == 0 {
            self.transformations &= !(PNG_BACKGROUND_EXPAND | PNG_ENCODE_ALPHA | PNG_EXPAND_tRNS);
            self.num_trans = 0;
        }
        // palette/rgb init and the background/alpha mode parts only touch flags that are
        // never set here.
        if self.transformations & PNG_GAMMA != 0 {
            self.build_gamma_table(self.bit_depth);
            if self.color_type == PNG_COLOR_TYPE_PALETTE {
                let t = self.gamma_table.as_ref().unwrap();
                for i in 0..self.num_palette as usize {
                    let c = &mut self.palette[i];
                    c.red = t[c.red as usize];
                    c.green = t[c.green as usize];
                    c.blue = t[c.blue as usize];
                }
                self.transformations &= !PNG_GAMMA;
            }
        }
    }

    /// png_build_gamma_table (png.c)
    fn build_gamma_table(&mut self, bit_depth: u8) {
        self.gamma_table = None;
        self.gamma_16_table = None;
        let file_gamma = self.file_gamma;
        let screen_gamma = self.screen_gamma;
        let correction = if screen_gamma > 0 { png_reciprocal2(screen_gamma, file_gamma) } else { PNG_FP_1 };
        if bit_depth <= 8 {
            self.gamma_table = Some(build_8bit_table(correction));
        } else {
            let sig_bit = if self.color_type & PNG_COLOR_MASK_COLOR != 0 {
                self.sig_bit.red.max(self.sig_bit.green).max(self.sig_bit.blue)
            } else {
                self.sig_bit.gray
            } as u32;
            let mut shift = if sig_bit > 0 && sig_bit < 16 { (16 - sig_bit) & 0xff } else { 0 };
            if self.transformations & (PNG_16_TO_8 | PNG_SCALE_16_TO_8) != 0 && shift < 16 - PNG_MAX_GAMMA_8 {
                shift = 16 - PNG_MAX_GAMMA_8;
            }
            if shift > 8 {
                shift = 8;
            }
            self.gamma_shift = shift;
            if self.transformations & (PNG_16_TO_8 | PNG_SCALE_16_TO_8) != 0 {
                self.gamma_16_table = Some(build_16to8_table(shift, png_reciprocal(correction)));
            } else {
                self.gamma_16_table = Some(build_16bit_table(shift, correction));
            }
        }
    }

    /// png_read_start_row
    fn read_start_row(&mut self) -> PngResult<()> {
        self.init_read_transformations();
        if self.interlaced != 0 {
            if self.transformations & PNG_INTERLACE == 0 {
                self.num_rows = (self.height + PASS_YINC[0] - 1 - PASS_YSTART[0]) / PASS_YINC[0];
            } else {
                self.num_rows = self.height;
            }
            let p = self.pass as usize;
            self.iwidth = (self.width + PASS_INC[p] - 1 - PASS_START[p]) / PASS_INC[p];
        } else {
            self.num_rows = self.height;
            self.iwidth = self.width;
        }
        let mut max_pixel_depth = self.pixel_depth as u32;
        if self.transformations & PNG_EXPAND != 0 {
            if self.color_type == PNG_COLOR_TYPE_PALETTE {
                max_pixel_depth = if self.num_trans != 0 { 32 } else { 24 };
            } else if self.color_type == PNG_COLOR_TYPE_GRAY {
                if max_pixel_depth < 8 {
                    max_pixel_depth = 8;
                }
                if self.num_trans != 0 {
                    max_pixel_depth *= 2;
                }
            } else if self.color_type == PNG_COLOR_TYPE_RGB && self.num_trans != 0 {
                max_pixel_depth *= 4;
                max_pixel_depth /= 3;
            }
        }
        self.maximum_pixel_depth = max_pixel_depth as u8;
        self.transformed_pixel_depth = 0;
        let rb = ((self.width + 7) & !7u32) as u32;
        let row_bytes = rowbytes(max_pixel_depth, rb) + 1 + ((max_pixel_depth as usize + 7) >> 3);
        self.row_buf = vec![0u8; row_bytes + 48];
        self.prev_row = vec![0u8; row_bytes + 48];
        if !self.inflate_claim(png_IDAT)? {
            return Err("zstream unclaimed".into());
        }
        self.row_init = true;
        Ok(())
    }

    /// png_read_transform_info
    fn read_transform_info(&mut self) -> PngResult<()> {
        let tr = self.transformations;
        let i = &mut self.info;
        if tr & PNG_EXPAND != 0 {
            if i.color_type == PNG_COLOR_TYPE_PALETTE {
                i.color_type = if self.num_trans > 0 { PNG_COLOR_TYPE_RGB_ALPHA } else { PNG_COLOR_TYPE_RGB };
                i.bit_depth = 8;
                i.num_trans = 0;
                if self.palette.is_empty() {
                    return Err("Palette is NULL in indexed image".into());
                }
            } else {
                if self.num_trans != 0 && tr & PNG_EXPAND_tRNS != 0 {
                    i.color_type |= PNG_COLOR_MASK_ALPHA;
                }
                if i.bit_depth < 8 {
                    i.bit_depth = 8;
                }
                i.num_trans = 0;
            }
        }
        i.gamma = self.file_gamma;
        if i.bit_depth == 16 && tr & (PNG_SCALE_16_TO_8 | PNG_16_TO_8) != 0 {
            i.bit_depth = 8;
        }
        i.channels = if i.color_type == PNG_COLOR_TYPE_PALETTE {
            1
        } else if i.color_type & PNG_COLOR_MASK_COLOR != 0 {
            3
        } else {
            1
        };
        if tr & PNG_STRIP_ALPHA != 0 {
            i.color_type &= !PNG_COLOR_MASK_ALPHA;
            i.num_trans = 0;
        }
        if i.color_type & PNG_COLOR_MASK_ALPHA != 0 {
            i.channels += 1;
        }
        i.pixel_depth = i.channels * i.bit_depth;
        i.rowbytes = rowbytes(i.pixel_depth as u32, i.width);
        self.info_rowbytes = i.rowbytes;
        Ok(())
    }

    // ---------------------------------------------------------------- reading rows
    /// png_read_row(png_ptr, row, NULL)
    pub fn read_row(&mut self, row: &mut [u8]) -> PngResult<()> {
        if !self.row_init {
            self.read_start_row()?;
        }
        let mut ri = RowInfo {
            width: self.iwidth,
            color_type: self.color_type,
            bit_depth: self.bit_depth,
            channels: self.channels,
            pixel_depth: self.pixel_depth,
            rowbytes: 0,
        };
        ri.rowbytes = rowbytes(ri.pixel_depth as u32, ri.width);

        if self.interlaced != 0 && self.transformations & PNG_INTERLACE != 0 {
            let rn = self.row_number;
            let skip = match self.pass {
                0 => rn & 7 != 0,
                1 => rn & 7 != 0 || self.width < 5,
                2 => rn & 7 != 4,
                3 => rn & 3 != 0 || self.width < 3,
                4 => rn & 3 != 2,
                5 => rn & 1 != 0 || self.width < 2,
                _ => rn & 1 == 0,
            };
            if skip {
                self.read_finish_row()?;
                return Ok(());
            }
        }
        if self.mode & PNG_HAVE_IDAT == 0 {
            return Err("Invalid attempt to read row data".into());
        }
        self.row_buf[0] = 255;
        let n = ri.rowbytes + 1;
        let mut out = std::mem::take(&mut self.row_buf);
        let r = self.read_idat_data(Some(&mut out[..n]));
        self.row_buf = out;
        r?;
        let filter = self.row_buf[0];
        if filter > 0 {
            if filter < 5 {
                read_filter_row(&ri, &mut self.row_buf[1..], &self.prev_row[1..], filter);
            } else {
                return Err("bad adaptive filter value".into());
            }
        }
        self.prev_row[..n].copy_from_slice(&self.row_buf[..n]);

        // num_palette_max >= 0 (CHECK_FOR_INVALID_INDEX): always transform
        self.do_read_transformations(&mut ri);

        if self.transformed_pixel_depth == 0 {
            self.transformed_pixel_depth = ri.pixel_depth;
            if ri.pixel_depth > self.maximum_pixel_depth {
                return Err("sequential row overflow".into());
            }
        } else if self.transformed_pixel_depth != ri.pixel_depth {
            return Err("internal sequential row size calculation error".into());
        }

        self.combine_row(row, &ri)?;
        self.read_finish_row()?;
        Ok(())
    }

    /// png_read_image
    pub fn read_image(&mut self, rows: &mut [Vec<u8>]) -> PngResult<()> {
        let pass = if !self.row_init {
            let p = self.set_interlace_handling();
            self.read_start_row()?;
            p
        } else {
            if self.interlaced != 0 && self.transformations & PNG_INTERLACE == 0 {
                self.num_rows = self.height;
            }
            self.set_interlace_handling()
        };
        let h = self.height as usize;
        for _ in 0..pass {
            for i in 0..h {
                let mut r = std::mem::take(&mut rows[i]);
                let res = self.read_row(&mut r);
                rows[i] = r;
                res?;
            }
        }
        Ok(())
    }

    /// png_combine_row(png_ptr, dp, 0 or -1) with the pass pixels of row_buf; the
    /// pass row is placed directly (png_do_read_interlace + the sparkle copy).
    fn combine_row(&mut self, dp: &mut [u8], ri: &RowInfo) -> PngResult<()> {
        let pixel_depth = self.transformed_pixel_depth as usize;
        if pixel_depth == 0 {
            return Err("internal row logic error".into());
        }
        let row_width = self.width as usize;
        if self.info_rowbytes != 0 && self.info_rowbytes != rowbytes(pixel_depth as u32, row_width as u32) {
            return Err("internal row size calculation error".into());
        }
        let sp = &self.row_buf[1..];
        let total = rowbytes(pixel_depth as u32, row_width as u32);
        let end_mask_bits = (pixel_depth * row_width) & 7;
        let (end_idx, end_byte, end_mask) = if end_mask_bits != 0 {
            let idx = total - 1;
            (Some(idx), dp[idx], 0xffu32 >> end_mask_bits)
        } else {
            (None, 0, 0)
        };
        let interlaced = self.interlaced != 0 && self.transformations & PNG_INTERLACE != 0;
        let pass = self.pass as usize;
        if interlaced && pass < 6 {
            let start = PASS_START[pass] as usize;
            let inc = PASS_INC[pass] as usize;
            if row_width <= start {
                return Ok(());
            }
            let n = ri.width as usize; // pixels in the pass row
            if pixel_depth >= 8 {
                let bpp = pixel_depth / 8;
                for k in 0..n {
                    let x = start + k * inc;
                    if x >= row_width {
                        break;
                    }
                    dp[x * bpp..x * bpp + bpp].copy_from_slice(&sp[k * bpp..k * bpp + bpp]);
                }
            } else {
                let pd = pixel_depth;
                let mask = ((1u32 << pd) - 1) as u8;
                for k in 0..n {
                    let x = start + k * inc;
                    if x >= row_width {
                        break;
                    }
                    let sbit = k * pd;
                    let v = (sp[sbit / 8] >> (8 - pd - (sbit % 8))) & mask;
                    let dbit = x * pd;
                    let sh = 8 - pd - (dbit % 8);
                    let b = &mut dp[dbit / 8];
                    *b = (*b & !(mask << sh)) | (v << sh);
                }
            }
        } else {
            dp[..total].copy_from_slice(&sp[..total]);
        }
        if let Some(idx) = end_idx {
            dp[idx] = ((end_byte as u32 & end_mask) | (dp[idx] as u32 & !end_mask)) as u8;
        }
        Ok(())
    }

    /// png_read_IDAT_data (output None = the end-of-image check)
    fn read_idat_data(&mut self, mut output: Option<&mut [u8]>) -> PngResult<()> {
        let mut out_pos = 0usize;
        let mut avail_out = match &output {
            Some(o) => o.len(),
            None => 0,
        };
        loop {
            if self.zpos >= self.zbuf.len() {
                while self.idat_size == 0 {
                    self.crc_finish(0)?;
                    self.idat_size = self.read_chunk_header()?;
                    if self.chunk_name != png_IDAT {
                        return Err("Not enough image data".into());
                    }
                }
                let mut avail_in = PNG_IDAT_READ_SIZE;
                if avail_in > self.user_chunk_malloc_max {
                    avail_in = self.user_chunk_malloc_max;
                }
                if avail_in > self.idat_size {
                    avail_in = self.idat_size;
                }
                let mut b = vec![0u8; avail_in as usize];
                self.crc_read(&mut b)?;
                self.idat_size -= avail_in;
                self.zbuf = b;
                self.zpos = 0;
            }
            let st = match output.as_deref_mut() {
                Some(o) => {
                    let out = ZLIB_IO_MAX.min(avail_out);
                    avail_out -= out;
                    let (st, n) = self.zinflate(&mut o[out_pos..out_pos + out], Flush::NoFlush);
                    out_pos += n;
                    avail_out += out - n;
                    st
                }
                None => {
                    // after the last row: count the extra bytes
                    let mut tmp = [0u8; PNG_INFLATE_BUF_SIZE];
                    let (st, n) = self.zinflate(&mut tmp, Flush::NoFlush);
                    avail_out += n;
                    st
                }
            };
            if st == Status::StreamEnd {
                self.mode |= PNG_AFTER_IDAT;
                self.zstream_ended = true;
                // benign "Extra compressed data" when input remains
                break;
            }
            if st != Status::Ok {
                let msg = zstream_msg(st, self.zstream.as_ref().and_then(|z| z.msg()));
                if output.is_some() {
                    return self.chunk_error(&msg);
                }
                return Ok(()); // benign
            }
            if avail_out == 0 {
                break;
            }
        }
        if avail_out > 0 && output.is_some() {
            return Err("Not enough image data".into());
        }
        Ok(())
    }

    /// png_read_finish_IDAT
    fn read_finish_idat(&mut self) -> PngResult<()> {
        if !self.zstream_ended {
            self.read_idat_data(None)?;
            if !self.zstream_ended {
                self.mode |= PNG_AFTER_IDAT;
                self.zstream_ended = true;
            }
        }
        if self.zowner == png_IDAT {
            self.zbuf.clear();
            self.zpos = 0;
            self.zowner = 0;
            let n = self.idat_size;
            self.crc_finish(n)?;
        }
        Ok(())
    }

    /// png_read_finish_row
    fn read_finish_row(&mut self) -> PngResult<()> {
        self.row_number += 1;
        if self.row_number < self.num_rows {
            return Ok(());
        }
        if self.interlaced != 0 {
            self.row_number = 0;
            let n = self.rowbytes + 1;
            for b in &mut self.prev_row[..n] {
                *b = 0;
            }
            loop {
                self.pass += 1;
                if self.pass >= 7 {
                    break;
                }
                let p = self.pass as usize;
                self.iwidth = (self.width + PASS_INC[p] - 1 - PASS_START[p]) / PASS_INC[p];
                if self.transformations & PNG_INTERLACE == 0 {
                    self.num_rows = (self.height + PASS_YINC[p] - 1 - PASS_YSTART[p]) / PASS_YINC[p];
                } else {
                    break;
                }
                if !(self.num_rows == 0 || self.iwidth == 0) {
                    break;
                }
            }
            if self.pass < 7 {
                return Ok(());
            }
        }
        self.read_finish_idat()
    }

    /// png_do_read_transformations for the transformations pdfTeX uses.
    fn do_read_transformations(&mut self, ri: &mut RowInfo) {
        let tr = self.transformations;
        let row = &mut self.row_buf[1..];
        if tr & PNG_EXPAND != 0 {
            if ri.color_type == PNG_COLOR_TYPE_PALETTE {
                do_expand_palette(ri, row, &self.palette, &self.trans_alpha, self.num_trans as usize);
            } else if self.num_trans != 0 && tr & PNG_EXPAND_tRNS != 0 {
                do_expand(ri, row, Some(&self.trans_color));
            } else {
                do_expand(ri, row, None);
            }
        }
        if tr & PNG_STRIP_ALPHA != 0
            && tr & PNG_COMPOSE == 0
            && (ri.color_type == PNG_COLOR_TYPE_RGB_ALPHA || ri.color_type == PNG_COLOR_TYPE_GRAY_ALPHA)
        {
            do_strip_channel(ri, row, false);
        }
        if tr & PNG_GAMMA != 0 && self.color_type != PNG_COLOR_TYPE_PALETTE {
            do_gamma(ri, row, self.gamma_table.as_deref(), self.gamma_16_table.as_deref(), self.gamma_shift);
        }
        if tr & PNG_16_TO_8 != 0 {
            do_chop(ri, row);
        }
    }
}

/// zlib z_stream msg for an inflate status (png_zstream_error fallbacks).
fn zstream_msg(st: Status, msg: Option<&'static str>) -> String {
    if let Some(m) = msg {
        return m.to_string();
    }
    match st {
        Status::Ok => "unexpected zlib return code",
        Status::StreamEnd => "unexpected end of LZ stream",
        Status::NeedDict => "missing LZ dictionary",
        Status::Errno => "zlib IO error",
        Status::StreamError => "bad parameters to zlib",
        Status::DataError => "damaged LZ stream",
        Status::MemError => "insufficient memory",
        Status::BufError => "truncated",
        Status::VersionError => "unsupported zlib version",
    }
    .to_string()
}

/// check_chunk_name (pngrutil.c, from libpng 1.7)
fn check_chunk_name(mut name: u32) -> bool {
    let u = |a: u32, b: u32, c: u32, d: u32| (a << 24) | (b << 16) | (c << 8) | d;
    name &= !u(32, 32, 0, 32);
    let mut t = (name & !0x1f1f1f1fu32) ^ 0x40404040u32;
    name = name.wrapping_sub(u(65, 65, 65, 65));
    t |= name;
    name = name.wrapping_sub(u(25, 25, 25, 26));
    t |= !name;
    (t & 0xe0e0e0e0u32) == 0
}

fn icc_check_header(profile_length: u32, profile: &[u8], color_type: u8) -> bool {
    if get_uint_32(profile) != profile_length {
        return false;
    }
    if profile[8] > 3 && profile_length & 3 != 0 {
        return false;
    }
    let temp = get_uint_32(&profile[128..]);
    if temp > 357913930 || (profile_length as u64) < 132 + 12 * temp as u64 {
        return false;
    }
    let temp = get_uint_32(&profile[64..]);
    if temp >= 0xffff {
        return false;
    }
    let _ = PNG_sRGB_INTENT_LAST; // >= LAST is only a warning
    if get_uint_32(&profile[36..]) != 0x61637370 {
        return false;
    }
    match get_uint_32(&profile[16..]) {
        0x52474220 => {
            if color_type & PNG_COLOR_MASK_COLOR == 0 {
                return false;
            }
        }
        0x47524159 => {
            if color_type & PNG_COLOR_MASK_COLOR != 0 {
                return false;
            }
        }
        _ => return false,
    }
    match get_uint_32(&profile[12..]) {
        0x61627374 | 0x6c696e6b => return false,
        _ => {}
    }
    matches!(get_uint_32(&profile[20..]), 0x58595a20 | 0x4c616220)
}

fn icc_check_tag_table(profile_length: u32, profile: &[u8]) -> bool {
    let tag_count = get_uint_32(&profile[128..]) as usize;
    for k in 0..tag_count {
        let tag = &profile[132 + 12 * k..];
        let start = get_uint_32(&tag[4..]);
        let length = get_uint_32(&tag[8..]);
        if start > profile_length || length > profile_length - start {
            return false;
        }
    }
    true
}

// ------------------------------------------------------------------------- gamma (png.c)
fn translate_gamma_flags(g: i32, is_screen: bool) -> i32 {
    if g == PNG_DEFAULT_sRGB || g == PNG_FP_1 / PNG_DEFAULT_sRGB {
        if is_screen {
            PNG_GAMMA_sRGB
        } else {
            PNG_GAMMA_sRGB_INVERSE
        }
    } else if g == PNG_GAMMA_MAC_18 || g == PNG_FP_1 / PNG_GAMMA_MAC_18 {
        if is_screen {
            PNG_GAMMA_MAC_OLD
        } else {
            PNG_GAMMA_MAC_INVERSE
        }
    } else {
        g
    }
}

fn convert_gamma_value(mut g: f64) -> PngResult<i32> {
    if g > 0.0 && g < 128.0 {
        g *= PNG_FP_1 as f64;
    }
    g = (g + 0.5).floor();
    if g > PNG_FP_MAX || g < PNG_FP_MIN {
        return Err("gamma value: fixed point overflow".into());
    }
    Ok(g as i32)
}

fn unsupported_gamma(g: i32) -> bool {
    g < PNG_LIB_GAMMA_MIN || g > PNG_LIB_GAMMA_MAX
}

fn png_muldiv(a: i32, times: i32, divisor: i32) -> Option<i32> {
    if divisor == 0 {
        return None;
    }
    if a == 0 || times == 0 {
        return Some(0);
    }
    let mut r = a as f64;
    r *= times as f64;
    r /= divisor as f64;
    r = (r + 0.5).floor();
    if r <= 2147483647.0 && r >= -2147483648.0 {
        Some(r as i32)
    } else {
        None
    }
}

pub fn png_reciprocal(a: i32) -> i32 {
    let r = (1E10 / a as f64 + 0.5).floor();
    if r <= 2147483647.0 && r >= -2147483648.0 {
        return r as i32;
    }
    0
}

fn png_reciprocal2(a: i32, b: i32) -> i32 {
    if a != 0 && b != 0 {
        let mut r = 1E15 / a as f64;
        r /= b as f64;
        r = (r + 0.5).floor();
        if r <= 2147483647.0 && r >= -2147483648.0 {
            return r as i32;
        }
    }
    0
}

fn png_gamma_significant(g: i32) -> bool {
    g < PNG_FP_1 - PNG_GAMMA_THRESHOLD_FIXED || g > PNG_FP_1 + PNG_GAMMA_THRESHOLD_FIXED
}

fn png_gamma_threshold(screen_gamma: i32, file_gamma: i32) -> bool {
    match png_muldiv(screen_gamma, file_gamma, PNG_FP_1) {
        None => true,
        Some(g) => png_gamma_significant(g),
    }
}

fn png_gamma_8bit_correct(value: u32, gamma_val: i32) -> u8 {
    if value > 0 && value < 255 {
        let r = (255.0 * (value as i32 as f64 / 255.0).powf(gamma_val as f64 * 0.00001) + 0.5).floor();
        return r as u8;
    }
    (value & 0xff) as u8
}

fn png_gamma_16bit_correct(value: u32, gamma_val: i32) -> u16 {
    if value > 0 && value < 65535 {
        let r = (65535.0 * (value as i32 as f64 / 65535.0).powf(gamma_val as f64 * 0.00001) + 0.5).floor();
        return r as u16;
    }
    value as u16
}

fn build_8bit_table(gamma_val: i32) -> Vec<u8> {
    if png_gamma_significant(gamma_val) {
        (0..256u32).map(|i| png_gamma_8bit_correct(i, gamma_val)).collect()
    } else {
        (0..256u32).map(|i| i as u8).collect()
    }
}

fn build_16bit_table(shift: u32, gamma_val: i32) -> Vec<Vec<u16>> {
    let num = 1u32 << (8 - shift);
    let fmax = 1.0 / (((1i32 << (16 - shift)) - 1) as f64);
    let max = (1u32 << (16 - shift)) - 1;
    let max_by_2 = 1u32 << (15 - shift);
    let mut table = Vec::with_capacity(num as usize);
    for i in 0..num {
        let mut sub = vec![0u16; 256];
        if png_gamma_significant(gamma_val) {
            for j in 0..256u32 {
                let ig = (j << (8 - shift)) + i;
                let d = (65535.0 * (ig as f64 * fmax).powf(gamma_val as f64 * 0.00001) + 0.5).floor();
                sub[j as usize] = d as u16;
            }
        } else {
            for j in 0..256u32 {
                let mut ig = (j << (8 - shift)) + i;
                if shift != 0 {
                    ig = (ig * 65535 + max_by_2) / max;
                }
                sub[j as usize] = ig as u16;
            }
        }
        table.push(sub);
    }
    table
}

fn build_16to8_table(shift: u32, gamma_val: i32) -> Vec<Vec<u16>> {
    let num = 1u32 << (8 - shift);
    let max = (1u32 << (16 - shift)) - 1;
    let mut table = vec![vec![0u16; 256]; num as usize];
    let mut last: u32 = 0;
    for i in 0..255u32 {
        let out = (i * 257) as u16;
        let mut bound = png_gamma_16bit_correct(out as u32 + 128, gamma_val) as u32;
        bound = (bound * max + 32768) / 65535 + 1;
        while last < bound {
            table[(last & (0xff >> shift)) as usize][(last >> (8 - shift)) as usize] = out;
            last += 1;
        }
    }
    while last < (num << 8) {
        table[(last & (0xff >> shift)) as usize][(last >> (8 - shift)) as usize] = 65535;
        last += 1;
    }
    table
}

// ------------------------------------------------------------------- row transforms
fn read_filter_row(ri: &RowInfo, row: &mut [u8], prev: &[u8], filter: u8) {
    let bpp = (ri.pixel_depth as usize + 7) >> 3;
    let n = ri.rowbytes;
    match filter {
        1 => {
            for i in bpp..n {
                row[i] = row[i].wrapping_add(row[i - bpp]);
            }
        }
        2 => {
            for i in 0..n {
                row[i] = row[i].wrapping_add(prev[i]);
            }
        }
        3 => {
            for i in 0..bpp.min(n) {
                row[i] = row[i].wrapping_add(prev[i] / 2);
            }
            for i in bpp..n {
                row[i] = row[i].wrapping_add(((prev[i] as u32 + row[i - bpp] as u32) / 2) as u8);
            }
        }
        4 => {
            for i in 0..bpp.min(n) {
                row[i] = row[i].wrapping_add(prev[i]);
            }
            for i in bpp..n {
                let a = row[i - bpp] as i32;
                let b = prev[i] as i32;
                let c = prev[i - bpp] as i32;
                let p = b - c;
                let pc0 = a - c;
                let mut pa = p.abs();
                let pb = pc0.abs();
                let pc = (p + pc0).abs();
                let mut pred = a;
                if pb < pa {
                    pa = pb;
                    pred = b;
                }
                if pc < pa {
                    pred = c;
                }
                row[i] = row[i].wrapping_add(pred as u8);
            }
        }
        _ => {}
    }
}

fn do_expand_palette(ri: &mut RowInfo, row: &mut [u8], palette: &[PngColor], trans_alpha: &[u8; 256], num_trans: usize) {
    if ri.color_type != PNG_COLOR_TYPE_PALETTE {
        return;
    }
    let w = ri.width as usize;
    if ri.bit_depth < 8 {
        let bd = ri.bit_depth as usize;
        // unpack from the end so the expansion can be done in place
        let mut idx = vec![0u8; w];
        let mask = ((1u32 << bd) - 1) as u8;
        for (x, v) in idx.iter_mut().enumerate() {
            let bit = x * bd;
            *v = (row[bit / 8] >> (8 - bd - bit % 8)) & mask;
        }
        row[..w].copy_from_slice(&idx);
        ri.bit_depth = 8;
        ri.pixel_depth = 8;
        ri.rowbytes = w;
    }
    if ri.bit_depth == 8 {
        let idx: Vec<u8> = row[..w].to_vec();
        if num_trans > 0 {
            for (x, &p) in idx.iter().enumerate() {
                let c = palette[p as usize];
                row[4 * x] = c.red;
                row[4 * x + 1] = c.green;
                row[4 * x + 2] = c.blue;
                row[4 * x + 3] = if (p as usize) >= num_trans { 0xff } else { trans_alpha[p as usize] };
            }
            ri.bit_depth = 8;
            ri.pixel_depth = 32;
            ri.rowbytes = w * 4;
            ri.color_type = 6;
            ri.channels = 4;
        } else {
            for (x, &p) in idx.iter().enumerate() {
                let c = palette[p as usize];
                row[3 * x] = c.red;
                row[3 * x + 1] = c.green;
                row[3 * x + 2] = c.blue;
            }
            ri.bit_depth = 8;
            ri.pixel_depth = 24;
            ri.rowbytes = w * 3;
            ri.color_type = 2;
            ri.channels = 3;
        }
    }
}

fn do_expand(ri: &mut RowInfo, row: &mut [u8], trans_color: Option<&PngColor16>) {
    let w = ri.width as usize;
    if ri.color_type == PNG_COLOR_TYPE_GRAY {
        let mut gray: u32 = trans_color.map(|t| t.gray as u32).unwrap_or(0);
        if ri.bit_depth < 8 {
            let bd = ri.bit_depth as usize;
            let (mul, mask) = match bd {
                1 => (0xffu32, 0x01u32),
                2 => (0x55, 0x03),
                _ => (0x11, 0x0f),
            };
            gray = (gray & mask) * mul;
            let mut px = vec![0u8; w];
            for (x, v) in px.iter_mut().enumerate() {
                let bit = x * bd;
                let s = ((row[bit / 8] >> (8 - bd - bit % 8)) as u32) & mask;
                *v = (s * mul) as u8;
            }
            row[..w].copy_from_slice(&px);
            ri.bit_depth = 8;
            ri.pixel_depth = 8;
            ri.rowbytes = w;
        }
        if trans_color.is_some() {
            if ri.bit_depth == 8 {
                let g = gray & 0xff;
                for x in (0..w).rev() {
                    let v = row[x];
                    row[2 * x] = v;
                    row[2 * x + 1] = if v as u32 == g { 0 } else { 0xff };
                }
            } else if ri.bit_depth == 16 {
                let gh = (gray >> 8) & 0xff;
                let gl = gray & 0xff;
                for x in (0..w).rev() {
                    let (h, l) = (row[2 * x], row[2 * x + 1]);
                    row[4 * x] = h;
                    row[4 * x + 1] = l;
                    let a = if h as u32 == gh && l as u32 == gl { 0 } else { 0xff };
                    row[4 * x + 2] = a;
                    row[4 * x + 3] = a;
                }
            }
            ri.color_type = PNG_COLOR_TYPE_GRAY_ALPHA;
            ri.channels = 2;
            ri.pixel_depth = ri.bit_depth << 1;
            ri.rowbytes = rowbytes(ri.pixel_depth as u32, ri.width);
        }
    } else if ri.color_type == PNG_COLOR_TYPE_RGB {
        if let Some(t) = trans_color {
            if ri.bit_depth == 8 {
                let (r, g, b) = ((t.red & 0xff) as u8, (t.green & 0xff) as u8, (t.blue & 0xff) as u8);
                for x in (0..w).rev() {
                    let (pr, pg, pb) = (row[3 * x], row[3 * x + 1], row[3 * x + 2]);
                    row[4 * x] = pr;
                    row[4 * x + 1] = pg;
                    row[4 * x + 2] = pb;
                    row[4 * x + 3] = if pr == r && pg == g && pb == b { 0 } else { 0xff };
                }
            } else if ri.bit_depth == 16 {
                let tv = [
                    (t.red >> 8) as u8,
                    t.red as u8,
                    (t.green >> 8) as u8,
                    t.green as u8,
                    (t.blue >> 8) as u8,
                    t.blue as u8,
                ];
                for x in (0..w).rev() {
                    let mut px = [0u8; 6];
                    px.copy_from_slice(&row[6 * x..6 * x + 6]);
                    row[8 * x..8 * x + 6].copy_from_slice(&px);
                    let a = if px == tv { 0 } else { 0xff };
                    row[8 * x + 6] = a;
                    row[8 * x + 7] = a;
                }
            }
            ri.color_type = PNG_COLOR_TYPE_RGB_ALPHA;
            ri.channels = 4;
            ri.pixel_depth = ri.bit_depth << 2;
            ri.rowbytes = rowbytes(ri.pixel_depth as u32, ri.width);
        }
    }
}

/// png_do_strip_channel(row_info, row, at_start = 0): drop the last channel.
fn do_strip_channel(ri: &mut RowInfo, row: &mut [u8], _at_start: bool) {
    let ep = ri.rowbytes;
    if ri.channels == 2 {
        if ri.bit_depth == 8 {
            let (mut sp, mut dp) = (2usize, 1usize);
            while sp < ep {
                row[dp] = row[sp];
                dp += 1;
                sp += 2;
            }
            ri.pixel_depth = 8;
        } else if ri.bit_depth == 16 {
            let (mut sp, mut dp) = (4usize, 2usize);
            while sp < ep {
                row[dp] = row[sp];
                row[dp + 1] = row[sp + 1];
                dp += 2;
                sp += 4;
            }
            ri.pixel_depth = 16;
        } else {
            return;
        }
        ri.channels = 1;
        if ri.color_type == PNG_COLOR_TYPE_GRAY_ALPHA {
            ri.color_type = PNG_COLOR_TYPE_GRAY;
        }
    } else if ri.channels == 4 {
        if ri.bit_depth == 8 {
            let (mut sp, mut dp) = (4usize, 3usize);
            while sp < ep {
                row[dp] = row[sp];
                row[dp + 1] = row[sp + 1];
                row[dp + 2] = row[sp + 2];
                dp += 3;
                sp += 4;
            }
            ri.pixel_depth = 24;
        } else if ri.bit_depth == 16 {
            let (mut sp, mut dp) = (8usize, 6usize);
            while sp < ep {
                row.copy_within(sp..sp + 6, dp);
                dp += 6;
                sp += 8;
            }
            ri.pixel_depth = 48;
        } else {
            return;
        }
        ri.channels = 3;
        if ri.color_type == PNG_COLOR_TYPE_RGB_ALPHA {
            ri.color_type = PNG_COLOR_TYPE_RGB;
        }
    } else {
        return;
    }
    ri.rowbytes = rowbytes(ri.pixel_depth as u32, ri.width);
}

fn do_gamma(ri: &RowInfo, row: &mut [u8], t8: Option<&[u8]>, t16: Option<&[Vec<u16>]>, shift: u32) {
    let w = ri.width as usize;
    if !((ri.bit_depth <= 8 && t8.is_some()) || (ri.bit_depth == 16 && t16.is_some())) {
        return;
    }
    let g16 = |row: &mut [u8], k: usize| {
        let t = t16.unwrap();
        let v = t[(row[k + 1] as u32 >> shift) as usize][row[k] as usize];
        row[k] = (v >> 8) as u8;
        row[k + 1] = v as u8;
    };
    match ri.color_type {
        PNG_COLOR_TYPE_RGB | PNG_COLOR_TYPE_RGB_ALPHA | PNG_COLOR_TYPE_GRAY_ALPHA => {
            let (comps, step) = match ri.color_type {
                PNG_COLOR_TYPE_RGB => (3, 3),
                PNG_COLOR_TYPE_RGB_ALPHA => (3, 4),
                _ => (1, 2),
            };
            if ri.bit_depth == 8 {
                let t = t8.unwrap();
                for x in 0..w {
                    for c in 0..comps {
                        let k = x * step + c;
                        row[k] = t[row[k] as usize];
                    }
                }
            } else {
                for x in 0..w {
                    for c in 0..comps {
                        g16(row, 2 * (x * step + c));
                    }
                }
            }
        }
        PNG_COLOR_TYPE_GRAY => {
            if ri.bit_depth == 2 {
                let t = t8.unwrap();
                let mut i = 0;
                let mut sp = 0;
                while i < w {
                    let v = row[sp] as usize;
                    let a = v & 0xc0;
                    let b = v & 0x30;
                    let c = v & 0x0c;
                    let d = v & 0x03;
                    row[sp] = ((t[a | (a >> 2) | (a >> 4) | (a >> 6)] as usize & 0xc0)
                        | ((t[(b << 2) | b | (b >> 2) | (b >> 4)] as usize >> 2) & 0x30)
                        | ((t[(c << 4) | (c << 2) | c | (c >> 2)] as usize >> 4) & 0x0c)
                        | (t[(d << 6) | (d << 4) | (d << 2) | d] as usize >> 6)) as u8;
                    sp += 1;
                    i += 4;
                }
            }
            if ri.bit_depth == 4 {
                let t = t8.unwrap();
                let mut i = 0;
                let mut sp = 0;
                while i < w {
                    let msb = row[sp] as usize & 0xf0;
                    let lsb = row[sp] as usize & 0x0f;
                    row[sp] = ((t[msb | (msb >> 4)] as usize & 0xf0) | (t[(lsb << 4) | lsb] as usize >> 4)) as u8;
                    sp += 1;
                    i += 2;
                }
            } else if ri.bit_depth == 8 {
                let t = t8.unwrap();
                for x in 0..w {
                    row[x] = t[row[x] as usize];
                }
            } else if ri.bit_depth == 16 {
                for x in 0..w {
                    g16(row, 2 * x);
                }
            }
        }
        _ => {}
    }
}

fn do_chop(ri: &mut RowInfo, row: &mut [u8]) {
    if ri.bit_depth == 16 {
        let n = ri.rowbytes;
        let (mut sp, mut dp) = (0usize, 0usize);
        while sp < n {
            row[dp] = row[sp];
            dp += 1;
            sp += 2;
        }
        ri.bit_depth = 8;
        ri.pixel_depth = 8 * ri.channels;
        ri.rowbytes = ri.width as usize * ri.channels as usize;
    }
}
