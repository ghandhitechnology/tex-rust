//! tex-glyph.c (bitmap font lookup with dpi tolerance and fallbacks),
//! magstep.c, and proginit.c (`kpathsea_init_prog`).

use crate::util::*;
use crate::{FileFormat, Kpse, SrcLevel};

/// `kpse_glyph_source_type`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GlyphSource {
    /// The searched-for font already existed.
    Normal,
    /// The name was an alias for an existing file.
    Alias,
    /// Created on the fly by mktexpk.
    Maketex,
    /// Found at a fallback resolution.
    FallbackRes,
    /// Not found; the fallback font was.
    Fallback,
}

/// `kpse_glyph_file_type`.
#[derive(Clone, Debug)]
pub struct GlyphFileInfo {
    /// Font name found.
    pub name: Vec<u8>,
    /// Resolution found.
    pub dpi: u32,
    /// `Pk` or `Gf`.
    pub format: FileFormat,
    /// Where the font came from.
    pub source: GlyphSource,
}

const UNIX_BITMAP_SPEC: &[u8] = b"$KPATHSEA_NAME.$KPATHSEA_DPI$KPATHSEA_FORMAT";
const DPI_BITMAP_SPEC: &[u8] = b"dpi$KPATHSEA_DPI/$KPATHSEA_NAME.$KPATHSEA_FORMAT";
const DOS_BITMAP_SPEC: &[u8] = b"$KPATHSEA_DPI/$KPATHSEA_NAME.$KPATHSEA_FORMAT";

/// `KPSE_BITMAP_TOLERANCE (r)` as an unsigned.
fn tolerance(dpi: f64) -> u32 {
    (dpi / 500.0 + 1.0) as u32
}

/// magstep.c `magstep`.
fn magstep(n: i32, bdpi: i32) -> i32 {
    let mut n = n;
    let mut neg = false;
    if n < 0 {
        neg = true;
        n = -n;
    }
    let mut t: f64;
    if n & 1 != 0 {
        n &= !1;
        t = 1.095445115;
    } else {
        t = 1.0;
    }
    while n > 8 {
        n -= 8;
        t *= 2.0736;
    }
    while n > 0 {
        n -= 2;
        t *= 1.2;
    }
    (0.5 + if neg { bdpi as f64 / t } else { bdpi as f64 * t }) as i32
}

/// magstep.c `kpathsea_magstep_fix`: (true dpi, magstep or 0).
pub(crate) fn magstep_fix(dpi: u32, bdpi: u32) -> (u32, i32) {
    let sign: i32 = if dpi < bdpi { -1 } else { 1 };
    let mut real_dpi: u32 = 0;
    let mut mdpi: i32 = -1;
    let mut m = 0i32;
    while real_dpi == 0 && m < 40 {
        mdpi = magstep(m * sign, bdpi as i32);
        if (mdpi - dpi as i32).abs() <= 1 {
            real_dpi = mdpi as u32;
        } else if (mdpi - dpi as i32) * sign > 0 {
            real_dpi = dpi;
        }
        m += 1;
    }
    let mret = if real_dpi == (if mdpi != 0 { ((m - 1) * sign) as u32 } else { 0 }) { 1 } else { 0 };
    // NB: this reproduces the C expression
    //   *m_ret = real_dpi == (unsigned)(mdpi ? (m - 1) * sign : 0);
    // which compares a dpi with a magstep number.
    let m_ret = mret;
    (if real_dpi != 0 { real_dpi } else { dpi }, m_ret)
}

/// tex-glyph.c `kpathsea_bitmap_tolerance`.
pub(crate) fn bitmap_tolerance(dpi1: f64, dpi2: f64) -> bool {
    let tol = tolerance(dpi2);
    let lower = if (dpi2 - tol as f64) < 0.0 { 0.0 } else { (dpi2 - tol as f64) as u32 as f64 };
    let upper = (dpi2 + tol as f64) as u32 as f64;
    lower <= dpi1 && dpi1 <= upper
}

impl Kpse {
    /// proginit.c `kpathsea_init_prog`.
    pub fn init_prog(&mut self, prefix: &str, dpi: u32, mode: Option<&str>, fallback: Option<&str>) {
        let prefix = prefix.as_bytes();
        let font_var = cat(&[prefix, b"FONTS"]);
        let header_var = cat(&[prefix, b"HEADERS"]);
        let makepk_var = cat(&[prefix, b"MAKEPK"]);
        let size_var = cat(&[prefix, b"SIZES"]);
        if let Some(mk) = self.env.get(&makepk_var) {
            self.set_program_enabled(FileFormat::Pk, true, SrcLevel::Env);
            self.set_program_enabled(FileFormat::AnyGlyph, true, SrcLevel::Env);
            self.format_info[FileFormat::Pk as usize].program = Some(mk.clone());
            self.format_info[FileFormat::AnyGlyph as usize].program = Some(mk);
        }
        let fv = self.env.get(&font_var);
        for f in [FileFormat::Pk, FileFormat::Gf, FileFormat::AnyGlyph, FileFormat::Tfm] {
            self.format_info[f as usize].override_path = fv.clone();
        }
        let hv = self.env.get(&header_var);
        self.format_info[FileFormat::TexPsHeader as usize].override_path = hv;
        self.init_fallback_resolutions(&size_var);
        self.xputenv_int(b"MAKETEX_BASE_DPI", dpi as i32);
        self.fallback_font = fallback.map(|f| f.as_bytes().to_vec());
        let m: &[u8] = match mode {
            Some(m) => m.as_bytes(),
            None => b"/",
        };
        self.xputenv(b"MAKETEX_MODE", m);
    }

    /// tex-glyph.c `try_format`.
    fn try_format(&mut self, format: FileFormat) -> Option<Vec<u8>> {
        let path = match self.format_info[format as usize].path.clone() {
            Some(p) => p,
            None => self.init_format(format),
        };
        if let Some(sfx) = self.format_info[format as usize].suffix.as_ref().and_then(|s| s.first()).cloned() {
            self.xputenv(b"KPATHSEA_FORMAT", &sfx);
        }
        let specs: [&[u8]; 3] = [UNIX_BITMAP_SPEC, DPI_BITMAP_SPEC, DOS_BITMAP_SPEC];
        for must_exist in [false, true] {
            for spec in specs {
                let name = self.var_expand(spec);
                if let Some(r) = self.path_search_first(&path, &name, must_exist) {
                    return Some(r);
                }
            }
        }
        None
    }

    /// tex-glyph.c `try_size`.
    fn try_size(
        &mut self,
        fontname: &[u8],
        dpi: u32,
        format: FileFormat,
        glyph_file: &mut Option<GlyphFileInfo>,
    ) -> Option<Vec<u8>> {
        let try_gf = format == FileFormat::Gf || format == FileFormat::AnyGlyph;
        let try_pk = format == FileFormat::Pk || format == FileFormat::AnyGlyph;
        self.xputenv_int(b"KPATHSEA_DPI", dpi as i32);
        let mut ret = if try_pk { self.try_format(FileFormat::Pk) } else { None };
        let mut format_found = FileFormat::Pk;
        if ret.is_none() && try_gf {
            ret = self.try_format(FileFormat::Gf);
            format_found = FileFormat::Gf;
        }
        if ret.is_some() {
            *glyph_file = Some(GlyphFileInfo {
                name: fontname.to_vec(),
                dpi,
                format: format_found,
                source: GlyphSource::Normal,
            });
        }
        ret
    }

    /// tex-glyph.c `try_resolution`.
    fn try_resolution(
        &mut self,
        fontname: &[u8],
        dpi: u32,
        format: FileFormat,
        glyph_file: &mut Option<GlyphFileInfo>,
    ) -> Option<Vec<u8>> {
        let mut ret = self.try_size(fontname, dpi, format, glyph_file);
        if ret.is_none() {
            let tol = tolerance(dpi as f64);
            let lower = if (dpi.wrapping_sub(tol) as i32) < 0 { 0 } else { dpi.wrapping_sub(tol) };
            let upper = dpi.wrapping_add(tol);
            let mut r = lower;
            while ret.is_none() && r <= upper {
                if r != dpi {
                    ret = self.try_size(fontname, r, format, glyph_file);
                }
                if r == u32::MAX {
                    break;
                }
                r += 1;
            }
        }
        ret
    }

    /// tex-glyph.c `try_fontmap`.
    fn try_fontmap(
        &mut self,
        fontname: &mut Vec<u8>,
        dpi: u32,
        format: FileFormat,
        glyph_file: &mut Option<GlyphFileInfo>,
    ) -> Option<Vec<u8>> {
        let mapped = self.fontmap_lookup(fontname)?;
        let first_name = mapped[0].clone();
        let mut ret = None;
        let mut used: Option<Vec<u8>> = None;
        for m in mapped {
            self.xputenv(b"KPATHSEA_NAME", &m);
            ret = self.try_resolution(&m, dpi, format, glyph_file);
            if ret.is_some() {
                used = Some(m);
                break;
            }
        }
        if ret.is_some() {
            *fontname = used.unwrap();
        } else if self.fontmap_lookup(&first_name).is_none() {
            *fontname = first_name;
        }
        ret
    }

    /// tex-glyph.c `try_fallback_resolutions`.
    fn try_fallback_resolutions(
        &mut self,
        fontname: &[u8],
        dpi: u32,
        format: FileFormat,
        glyph_file: &mut Option<GlyphFileInfo>,
    ) -> Option<Vec<u8>> {
        let fr = self.fallback_resolutions.clone().unwrap_or_default();
        let mut closest_diff = u32::MAX;
        let mut loc: i64 = 0;
        for (s, &r) in fr.iter().enumerate() {
            let this_diff = if r > dpi { r - dpi } else { dpi - r };
            if this_diff < closest_diff {
                closest_diff = this_diff;
                loc = s as i64;
            }
        }
        if fr.is_empty() {
            return None;
        }
        let max_loc = fr.len() as i64;
        let mut lower_loc = loc - 1;
        let mut upper_loc = loc + 1;
        let int_max = i32::MAX as u32;
        loop {
            let fallback = fr[loc as usize];
            if fallback != dpi {
                if let Some(r) = self.try_resolution(fontname, fallback, format, glyph_file) {
                    return Some(r);
                }
            }
            let lower_diff =
                if lower_loc > -1 { dpi.wrapping_sub(fr[lower_loc as usize]) } else { int_max };
            let upper_diff =
                if upper_loc < max_loc { fr[upper_loc as usize].wrapping_sub(dpi) } else { int_max };
            if lower_diff == int_max && upper_diff == int_max {
                break;
            }
            if lower_diff < upper_diff {
                loc = lower_loc;
                lower_loc -= 1;
            } else {
                loc = upper_loc;
                upper_loc += 1;
            }
        }
        None
    }

    /// tex-glyph.c `kpathsea_find_glyph`: look for bitmap font FONT at
    /// resolution DPI in FORMAT (`Pk`, `Gf` or `AnyGlyph`).
    pub fn find_glyph(&mut self, font: impl AsRef<[u8]>, dpi: u32, format: FileFormat) -> Option<(Vec<u8>, GlyphFileInfo)> {
        let mut fontname = font.as_ref().to_vec();
        let mut gf: Option<GlyphFileInfo> = None;
        let mut source = GlyphSource::Normal;
        self.xputenv(b"KPATHSEA_NAME", &fontname);
        let mut ret = self.try_resolution(&fontname.clone(), dpi, format, &mut gf);
        if ret.is_none() {
            source = GlyphSource::Alias;
            ret = self.try_fontmap(&mut fontname, dpi, format, &mut gf);
            if ret.is_none() && !absolute_p(&fontname, true) {
                source = GlyphSource::Maketex;
                self.xputenv_int(b"KPATHSEA_DPI", dpi as i32);
                ret = self.make_tex(format, &fontname);
            }
            if ret.is_some() {
                let g = gf.get_or_insert(GlyphFileInfo {
                    name: Vec::new(),
                    dpi,
                    format,
                    source,
                });
                g.dpi = dpi;
                g.name = fontname.clone();
            } else {
                if self.fallback_resolutions.is_some() {
                    source = GlyphSource::FallbackRes;
                    ret = self.try_fallback_resolutions(&fontname, dpi, format, &mut gf);
                }
                if ret.is_none() {
                    if let Some(name) = self.fallback_font.clone() {
                        source = GlyphSource::Fallback;
                        self.xputenv(b"KPATHSEA_NAME", &name);
                        ret = self.try_resolution(&name, dpi, format, &mut gf);
                        if ret.is_none() && self.fallback_resolutions.is_some() {
                            ret = self.try_fallback_resolutions(&name, dpi, format, &mut gf);
                        }
                    }
                }
            }
        }
        let r = ret?;
        let mut g = gf.unwrap_or(GlyphFileInfo { name: fontname, dpi, format, source });
        g.source = source;
        Some((r, g))
    }
}
