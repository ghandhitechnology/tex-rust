//! Port of pdftexdir/writettf.c (+ writettf.h): TrueType embedding/subsetting and
//! OpenType (CFF) embedding. C `long` is 32 bits here (TeX Live's Windows build).

use std::collections::BTreeMap;

use super::macnames::{AMBIGUOUS_NAMES, MAC_GLYPH_NAMES, NMACGLYPHS};
use super::*;
use crate::gen::globals::Globals;
use crate::rt::kpse;

const DEFAULT_NTABS: u16 = 14;
const NEW_CMAP_SIZE: usize = 2;
const TABDIR_OFF: i32 = 12;

const ARG_1_AND_2_ARE_WORDS: u16 = 1 << 0;
const WE_HAVE_A_SCALE: u16 = 1 << 3;
const MORE_COMPONENTS: u16 = 1 << 5;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 1 << 6;
const WE_HAVE_A_TWO_BY_TWO: u16 = 1 << 7;
const WE_HAVE_INSTRUCTIONS: u16 = 1 << 8;

const BYTE_ENCODING_LENGTH: u32 = 256 + 3 * 2;
const TRIMMED_TABLE_MAP_LENGTH: u32 = 2 * (5 + 256);
const SEG_MAP_DELTA_LENGTH: u32 = (16 + 256) * 2;
const CMAP_ENTRY_LENGTH: u32 = 2 * 2 + 4;

const NEWTABNAMES: [&[u8; 4]; 14] =
    [b"OS/2", b"PCLT", b"cmap", b"cvt ", b"fpgm", b"glyf", b"head", b"hhea", b"hmtx", b"loca", b"maxp", b"name", b"post", b"prep"];

#[derive(Clone, Copy, Default)]
struct DirtabEntry {
    tag: [u8; 4],
    checksum: u32,
    offset: u32,
    length: u32,
}

#[derive(Clone, Default)]
struct GlyphEntry {
    offset: i32,
    newoffset: i32,
    adv_width: u16,
    lsb: i16,
    /// None = the notdef pointer
    name: GlyphName,
    newindex: i16,
    name_index: u16,
}

#[derive(Clone, Default)]
struct TtfencEntry {
    name: GlyphName,
    code: i32,
    newindex: i32,
}

#[derive(Clone, Copy, Default)]
struct NameRecord {
    platform_id: u16,
    encoding_id: u16,
    language_id: u16,
    name_id: u16,
    length: u16,
    offset: u16,
    new_offset: u16,
    new_length: u16,
}

#[derive(Clone, Copy, Default)]
struct CmapEntry {
    platform_id: u16,
    encoding_id: u16,
    offset: u32,
    format: u16,
}

/// writettf.c static state
#[derive(Default)]
pub struct TtfState {
    ntabs: u16,
    upem: u16,
    post_format: u32,
    loca_format: i16,
    last_glyf_offset: u32,
    glyphs_count: u16,
    new_glyphs_count: u16,
    nhmtxs: u16,
    new_ntabs: u16,
    glyph_tab: Option<Vec<GlyphEntry>>,
    glyph_index: Vec<i32>,
    new_cmap_tab: [CmapEntry; NEW_CMAP_SIZE],
    name_tab: Vec<NameRecord>,
    name_record_num: i32,
    name_buf: Vec<u8>,
    name_buf_size: i32,
    dir_tab: Vec<DirtabEntry>,
    checksum: u32,
    tab_length: u32,
    tmp_ulong: u32,
    check_sum_adjustment_offset: u32,
    file: CFile,
    ttfenc_tab: Vec<TtfencEntry>,
    fd_cur: usize,
    /// ttf_cmap_tree: (ttf_name, pid, eid) -> table[0x10000]
    cmap_tree: BTreeMap<(Vec<u8>, u16, u16), Vec<i32>>,
    pub ttf_length: i32,
}

fn is_c_isspace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// writettf.c strip_spaces_and_delims
fn strip_spaces_and_delims(s: &[u8], l: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for &c in s.iter().take(l) {
        if matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%') || is_c_isspace(c) {
            continue;
        }
        out.push(c);
    }
    out
}

/// writettf.c ttf_funit
fn ttf_funit(n: i32, upem: u16) -> i32 {
    let u = upem as i32;
    if u == 0 {
        return 0;
    }
    if n < 0 {
        -((-n / u) * 1000 + ((-n % u) * 1000) / u)
    } else {
        (n / u) * 1000 + ((n % u) * 1000) / u
    }
}

impl Globals {
    fn ttf(&mut self) -> &mut TtfState {
        &mut self.rt.fonts.ttf
    }

    /// writettf.c ttf_addchksm
    fn ttf_addchksm(&mut self, b: u8) -> u8 {
        let t = self.ttf();
        t.tmp_ulong = (t.tmp_ulong << 8).wrapping_add(b as u32);
        t.tab_length = t.tab_length.wrapping_add(1);
        if t.tab_length % 4 == 0 {
            t.checksum = t.checksum.wrapping_add(t.tmp_ulong);
            t.tmp_ulong = 0;
        }
        b
    }

    /// writettf.c ttf_getchksm
    fn ttf_getchksm(&mut self) -> u32 {
        while self.ttf().tab_length % 4 != 0 {
            let b = self.ttf_addchksm(0);
            self.fb_putchar(b);
        }
        self.ttf().checksum
    }

    /// writettf.c ttf_putnum
    fn ttf_putnum(&mut self, s: usize, n: i32) -> i32 {
        let mut buf = [0u8; 4];
        let mut i = n;
        for b in buf.iter_mut().take(s) {
            *b = (i & 0xFF) as u8;
            i >>= 8;
        }
        for k in (0..s).rev() {
            let b = self.ttf_addchksm(buf[k]);
            self.fb_putchar(b);
        }
        n
    }

    /// writettf.c ttf_getnum
    fn ttf_getnum(&mut self, mut s: usize) -> i32 {
        let mut i: i32 = 0;
        while s > 0 {
            let c = self.ttf().file.getc();
            if c < 0 {
                self.pdftex_fail("unexpected EOF");
            }
            i = (i << 8).wrapping_add(c);
            s -= 1;
        }
        i
    }

    fn get_byte(&mut self) -> u8 {
        self.ttf_getnum(1) as u8
    }
    fn get_char(&mut self) -> i8 {
        self.ttf_getnum(1) as i8
    }
    fn get_ushort(&mut self) -> u16 {
        self.ttf_getnum(2) as u16
    }
    fn get_short(&mut self) -> i16 {
        self.ttf_getnum(2) as i16
    }
    fn get_ulong(&mut self) -> u32 {
        self.ttf_getnum(4) as u32
    }
    fn get_fword(&mut self) -> i16 {
        self.ttf_getnum(2) as i16
    }
    fn get_ufword(&mut self) -> u16 {
        self.ttf_getnum(2) as u16
    }
    fn ttf_skip(&mut self, n: usize) {
        self.ttf_getnum(n);
    }
    fn put_byte(&mut self, n: i32) {
        self.ttf_putnum(1, n);
    }
    fn put_ushort(&mut self, n: i32) -> u16 {
        self.ttf_putnum(2, n) as u16
    }
    fn put_short(&mut self, n: i32) -> i16 {
        self.ttf_putnum(2, n) as i16
    }
    fn put_ulong(&mut self, n: u32) {
        self.ttf_putnum(4, n as i32);
    }
    fn copy_byte(&mut self) {
        let b = self.get_byte();
        self.put_byte(b as i32);
    }
    fn copy_char(&mut self) {
        let b = self.get_char();
        self.put_byte(b as i32);
    }
    fn copy_short(&mut self) -> i16 {
        let v = self.get_short();
        self.put_short(v as i32)
    }
    fn copy_ushort(&mut self) -> u16 {
        let v = self.get_ushort();
        self.put_ushort(v as i32)
    }

    /// writettf.c ttf_ncopy
    fn ttf_ncopy(&mut self, mut n: i32) {
        while n > 0 {
            self.copy_byte();
            n -= 1;
        }
    }

    /// writettf.c ttf_name_lookup
    fn ttf_name_lookup(&mut self, s: &[u8], required: bool) -> Option<usize> {
        let t = &self.rt.fonts.ttf;
        let r = t.dir_tab.iter().position(|e| &e.tag[..] == &s[..4]);
        if r.is_none() && required {
            self.pdftex_fail(&format!("can't find table `{}'", bs(s)));
        }
        r
    }

    /// writettf.c ttf_seek_tab
    fn ttf_seek_tab(&mut self, name: &[u8], offset: i32) -> usize {
        let tab = self.ttf_name_lookup(name, true).unwrap();
        let o = self.ttf().dir_tab[tab].offset as i32 as i64 + offset as i64;
        self.ttf().file.seek(o);
        tab
    }

    fn ttf_seek_off(&mut self, offset: i64) {
        self.ttf().file.seek(offset);
    }

    /// writettf.c ttf_copy_encoding
    fn ttf_copy_encoding(&mut self) {
        let fd = self.ttf().fd_cur;
        if self.ttf().ttfenc_tab.len() < 256 {
            self.ttf().ttfenc_tab = vec![TtfencEntry::default(); 256];
        }
        if let Some(fe) = self.fd(fd).fe {
            let glyph_names = self.rt.fonts.enc.fe[fe].glyph_names.clone();
            for e in self.ttf().ttfenc_tab.iter_mut() {
                e.name = None;
            }
            if gname(&glyph_names[97]) == b"a" {
                self.fd_mut(fd).tx_tree.as_mut().unwrap().insert(b'a' as i32);
            }
            let tx: Vec<i32> = self.fd(fd).tx_tree.as_ref().unwrap().iter().copied().collect();
            for q in tx {
                self.ttf().ttfenc_tab[q as usize].name = glyph_names[q as usize].clone();
            }
            self.make_subset_tag(fd);
        } else if self.fm(self.fd(fd).fm).is_subfont() {
            let fm = self.fd(fd).fm;
            let sf = self.fm(fm).subfont.unwrap();
            let charcodes = self.rt.fonts.sfd.subfonts[sf].charcodes;
            for e in self.ttf().ttfenc_tab.iter_mut() {
                e.code = -1;
            }
            let tx: Vec<i32> = self.fd(fd).tx_tree.as_ref().unwrap().iter().copied().collect();
            for q in tx {
                let code = charcodes[q as usize] as i32;
                self.ttf().ttfenc_tab[q as usize].code = code;
                if code == -1 {
                    let tfm = bs(self.fm(fm).tfm());
                    self.pdftex_warn(&format!("character {} in subfont {} is not mapped to any charcode", q, tfm));
                } else {
                    let s = format!("/c{:04X}", code);
                    self.fd_mut(fd).gl_tree.as_mut().unwrap().insert(s.into_bytes());
                }
            }
            self.make_subset_tag(fd);
        } else {
            unreachable!();
        }
    }

    /// writettf.c ttf_read_name
    fn ttf_read_name(&mut self) {
        let fd = self.ttf().fd_cur;
        let tab = self.ttf_seek_tab(b"name", 2);
        let n = self.get_ushort() as i32;
        let len = self.ttf().dir_tab[tab].length as i32;
        self.ttf().name_record_num = n;
        self.ttf().name_buf_size = len - (3 * 2 + n * 6 * 2);
        self.ttf_skip(2);
        let mut recs = Vec::with_capacity(n.max(0) as usize);
        for _ in 0..n {
            let r = NameRecord {
                platform_id: self.get_ushort(),
                encoding_id: self.get_ushort(),
                language_id: self.get_ushort(),
                name_id: self.get_ushort(),
                length: self.get_ushort(),
                offset: self.get_ushort(),
                ..Default::default()
            };
            recs.push(r);
        }
        let size = self.ttf().name_buf_size.max(0) as usize;
        let mut nb = Vec::with_capacity(size);
        for _ in 0..size {
            nb.push(self.get_char() as u8);
        }
        self.ttf().name_tab = recs.clone();
        self.ttf().name_buf = nb.clone();
        let slice_at = |off: usize, l: usize| -> Vec<u8> { (0..l).map(|k| nb.get(off + k).copied().unwrap_or(0)).collect() };
        for r in &recs {
            if r.platform_id == 1 && r.encoding_id == 0 && r.name_id == 6 {
                let s = strip_spaces_and_delims(&slice_at(r.offset as usize, r.length as usize), r.length as usize);
                let e = self.fd_mut(fd);
                e.fontname = Some(s);
                e.font_dim[FONTNAME_CODE].set = true;
                break;
            }
        }
        if !self.fd(fd).font_dim[FONTNAME_CODE].set {
            for r in &recs {
                if r.platform_id == 3 && (r.encoding_id == 0 || r.encoding_id == 1) && r.name_id == 6 {
                    let mut buf = Vec::new();
                    let mut j = 0;
                    while j < r.length as usize {
                        buf.push(nb.get(r.offset as usize + j + 1).copied().unwrap_or(0));
                        j += 2;
                    }
                    if let Some(z) = buf.iter().position(|&c| c == 0) {
                        buf.truncate(z);
                    }
                    let l = buf.len();
                    let s = strip_spaces_and_delims(&buf, l);
                    let e = self.fd_mut(fd);
                    e.fontname = Some(s);
                    e.font_dim[FONTNAME_CODE].set = true;
                    break;
                }
            }
        }
    }

    /// writettf.c ttf_read_mapx
    fn ttf_read_mapx(&mut self) {
        self.ttf_seek_tab(b"maxp", 4);
        let n = self.get_ushort();
        let t = self.ttf();
        t.glyphs_count = n;
        let mut g = vec![GlyphEntry::default(); n as usize + 1];
        for e in g.iter_mut().take(n as usize) {
            e.newindex = -1;
            e.newoffset = 0;
            e.name_index = 0;
            e.name = None;
        }
        t.glyph_tab = Some(g);
        t.glyph_index = vec![0; n as usize + 2];
        t.glyph_index[0] = 0;
        t.glyph_index[1] = 1;
    }

    /// writettf.c ttf_read_head
    fn ttf_read_head(&mut self) {
        let fd = self.ttf().fd_cur;
        self.ttf_seek_tab(b"head", 2 * 4 + 2 * 4 + 2);
        let upem = self.get_ushort();
        self.ttf().upem = upem;
        self.ttf_skip(16);
        for k in FONTBBOX1_CODE..=FONTBBOX4_CODE {
            let v = ttf_funit(self.get_fword() as i32, upem);
            let e = &mut self.fd_mut(fd).font_dim[k];
            e.val = v;
            e.set = true;
        }
        self.ttf_skip(2 * 2 + 2);
        let lf = self.get_short();
        self.ttf().loca_format = lf;
    }

    /// writettf.c ttf_read_hhea
    fn ttf_read_hhea(&mut self) {
        let fd = self.ttf().fd_cur;
        self.ttf_seek_tab(b"hhea", 4);
        let upem = self.ttf().upem;
        let a = ttf_funit(self.get_fword() as i32, upem);
        let d = ttf_funit(self.get_fword() as i32, upem);
        let e = self.fd_mut(fd);
        e.font_dim[ASCENT_CODE] = IntParm { val: a, set: true };
        e.font_dim[DESCENT_CODE] = IntParm { val: d, set: true };
        self.ttf_skip(2 + 2 + 3 * 2 + 8 * 2);
        let n = self.get_ushort();
        self.ttf().nhmtxs = n;
    }

    /// writettf.c ttf_read_pclt
    fn ttf_read_pclt(&mut self) {
        if self.ttf_name_lookup(b"PCLT", false).is_none() {
            return;
        }
        let fd = self.ttf().fd_cur;
        self.ttf_seek_tab(b"PCLT", 4 + 4 + 2);
        let upem = self.ttf().upem;
        let x = ttf_funit(self.get_ushort() as i32, upem);
        self.ttf_skip(2 * 2);
        let c = ttf_funit(self.get_ushort() as i32, upem);
        let e = self.fd_mut(fd);
        e.font_dim[XHEIGHT_CODE] = IntParm { val: x, set: true };
        e.font_dim[CAPHEIGHT_CODE] = IntParm { val: c, set: true };
    }

    /// writettf.c ttf_read_hmtx
    fn ttf_read_hmtx(&mut self) {
        self.ttf_seek_tab(b"hmtx", 0);
        let (n, gc) = (self.ttf().nhmtxs as usize, self.ttf().glyphs_count as usize);
        let len = self.ttf().glyph_tab.as_ref().unwrap().len();
        let mut i = 0;
        while i < n {
            let a = self.get_ufword();
            let l = self.get_ufword() as i16;
            if i < len {
                let g = &mut self.ttf().glyph_tab.as_mut().unwrap()[i];
                g.adv_width = a;
                g.lsb = l;
            }
            i += 1;
        }
        if n < gc {
            let last = if i > 0 { self.ttf().glyph_tab.as_ref().unwrap()[(i - 1).min(len - 1)].adv_width } else { 0 };
            while i < gc {
                let l = self.get_ufword() as i16;
                let g = &mut self.ttf().glyph_tab.as_mut().unwrap()[i];
                g.adv_width = last;
                g.lsb = l;
                i += 1;
            }
        }
    }

    /// writettf.c ttf_read_post
    fn ttf_read_post(&mut self) {
        let fd = self.ttf().fd_cur;
        let tab = self.ttf_seek_tab(b"post", 0);
        let pf = self.get_ulong();
        self.ttf().post_format = pf;
        let italic_angle = self.get_ulong();
        let mut int_part = (italic_angle >> 16) as i32;
        let mut sign = 1;
        if int_part > 0x7FFF {
            int_part = 0x10000 - int_part;
            sign = -1;
        }
        let frac_part = (italic_angle % 0x10000) as i32;
        let e = &mut self.fd_mut(fd).font_dim[ITALIC_ANGLE_CODE];
        e.val = d2i(sign as f64 * (int_part as f64 + frac_part as f64 * 1.0 / 0x10000 as f64));
        e.set = true;
        if self.ttf().glyph_tab.is_none() {
            return;
        }
        self.ttf_skip(2 * 2 + 5 * 4);
        let len = self.ttf().glyph_tab.as_ref().unwrap().len();
        match pf {
            0x10000 => {
                let g = self.ttf().glyph_tab.as_mut().unwrap();
                for i in 0..NMACGLYPHS.min(len) {
                    g[i].name = MAC_GLYPH_NAMES[i].map(|s| s.to_vec());
                    g[i].name_index = i as u16;
                }
            }
            0x20000 => {
                let nnames = self.get_ushort() as usize;
                for i in 0..nnames {
                    let v = self.get_ushort();
                    if i < len {
                        self.ttf().glyph_tab.as_mut().unwrap()[i].name_index = v;
                    }
                }
                let d = self.ttf().dir_tab[tab];
                let length = d.length as i32 - (self.ttf().file.tell() as i32 - d.offset as i32);
                let mut buf: Vec<u8> = Vec::new();
                while (buf.len() as i32) < length {
                    let mut k = self.get_byte() as i32;
                    while k > 0 {
                        buf.push(self.get_char() as u8);
                        k -= 1;
                    }
                    buf.push(0);
                }
                for i in 0..nnames.min(len) {
                    let ni = self.ttf().glyph_tab.as_ref().unwrap()[i].name_index as usize;
                    let name = if ni < NMACGLYPHS {
                        MAC_GLYPH_NAMES[ni].map(|s| s.to_vec())
                    } else {
                        let mut p = 0usize;
                        let mut k = ni - NMACGLYPHS;
                        while k > 0 {
                            p += c_strlen(&buf, p) + 1;
                            k -= 1;
                        }
                        Some(c_str(&buf, p).to_vec())
                    };
                    self.ttf().glyph_tab.as_mut().unwrap()[i].name = name;
                }
            }
            _ => {
                if pf != 0x00030000 {
                    self.pdftex_warn(&format!("unsupported format ({:08X}) of `post' table, assuming 3.0", pf));
                }
                let g = self.ttf().glyph_tab.as_mut().unwrap();
                for i in 0..NMACGLYPHS.min(len) {
                    g[i].name_index = i as u16;
                }
            }
        }
    }

    /// writettf.c ttf_read_loca
    fn ttf_read_loca(&mut self) {
        self.ttf_seek_tab(b"loca", 0);
        let n = self.ttf().glyphs_count as usize + 1;
        let lf = self.ttf().loca_format;
        for i in 0..n {
            let o = if lf != 0 { self.get_ulong() as i32 } else { (self.get_ushort() as i32) << 1 };
            self.ttf().glyph_tab.as_mut().unwrap()[i].offset = o;
        }
    }

    /// writettf.c ttf_read_tabdir
    fn ttf_read_tabdir(&mut self) {
        self.ttf_skip(4);
        let n = self.get_ushort();
        self.ttf().ntabs = n;
        self.ttf_skip(3 * 2);
        let mut v = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let mut tag = [0u8; 4];
            for t in tag.iter_mut() {
                *t = self.get_char() as u8;
            }
            let checksum = self.get_ulong();
            let offset = self.get_ulong();
            let length = self.get_ulong();
            v.push(DirtabEntry { tag, checksum, offset, length });
        }
        self.ttf().dir_tab = v;
    }

    /// writettf.c ttf_read_cmap: the (cached) unicode->glyph table, or None.
    fn ttf_read_cmap(&mut self, ttf_name: &[u8], pid: i32, eid: i32, warn: bool) -> Option<(Vec<u8>, u16, u16)> {
        let key = (ttf_name.to_vec(), pid as u16, eid as u16);
        if self.ttf().cmap_tree.contains_key(&key) {
            return Some(key);
        }
        self.ttf_seek_tab(b"cmap", 2);
        let ncmapsubtabs = self.get_ushort();
        let cmap_offset = self.ttf().file.tell() as u32 - 2 * 2;
        let mut found = false;
        for _ in 0..ncmapsubtabs {
            let tmp_pid = self.get_ushort();
            let tmp_eid = self.get_ushort();
            let tmp_offset = self.get_ulong();
            if tmp_pid as i32 == pid && tmp_eid as i32 == eid {
                self.ttf_seek_off(cmap_offset.wrapping_add(tmp_offset) as i64);
                let format = self.get_ushort();
                if format == 4 {
                    found = true;
                    break;
                } else {
                    if warn {
                        self.pdftex_warn(&format!("cmap format {} unsupported", format));
                    }
                    return None;
                }
            }
        }
        if !found {
            if warn {
                self.pdftex_warn(&format!("cannot find cmap subtable for (pid,eid) = ({}, {})", pid, eid));
            }
            return None;
        }
        let mut table = vec![-1i32; 0x10000];
        let mut length = self.get_ushort() as i32;
        self.get_ushort();
        let seg_count = (self.get_ushort() / 2) as usize;
        self.get_ushort();
        self.get_ushort();
        self.get_ushort();
        let mut end_code = vec![0u16; seg_count];
        let mut start_code = vec![0u16; seg_count];
        let mut id_delta = vec![0u16; seg_count];
        let mut id_range_offset = vec![0u16; seg_count];
        for v in end_code.iter_mut() {
            *v = self.get_ushort();
        }
        self.get_ushort();
        for v in start_code.iter_mut() {
            *v = self.get_ushort();
        }
        for v in id_delta.iter_mut() {
            *v = self.get_ushort();
        }
        for v in id_range_offset.iter_mut() {
            *v = self.get_ushort();
        }
        length -= 8 * 2 + 4 * seg_count as i32 * 2;
        let n = length / 2;
        let mut glyph_id = Vec::with_capacity(n.max(0) as usize);
        for _ in 0..n {
            glyph_id.push(self.get_ushort());
        }
        let gc = self.ttf().glyphs_count as i32;
        for s in 0..seg_count {
            let mut i = start_code[s] as i32;
            while i <= end_code[s] as i32 {
                if i == 0xFFFF {
                    break;
                }
                if id_range_offset[s] != 0xFFFF {
                    let index: i32 = if id_range_offset[s] == 0 {
                        (id_delta[s] as i32 + i) & 0xFFFF
                    } else {
                        let k = (i - start_code[s] as i32) + id_range_offset[s] as i32 / 2 + s as i32 - seg_count as i32;
                        let mut idx = glyph_id.get(k.max(0) as usize).copied().unwrap_or(0) as i32;
                        if idx != 0 {
                            idx = (idx + id_delta[s] as i32) & 0xFFFF;
                        }
                        idx
                    };
                    if index >= gc {
                        self.pdftex_fail(&format!("cmap: glyph index {} out of range [0..{})", index, gc));
                    }
                    if table[i as usize] != -1 {
                        self.pdftex_warn(&format!(
                            "cmap: multiple glyphs are mapped to unicode {:04X}, only {} will be used (glyph {} being ignored)",
                            i, table[i as usize], index
                        ));
                    } else {
                        table[i as usize] = index;
                    }
                }
                i += 1;
            }
        }
        self.ttf().cmap_tree.insert(key.clone(), table);
        Some(key)
    }

    /// writettf.c ttf_read_font
    fn ttf_read_font(&mut self) {
        self.ttf_read_tabdir();
        for t in [b"PCLT", b"fpgm", b"cvt ", b"prep"] {
            if self.ttf_name_lookup(t, false).is_none() {
                self.ttf().new_ntabs -= 1;
            }
        }
        self.ttf_read_mapx();
        self.ttf_read_head();
        self.ttf_read_hhea();
        self.ttf_read_pclt();
        self.ttf_read_hmtx();
        self.ttf_read_post();
        self.ttf_read_loca();
        self.ttf_read_name();
    }

    /// writettf.c ttf_reset_chksm
    fn ttf_reset_chksm(&mut self, tab: usize) {
        let off = self.fb_offset() as u32;
        let t = self.ttf();
        t.checksum = 0;
        t.tab_length = 0;
        t.tmp_ulong = 0;
        t.dir_tab[tab].offset = off;
        if off % 4 != 0 {
            let tag = bs(&t.dir_tab[tab].tag);
            self.pdftex_warn(&format!("offset of `{}' is not a multiple of 4", tag));
        }
    }

    /// writettf.c ttf_set_chksm
    fn ttf_set_chksm(&mut self, tab: usize) {
        let off = self.fb_offset() as u32;
        let t = self.ttf();
        t.dir_tab[tab].length = off.wrapping_sub(t.dir_tab[tab].offset);
        let c = self.ttf_getchksm();
        self.ttf().dir_tab[tab].checksum = c;
    }

    /// writettf.c ttf_copytab
    fn ttf_copytab(&mut self, name: &[u8]) {
        let tab = self.ttf_seek_tab(name, 0);
        self.ttf_reset_chksm(tab);
        let mut i = self.ttf().dir_tab[tab].length as i32;
        while i > 0 {
            self.copy_char();
            i -= 1;
        }
        self.ttf_set_chksm(tab);
    }

    /// writettf.c ttf_byte_encoding
    fn ttf_byte_encoding(&mut self) {
        self.put_ushort(0);
        self.put_ushort(BYTE_ENCODING_LENGTH as i32);
        self.put_ushort(0);
        for i in 0..256 {
            let e = self.ttf().ttfenc_tab[i].clone();
            if e.newindex < 256 {
                self.put_byte(e.newindex);
            } else {
                if let Some(n) = &e.name {
                    self.pdftex_warn(&format!("glyph `{}' has been mapped to `.notdef' in `ttf_byte_encoding' cmap table", bs(n)));
                }
                self.put_byte(0);
            }
        }
    }

    /// writettf.c ttf_trimmed_table_map
    fn ttf_trimmed_table_map(&mut self) {
        self.put_ushort(6);
        self.put_ushort(TRIMMED_TABLE_MAP_LENGTH as i32);
        self.put_ushort(0);
        self.put_ushort(0);
        self.put_ushort(256);
        for i in 0..256 {
            let n = self.ttf().ttfenc_tab[i].newindex;
            self.put_ushort(n);
        }
    }

    /// writettf.c ttf_seg_map_delta
    fn ttf_seg_map_delta(&mut self) {
        for v in [4, SEG_MAP_DELTA_LENGTH as i32, 0, 4, 4, 1, 0, 0xF0FF, 0xFFFF, 0, 0xF000, 0xFFFF, 0, 1, 2 * 2, 0] {
            self.put_ushort(v);
        }
        for i in 0..256 {
            let n = self.ttf().ttfenc_tab[i].newindex;
            self.put_ushort(n);
        }
    }

    /// writettf.c ttf_select_cmap
    fn ttf_select_cmap(&mut self) {
        let t = self.ttf();
        t.new_cmap_tab[0] = CmapEntry { platform_id: 1, encoding_id: 0, offset: 0, format: if t.new_glyphs_count < 256 { 0 } else { 6 } };
        t.new_cmap_tab[1] = CmapEntry { platform_id: 3, encoding_id: 0, offset: 0, format: 4 };
    }

    /// writettf.c ttf_write_cmap
    fn ttf_write_cmap(&mut self) {
        let tab = self.ttf_name_lookup(b"cmap", true).unwrap();
        self.ttf_select_cmap();
        self.ttf_reset_chksm(tab);
        self.put_ushort(0);
        self.put_ushort(NEW_CMAP_SIZE as i32);
        let mut offset: u32 = 2 * 2 + NEW_CMAP_SIZE as u32 * CMAP_ENTRY_LENGTH;
        for k in 0..NEW_CMAP_SIZE {
            let mut ce = self.ttf().new_cmap_tab[k];
            ce.offset = offset;
            match ce.format {
                0 => offset += BYTE_ENCODING_LENGTH,
                4 => offset += SEG_MAP_DELTA_LENGTH,
                6 => offset += TRIMMED_TABLE_MAP_LENGTH,
                _ => self.pdftex_fail("invalid format (it should not have happened)"),
            }
            self.ttf().new_cmap_tab[k] = ce;
            self.put_ushort(ce.platform_id as i32);
            self.put_ushort(ce.encoding_id as i32);
            self.put_ulong(ce.offset);
        }
        for k in 0..NEW_CMAP_SIZE {
            match self.ttf().new_cmap_tab[k].format {
                0 => self.ttf_byte_encoding(),
                4 => self.ttf_seg_map_delta(),
                6 => self.ttf_trimmed_table_map(),
                _ => {}
            }
        }
        self.ttf_set_chksm(tab);
    }

    /// writettf.c prepend_subset_tags
    fn prepend_subset_tags(&self, index: usize, p: &mut Vec<u8>) -> u16 {
        let t = &self.rt.fonts.ttf;
        let tag = self.fd(t.fd_cur).subset_tag.clone().unwrap();
        if t.name_tab[index].platform_id == 3 {
            for &c in tag.iter().take(6) {
                p.push(0);
                p.push(c);
            }
            p.push(0);
            p.push(b'+');
            14
        } else {
            p.extend_from_slice(&tag[..6]);
            p.push(b'+');
            7
        }
    }

    /// writettf.c ttf_write_name
    fn ttf_write_name(&mut self) {
        let tab = self.ttf_name_lookup(b"name", true).unwrap();
        let fd = self.ttf().fd_cur;
        let new_name_buf: Vec<u8>;
        if self.fm(self.fd(fd).fm).is_subsetted() {
            let mut p: Vec<u8> = Vec::new();
            let n = self.ttf().name_record_num as usize;
            for i in 0..n {
                let rec = self.ttf().name_tab[i];
                let new_offset = p.len() as u16;
                let l = if (rec.name_id == 1 || rec.name_id == 3 || rec.name_id == 4 || rec.name_id == 6)
                    && ((rec.platform_id == 1 && rec.encoding_id == 0)
                        || (rec.platform_id == 3 && rec.encoding_id == 0)
                        || (rec.platform_id == 3 && rec.encoding_id == 1))
                {
                    self.prepend_subset_tags(i, &mut p)
                } else {
                    0
                };
                let nb = &self.rt.fonts.ttf.name_buf;
                for k in 0..rec.length as usize {
                    p.push(nb.get(rec.offset as usize + k).copied().unwrap_or(0));
                }
                let r = &mut self.ttf().name_tab[i];
                r.new_offset = new_offset;
                r.new_length = rec.length.wrapping_add(l);
            }
            new_name_buf = p;
        } else {
            let t = self.ttf();
            let size = t.name_buf_size.max(0) as usize;
            new_name_buf = t.name_buf[..size.min(t.name_buf.len())].to_vec();
            for r in t.name_tab.iter_mut() {
                // new_length/new_offset are not set in the C code for this case: they keep
                // their (zero-initialized by xtalloc? no: uninitialized) values.
                let _ = r;
            }
        }
        self.ttf_reset_chksm(tab);
        self.put_ushort(0);
        let n = self.ttf().name_record_num;
        self.put_ushort(n);
        self.put_ushort(3 * 2 + n * 6 * 2);
        for i in 0..n as usize {
            let r = self.ttf().name_tab[i];
            self.put_ushort(r.platform_id as i32);
            self.put_ushort(r.encoding_id as i32);
            self.put_ushort(r.language_id as i32);
            self.put_ushort(r.name_id as i32);
            self.put_ushort(r.new_length as i32);
            self.put_ushort(r.new_offset as i32);
        }
        for b in new_name_buf {
            self.put_byte(b as i8 as i32);
        }
        self.ttf_set_chksm(tab);
    }

    /// writettf.c ttf_write_dirtab
    fn ttf_write_dirtab(&mut self) {
        let save_offset = self.fb_offset() as u32;
        self.fb_seek(TABDIR_OFF);
        let fd = self.ttf().fd_cur;
        let entries: Vec<DirtabEntry> = if self.fm(self.fd(fd).fm).is_subsetted() {
            let mut v = Vec::new();
            for name in NEWTABNAMES {
                if let Some(t) = self.ttf_name_lookup(name, false) {
                    v.push(self.ttf().dir_tab[t]);
                }
            }
            v
        } else {
            self.ttf().dir_tab.clone()
        };
        for e in entries {
            for k in 0..4 {
                self.put_byte(e.tag[k] as i8 as i32);
            }
            self.put_ulong(e.checksum);
            self.put_ulong(e.offset);
            self.put_ulong(e.length);
        }
        // adjust checkSumAdjustment; fb_array is `char` (signed): bytes >= 0x80 are added
        // sign-extended.
        let mut tmp: u32 = 0;
        let mut checksum: u32 = 0;
        let mut i: u32 = 0;
        {
            let fb = &self.rt.pdfu.fb;
            while i < save_offset {
                let b = fb.get(i as usize).copied().unwrap_or(0);
                tmp = (tmp << 8).wrapping_add(b as i8 as i32 as u32);
                i += 1;
                if i % 4 == 0 {
                    checksum = checksum.wrapping_add(tmp);
                    tmp = 0;
                }
            }
        }
        if i % 4 != 0 {
            self.pdftex_warn(&format!("font length is not a multiple of 4 ({})", i));
            checksum <<= 8 * (4 - i % 4);
        }
        let t = self.ttf();
        t.tmp_ulong = tmp;
        t.checksum = checksum;
        let k = 0xB1B0AFBAu32.wrapping_sub(checksum);
        let off = self.ttf().check_sum_adjustment_offset;
        self.fb_seek(off as i32);
        self.put_ulong(k);
        self.fb_seek(save_offset as i32);
    }

    /// writettf.c ttf_write_glyf
    fn ttf_write_glyf(&mut self) {
        let tab = self.ttf_name_lookup(b"glyf", true).unwrap();
        let glyf_offset = self.ttf().dir_tab[tab].offset as i32 as i64;
        let new_glyf_offset = self.fb_offset() as i64;
        self.ttf_reset_chksm(tab);
        let mut id = 0usize;
        while id < self.ttf().new_glyphs_count as usize {
            let gi = self.ttf().glyph_index[id] as usize;
            let off = self.fb_offset() as i64;
            let (o0, o1) = {
                let g = self.ttf().glyph_tab.as_mut().unwrap();
                g[gi].newoffset = (off - new_glyf_offset) as i32;
                (g[gi].offset, g.get(gi + 1).map(|e| e.offset).unwrap_or(0))
            };
            if o0 != o1 {
                self.ttf_seek_off(glyf_offset + o0 as i64);
                let k = self.copy_short();
                self.ttf_ncopy(4 * 2);
                if k < 0 {
                    let mut flags;
                    loop {
                        flags = self.copy_ushort();
                        let idx = self.get_ushort() as usize;
                        let t = self.ttf();
                        let g = t.glyph_tab.as_mut().unwrap();
                        if g[idx].newindex < 0 {
                            g[idx].newindex = t.new_glyphs_count as i16;
                            let n = t.new_glyphs_count as usize;
                            if n >= t.glyph_index.len() {
                                t.glyph_index.resize(n + 1, 0);
                            }
                            t.glyph_index[n] = idx as i32;
                            t.new_glyphs_count += 1;
                        }
                        let ni = self.ttf().glyph_tab.as_ref().unwrap()[idx].newindex;
                        self.put_ushort(ni as i32);
                        if flags & ARG_1_AND_2_ARE_WORDS != 0 {
                            self.ttf_ncopy(2 * 2);
                        } else {
                            self.ttf_ncopy(2);
                        }
                        if flags & WE_HAVE_A_SCALE != 0 {
                            self.ttf_ncopy(2);
                        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
                            self.ttf_ncopy(2 * 2);
                        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
                            self.ttf_ncopy(4 * 2);
                        }
                        if flags & MORE_COMPONENTS == 0 {
                            break;
                        }
                    }
                    if flags & WE_HAVE_INSTRUCTIONS != 0 {
                        let n = self.copy_ushort();
                        self.ttf_ncopy(n as i32);
                    }
                } else {
                    self.ttf_ncopy(o1 - o0 - 2 - 4 * 2);
                }
            }
            id += 1;
        }
        let lgo = (self.fb_offset() as i64 - new_glyf_offset) as u32;
        self.ttf().last_glyf_offset = lgo;
        self.ttf_set_chksm(tab);
    }

    /// writettf.c ttf_reindex_glyphs
    fn ttf_reindex_glyphs(&mut self) {
        let fd = self.ttf().fd_cur;
        let fm = self.fd(fd).fm;
        let fme = self.fm(fm).clone();
        let mut cmap: Option<(Vec<u8>, u16, u16)> = None;
        let mut cmap_not_found = false;
        let gc = self.ttf().glyphs_count as i32;
        for ei in 0..256usize {
            self.ttf().ttfenc_tab[ei].newindex = 0;
            let e = self.ttf().ttfenc_tab[ei].clone();
            let glyph: usize;
            'found: {
                if fme.is_subfont() {
                    if e.code == -1 {
                        break 'found;
                    }
                    if cmap.is_none() && !cmap_not_found {
                        cmap = self.ttf_read_cmap(fme.ff(), fme.pid as i32, fme.eid as i32, true);
                        if cmap.is_none() {
                            cmap_not_found = true;
                        }
                    }
                    let Some(key) = &cmap else { break 'found };
                    let v = self.ttf().cmap_tree[key].get(e.code as usize).copied().unwrap_or(-1);
                    if v < 0 {
                        self.pdftex_warn(&format!(
                            "subfont {}: wrong mapping: character {} --> 0x{:04X} --> .notdef",
                            bs(fme.tfm()),
                            ei,
                            e.code
                        ));
                        break 'found;
                    }
                    glyph = v as usize;
                } else {
                    let Some(name) = e.name.clone() else { break 'found };
                    let byname = self.ttf().glyph_tab.as_ref().unwrap()[..gc as usize]
                        .iter()
                        .position(|g| g.name.as_deref() == Some(&name[..]));
                    if let Some(g) = byname {
                        glyph = g;
                    } else if let Some(index) = scan_uni(&name) {
                        if cmap.is_none() && !cmap_not_found {
                            cmap = self.ttf_read_cmap(fme.ff(), 3, 1, false);
                            if cmap.is_none() {
                                cmap = self.ttf_read_cmap(fme.ff(), 0, 3, false);
                            }
                            if cmap.is_none() {
                                self.pdftex_warn("no unicode mapping found, all `uniXXXX' names will be ignored");
                                cmap_not_found = true;
                            }
                        }
                        let Some(key) = &cmap else { break 'found };
                        let v = self.ttf().cmap_tree[key].get(index as usize).copied().unwrap_or(-1);
                        if v != -1 {
                            if v >= gc {
                                self.pdftex_warn(&format!(
                                    "`{}' is mapped to index {} which is out of valid range [0..{})",
                                    bs(&name),
                                    v,
                                    gc
                                ));
                                break 'found;
                            }
                            glyph = v as usize;
                        } else {
                            self.pdftex_warn(&format!("`unicode uni{:04X}' is not mapped to any glyph", index));
                            break 'found;
                        }
                    } else if let Some(index) = scan_index(&name) {
                        if index >= gc {
                            self.pdftex_warn(&format!("`{}' out of valid range [0..{})", bs(&name), gc));
                            break 'found;
                        }
                        glyph = index as usize;
                    } else {
                        self.pdftex_warn(&format!("glyph `{}' not found", bs(&name)));
                        break 'found;
                    }
                }
                // append_new_glyph:
                let t = self.ttf();
                let g = &mut t.glyph_tab.as_mut().unwrap()[glyph];
                if g.newindex < 0 {
                    let n = t.new_glyphs_count as usize;
                    g.newindex = n as i16;
                    if n >= t.glyph_index.len() {
                        t.glyph_index.resize(n + 1, 0);
                    }
                    t.glyph_index[n] = glyph as i32;
                    t.new_glyphs_count += 1;
                }
                let ni = t.glyph_tab.as_ref().unwrap()[glyph].newindex;
                t.ttfenc_tab[ei].newindex = ni as i32;
            }
        }
    }

    /// writettf.c ttf_write_head
    fn ttf_write_head(&mut self) {
        let tab = self.ttf_seek_tab(b"head", 0);
        self.ttf_reset_chksm(tab);
        self.ttf_ncopy(2 * 4);
        let off = self.fb_offset() as u32;
        self.ttf().check_sum_adjustment_offset = off;
        self.put_ulong(0);
        self.ttf_skip(4);
        self.ttf_ncopy(4 + 2 * 2 + 16 + 4 * 2 + 2 * 2 + 2);
        let fd = self.ttf().fd_cur;
        if self.fm(self.fd(fd).fm).is_subsetted() {
            let lf = self.ttf().loca_format;
            self.put_short(lf as i32);
            self.put_short(0);
        } else {
            self.ttf_ncopy(2 * 2);
        }
        self.ttf_set_chksm(tab);
    }

    /// writettf.c ttf_write_hhea
    fn ttf_write_hhea(&mut self) {
        let tab = self.ttf_seek_tab(b"hhea", 0);
        self.ttf_reset_chksm(tab);
        self.ttf_ncopy(4 + 3 * 2 + 2 + 3 * 2 + 8 * 2);
        let n = self.ttf().new_glyphs_count;
        self.put_ushort(n as i32);
        self.ttf_set_chksm(tab);
    }

    /// writettf.c ttf_write_htmx
    fn ttf_write_htmx(&mut self) {
        let tab = self.ttf_seek_tab(b"hmtx", 0);
        self.ttf_reset_chksm(tab);
        for id in 0..self.ttf().new_glyphs_count as usize {
            let gi = self.ttf().glyph_index[id] as usize;
            let g = self.ttf().glyph_tab.as_ref().unwrap()[gi].clone();
            self.put_ushort(g.adv_width as i32);
            self.put_ushort(g.lsb as u16 as i32);
        }
        self.ttf_set_chksm(tab);
    }

    /// writettf.c ttf_write_loca
    fn ttf_write_loca(&mut self) {
        let tab = self.ttf_seek_tab(b"loca", 0);
        self.ttf_reset_chksm(tab);
        let n = self.ttf().new_glyphs_count as usize;
        let offs: Vec<i32> = (0..n)
            .map(|id| {
                let t = &self.rt.fonts.ttf;
                t.glyph_tab.as_ref().unwrap()[t.glyph_index[id] as usize].newoffset
            })
            .collect();
        let lgo = self.ttf().last_glyf_offset;
        let mut lf = 0;
        if lgo >= 0x00020000 || (lgo & 1) != 0 {
            lf = 1;
        } else if offs.iter().any(|o| o & 1 != 0) {
            lf = 1;
        }
        self.ttf().loca_format = lf;
        if lf != 0 {
            for o in offs {
                self.put_ulong(o as u32);
            }
            self.put_ulong(lgo);
        } else {
            for o in offs {
                self.put_ushort(o / 2);
            }
            self.put_ushort((lgo / 2) as i32);
        }
        self.ttf_set_chksm(tab);
    }

    /// writettf.c ttf_write_mapx
    fn ttf_write_mapx(&mut self) {
        let tab = self.ttf_seek_tab(b"maxp", 4 + 2);
        self.ttf_reset_chksm(tab);
        self.put_ulong(0x00010000);
        let n = self.ttf().new_glyphs_count;
        self.put_ushort(n as i32);
        self.ttf_ncopy(13 * 2);
        self.ttf_set_chksm(tab);
    }

    /// writettf.c ttf_write_OS2
    fn ttf_write_os2(&mut self) {
        let tab = self.ttf_seek_tab(b"OS/2", 0);
        self.ttf_reset_chksm(tab);
        let version = self.get_ushort();
        if version > 5 {
            self.pdftex_warn(&format!("unknown version of OS/2 table ({:04X})", version));
        }
        self.put_ushort(0x0001);
        self.ttf_ncopy(2 * 2 + 13 * 2 + 10);
        self.ttf_skip(4 * 4);
        self.put_ulong(0x00000003);
        self.put_ulong(0x10000000);
        self.put_ulong(0);
        self.put_ulong(0);
        self.ttf_ncopy(4 + 2);
        self.ttf_skip(2 * 2);
        self.put_ushort(0x0000);
        self.put_ushort(0xF0FF);
        self.ttf_ncopy(5 * 2);
        self.put_ulong(0x80000000);
        self.put_ulong(0);
        self.ttf_set_chksm(tab);
    }

    /// writettf.c ttf_write_post
    fn ttf_write_post(&mut self) {
        let tab = self.ttf_seek_tab(b"post", 4);
        self.ttf_reset_chksm(tab);
        let fd = self.ttf().fd_cur;
        if !self.fd(fd).write_ttf_glyph_names || self.ttf().post_format == 0x00030000 {
            self.put_ulong(0x00030000);
            self.ttf_ncopy(4 + 2 * 2 + 5 * 4);
        } else {
            self.put_ulong(0x00020000);
            self.ttf_ncopy(4 + 2 * 2 + 5 * 4);
            let n = self.ttf().new_glyphs_count as usize;
            self.put_ushort(n as i32);
            // `k' is TeX's global k (unsigned char) in the C code
            self.k = 0;
            for id in 0..n {
                let gi = self.ttf().glyph_index[id] as usize;
                let g = self.ttf().glyph_tab.as_ref().unwrap()[gi].clone();
                let unsafe_name = AMBIGUOUS_NAMES.iter().any(|&a| gname(&g.name) == a);
                let mut ni = g.name_index;
                if ni as usize >= NMACGLYPHS || unsafe_name {
                    ni = (NMACGLYPHS as u32 + self.k as u32) as u16;
                    self.k = self.k.wrapping_add(1);
                    self.ttf().glyph_tab.as_mut().unwrap()[gi].name_index = ni;
                }
                self.put_ushort(ni as i32);
            }
            for id in 0..n {
                let gi = self.ttf().glyph_index[id] as usize;
                let g = self.ttf().glyph_tab.as_ref().unwrap()[gi].clone();
                if g.name_index as usize >= NMACGLYPHS {
                    let s = gname(&g.name).to_vec();
                    self.put_byte(s.len() as i32);
                    for c in s {
                        self.put_byte(c as i8 as i32);
                    }
                }
            }
        }
        self.ttf_set_chksm(tab);
    }

    /// writettf.c ttf_init_font
    fn ttf_init_font(&mut self, n: i32) {
        let mut i = 1;
        let mut k = 0;
        while i <= n {
            i <<= 1;
            k += 1;
        }
        self.put_ulong(0x00010000);
        self.put_ushort(n);
        self.put_ushort(i << 3);
        self.put_ushort(k - 1);
        self.put_ushort((n << 4) - (i << 3));
        self.fb_seek(TABDIR_OFF + n * 4 * 4);
    }

    /// writettf.c ttf_subset_font
    fn ttf_subset_font(&mut self) {
        let n = self.ttf().new_ntabs as i32;
        self.ttf_init_font(n);
        for t in [b"PCLT", b"fpgm", b"cvt ", b"prep"] {
            if self.ttf_name_lookup(t, false).is_some() {
                self.ttf_copytab(t);
            }
        }
        self.ttf_reindex_glyphs();
        self.ttf_write_glyf();
        self.ttf_write_loca();
        self.ttf_write_os2();
        self.ttf_write_head();
        self.ttf_write_hhea();
        self.ttf_write_htmx();
        self.ttf_write_mapx();
        self.ttf_write_name();
        self.ttf_write_post();
        self.ttf_write_cmap();
        self.ttf_write_dirtab();
    }

    /// writettf.c ttf_copy_font
    fn ttf_copy_font(&mut self) {
        let n = self.ttf().ntabs as i32;
        self.ttf_init_font(n);
        for i in 0..n as usize {
            let tag = self.ttf().dir_tab[i].tag;
            if &tag == b"head" {
                self.ttf_write_head();
            } else {
                self.ttf_copytab(&tag);
            }
        }
        self.ttf_write_dirtab();
    }

    /// writettf.c writettf
    pub fn writettf(&mut self, fd: usize) {
        self.ttf().fd_cur = fd;
        let fm = self.fd(fd).fm;
        let e = self.fm(fm).clone();
        self.set_cur_file_name(e.ff());
        if e.is_subsetted() && self.fd(fd).fe.is_none() && !e.is_subfont() {
            self.pdftex_fail("Subset TrueType must be a reencoded or a subfont");
        }
        let f = match self.font_open_input(kpse::TRUETYPE_FORMAT) {
            Some(f) => f,
            None => self.pdftex_fail("cannot open TrueType font file for reading"),
        };
        self.ttf().file = f;
        self.cur_file_name_from_nameoffile();
        let cur = self.name_of_file();
        if cur.len() >= 4 && strcasecmp_eq(&cur[cur.len() - 4..], b".ttc") {
            if self.get_ulong() != 0x74746366 {
                self.ttf().file.seek(0);
            } else {
                self.ttf_skip(4);
                self.ttf_skip(4);
                let o = self.get_ulong();
                self.ttf().file.seek(o as i32 as i64);
            }
        }
        let mut m = if e.is_subsetted() { b"<".to_vec() } else { b"<<".to_vec() };
        m.extend_from_slice(&cur);
        self.tex_printf_b(&m);
        self.fd_mut(fd).ff_found = true;
        {
            let t = self.ttf();
            t.new_glyphs_count = 2;
            t.new_ntabs = DEFAULT_NTABS;
            t.dir_tab.clear();
            t.glyph_tab = None;
            t.glyph_index.clear();
            t.name_tab.clear();
            t.name_buf.clear();
            if t.ttfenc_tab.len() < 256 {
                t.ttfenc_tab = vec![TtfencEntry::default(); 256];
            }
        }
        self.ttf_read_font();
        self.pdfsaveoffset = self.pdfgone + self.pdfptr as i64;
        self.pdfflush();
        if e.is_subsetted() {
            self.ttf_copy_encoding();
            self.ttf_subset_font();
        } else {
            self.ttf_copy_font();
        }
        let l = self.fb_offset();
        let t = self.ttf();
        t.ttf_length = l;
        t.dir_tab.clear();
        t.glyph_tab = None;
        t.glyph_index.clear();
        t.name_tab.clear();
        t.name_buf.clear();
        t.file = CFile::default();
        self.tex_printf(if e.is_subsetted() { ">" } else { ">>" });
        self.set_cur_file_name_none();
    }

    /// writettf.c writeotf
    pub fn writeotf(&mut self, fd: usize) {
        self.ttf().fd_cur = fd;
        let fm = self.fd(fd).fm;
        let e = self.fm(fm).clone();
        self.set_cur_file_name(e.ff());
        if e.is_subsetted() {
            self.pdftex_fail("OTF fonts must be included entirely");
        }
        let f = match self.font_open_input(kpse::OPENTYPE_FORMAT) {
            Some(f) => f,
            None => self.pdftex_fail("cannot open OpenType font file for reading"),
        };
        self.ttf().file = f;
        self.cur_file_name_from_nameoffile();
        let cur = self.name_of_file();
        let mut m = b"<<".to_vec();
        m.extend_from_slice(&cur);
        self.tex_printf_b(&m);
        self.fd_mut(fd).ff_found = true;
        self.ttf().dir_tab.clear();
        self.ttf().glyph_tab = None;
        self.ttf_read_tabdir();
        if self.ttf_name_lookup(b"head", false).is_some() {
            self.ttf_read_head();
        }
        if self.ttf_name_lookup(b"hhea", false).is_some() {
            self.ttf_read_hhea();
        }
        if self.ttf_name_lookup(b"PCLT", false).is_some() {
            self.ttf_read_pclt();
        }
        if self.ttf_name_lookup(b"post", false).is_some() {
            self.ttf_read_post();
        }
        let tab = self.ttf_seek_tab(b"CFF ", 0);
        let mut i = self.ttf().dir_tab[tab].length as i32;
        while i > 0 {
            self.copy_char();
            i -= 1;
        }
        self.ttf().dir_tab.clear();
        self.ttf().file = CFile::default();
        self.tex_printf(">>");
        self.set_cur_file_name_none();
    }

    /// writettf.c ttf_free
    pub fn ttf_free(&mut self) {
        self.ttf().cmap_tree.clear();
    }
}

/// sscanf(name, "uni%X%n", &index, &n) with n == strlen(name)
fn scan_uni(name: &[u8]) -> Option<i32> {
    let rest = name.strip_prefix(b"uni")?;
    let mut s = rest.to_vec();
    s.push(0);
    let (v, e) = scan_int(&s, 0, 16)?;
    if e + 3 == name.len() {
        Some(v as i32)
    } else {
        None
    }
}

/// sscanf(name, "index%i%n", &index, &n) with n == strlen(name)
fn scan_index(name: &[u8]) -> Option<i32> {
    let rest = name.strip_prefix(b"index")?;
    let mut s = rest.to_vec();
    s.push(0);
    let (v, e) = scan_int(&s, 0, 0)?;
    if e + 5 == name.len() {
        Some(v as i32)
    } else {
        None
    }
}
