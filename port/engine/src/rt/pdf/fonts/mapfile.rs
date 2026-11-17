//! Port of pdftexdir/mapfile.c: handling of map files/lines.

use super::*;
use crate::gen::globals::Globals;
use crate::rt::fxhash::{FxHashMap, FxHashSet};
use crate::rt::kpse;

/// mapfile.c ps_tree: (ps_name, slant, extend) -> fm_entry. Keyed by name first so that
/// lookups need no allocation; mapfile.c only looks entries up, inserts and deletes them
/// (no ordered traversal), so a hash map behaves exactly like the AVL tree.
/// fm_entry storage: entries restored from the map cache (super::mapcache::LazyMap) are
/// decoded on first access; entries added later are stored directly.
#[derive(Default)]
pub struct FmVec {
    cells: Vec<std::cell::OnceCell<Box<FmEntry>>>,
    lazy: Option<std::rc::Rc<super::mapcache::LazyMap>>,
}

impl FmVec {
    pub fn len(&self) -> usize {
        self.cells.len()
    }
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
    pub fn reserve(&mut self, n: usize) {
        self.cells.reserve(n);
    }
    pub fn push(&mut self, e: FmEntry) {
        self.cells.push(std::cell::OnceCell::from(Box::new(e)));
    }
    /// Entries 0..lazy.n are the restored ones.
    pub fn set_lazy(&mut self, l: std::rc::Rc<super::mapcache::LazyMap>) {
        self.cells = (0..l.n).map(|_| std::cell::OnceCell::new()).collect();
        self.lazy = Some(l);
    }
    fn decode(&self, i: usize) -> Box<FmEntry> {
        Box::new(self.lazy.as_ref().expect("lazy map entry").entry(i))
    }
}

impl std::ops::Index<usize> for FmVec {
    type Output = FmEntry;
    fn index(&self, i: usize) -> &FmEntry {
        self.cells[i].get_or_init(|| self.decode(i))
    }
}

impl std::ops::IndexMut<usize> for FmVec {
    fn index_mut(&mut self, i: usize) -> &mut FmEntry {
        if self.cells[i].get().is_none() {
            let b = self.decode(i);
            let _ = self.cells[i].set(b);
        }
        self.cells[i].get_mut().unwrap()
    }
}

/// mapfile.c tfm_tree: tfm name -> fm_entry. Entries restored from the map cache stay in
/// its sorted index (binary search); changes go to an overlay (None = deleted).
#[derive(Default)]
pub struct TfmTree {
    overlay: FxHashMap<Vec<u8>, Option<usize>>,
    base: Option<std::rc::Rc<super::mapcache::LazyMap>>,
}

impl TfmTree {
    pub fn get(&self, name: &[u8]) -> Option<usize> {
        match self.overlay.get(name) {
            Some(v) => *v,
            None => self.base.as_ref().and_then(|b| b.tfm_get(name)),
        }
    }
    pub fn contains_key(&self, name: &[u8]) -> bool {
        self.get(name).is_some()
    }
    pub fn insert(&mut self, name: Vec<u8>, i: usize) {
        self.overlay.insert(name, Some(i));
    }
    pub fn remove(&mut self, name: &[u8]) {
        if self.base.is_some() {
            self.overlay.insert(name.to_vec(), None);
        } else {
            self.overlay.remove(name);
        }
    }
    pub fn clear(&mut self) {
        self.overlay.clear();
        self.base = None;
    }
    pub fn reserve(&mut self, n: usize) {
        self.overlay.reserve(n);
    }
    pub fn set_base(&mut self, l: std::rc::Rc<super::mapcache::LazyMap>) {
        self.overlay.clear();
        self.base = Some(l);
    }
    /// All (name, fm_entry) pairs.
    pub fn entries(&self) -> Vec<(Vec<u8>, usize)> {
        let mut v: Vec<(Vec<u8>, usize)> = match &self.base {
            Some(b) => b.tfm_all().into_iter().filter(|(n, _)| !self.overlay.contains_key(n)).collect(),
            None => Vec::new(),
        };
        v.extend(self.overlay.iter().filter_map(|(n, i)| i.map(|i| (n.clone(), i))));
        v
    }
    pub fn is_empty(&self) -> bool {
        match &self.base {
            Some(b) if b.tfm_count() > 0 => self.entries().is_empty(),
            _ => self.overlay.values().all(|v| v.is_none()),
        }
    }
}

#[derive(Default)]
pub struct PsTree {
    map: FxHashMap<Vec<u8>, Vec<(i32, i32, usize)>>,
    /// entries restored from the map cache, inserted on first use
    pending: Vec<(Vec<u8>, i32, i32, usize)>,
    lazy: Option<std::rc::Rc<super::mapcache::LazyMap>>,
}

impl PsTree {
    fn settle(&mut self) {
        if let Some(l) = self.lazy.take() {
            self.pending = l.ps_entries();
        }
        if !self.pending.is_empty() {
            for (name, slant, extend, i) in std::mem::take(&mut self.pending) {
                self.insert_if_absent(&name, slant, extend, i);
            }
        }
    }
    pub fn get(&mut self, name: &[u8], slant: i32, extend: i32) -> Option<usize> {
        self.settle();
        self.map.get(name)?.iter().find(|e| e.0 == slant && e.1 == extend).map(|e| e.2)
    }
    /// avl_probe: insert unless an entry with the same key exists.
    pub fn insert_if_absent(&mut self, name: &[u8], slant: i32, extend: i32, fm: usize) {
        self.settle();
        if let Some(v) = self.map.get_mut(name) {
            if !v.iter().any(|e| e.0 == slant && e.1 == extend) {
                v.push((slant, extend, fm));
            }
        } else {
            self.map.insert(name.to_vec(), vec![(slant, extend, fm)]);
        }
    }
    pub fn remove(&mut self, name: &[u8], slant: i32, extend: i32) {
        self.settle();
        if let Some(v) = self.map.get_mut(name) {
            v.retain(|e| !(e.0 == slant && e.1 == extend));
        }
    }
    pub fn clear(&mut self) {
        self.map.clear();
        self.pending.clear();
        self.lazy = None;
    }
    pub fn is_empty(&self) -> bool {
        self.lazy.is_none() && self.pending.is_empty() && self.map.values().all(|v| v.is_empty())
    }
    /// Restore from the map cache (decoded on first use).
    pub fn set_lazy(&mut self, l: std::rc::Rc<super::mapcache::LazyMap>) {
        self.map.clear();
        self.pending.clear();
        self.lazy = Some(l);
    }
    pub fn map(&mut self) -> &FxHashMap<Vec<u8>, Vec<(i32, i32, usize)>> {
        self.settle();
        &self.map
    }
    /// All entries (name, slant, extend, fm_entry), in insertion order per name.
    pub fn entries(&mut self) -> Vec<(Vec<u8>, i32, i32, usize)> {
        self.settle();
        let mut v = Vec::new();
        for (name, list) in &self.map {
            for &(sl, ex, i) in list {
                v.push((name.clone(), sl, ex, i));
            }
        }
        v
    }
    /// Restore from the map cache: entries in their original insertion order.
    pub fn set_pending(&mut self, v: Vec<(Vec<u8>, i32, i32, usize)>) {
        self.map.clear();
        self.pending = v;
    }
}

const FM_BUF_SIZE: usize = 1024;

// typedef enum { FM_DUPIGNORE, FM_REPLACE, FM_DELETE } updatemode;
pub const FM_DUPIGNORE: i32 = 0;
pub const FM_REPLACE: i32 = 1;
pub const FM_DELETE: i32 = 2;
// typedef enum { MAPFILE, MAPLINE } maptype;
pub const MAPFILE: i32 = 0;
pub const MAPLINE: i32 = 1;

const NONTFM: &[u8] = b"<nontfm>";

/// mapfile.c mapitem
#[derive(Clone, Debug)]
pub struct MapItem {
    pub mode: i32,
    pub type_: i32,
    pub line: Option<Vec<u8>>,
    pub lineno: i32,
}

/// mapfile.c static state. `fm[0]` is dummy_fm_entry()'s static (all zero) entry.
#[derive(Default)]
pub struct MapState {
    pub mitem: Option<MapItem>,
    pub fm: FmVec,
    /// tfm_tree != NULL (all four trees are created together)
    pub trees: bool,
    /// The AVL trees of mapfile.c are only looked up, inserted into and deleted from
    /// (never traversed in order), so hash maps give identical results.
    pub tfm_tree: TfmTree,
    pub ps_tree: PsTree,
    /// ff_tree: ff_name -> ff_path
    pub ff_tree: FxHashMap<Vec<u8>, Option<Vec<u8>>>,
    pub encname_tree: FxHashSet<Vec<u8>>,
    fm_file: Option<CFile>,
    /// fm_scan_line's line buffer, reused from line to line
    line_buf: Vec<u8>,
    /// warnings printed while a map file is parsed for the map cache (mapcache.rs), and
    /// whether the parse did something the cache cannot reproduce (read .sfd files)
    pub capture: Option<Vec<String>>,
    pub uncacheable: bool,
}

pub const DUMMY_FM: usize = 0;

/// Encoding of an fm_entry index as the `fmentryptr` stored in pdffontmap[].
pub fn fm_to_ptr(i: usize) -> Ptr<i32> {
    Ptr::from_raw(((i + 1) * 16) as *mut i32)
}
pub fn fm_from_ptr(p: Ptr<i32>) -> usize {
    p.addr() / 16 - 1
}

/// mapfile.c new_fm_entry
pub fn new_fm_entry() -> FmEntry {
    FmEntry { fd_flags: FD_FLAGS_NOT_SET_IN_MAPLINE, pid: -1, eid: -1, ..Default::default() }
}

/// mapfile.c check_std_t1font: the font number if s is one of the 14 standard font names.
pub fn check_std_t1font(s: &[u8]) -> i32 {
    const NAMES: [&[u8]; 14] = [
        b"Courier",
        b"Courier-Bold",
        b"Courier-Oblique",
        b"Courier-BoldOblique",
        b"Helvetica",
        b"Helvetica-Bold",
        b"Helvetica-Oblique",
        b"Helvetica-BoldOblique",
        b"Symbol",
        b"Times-Roman",
        b"Times-Bold",
        b"Times-Italic",
        b"Times-BoldItalic",
        b"ZapfDingbats",
    ];
    const INDEX: [i32; 22] = [-1, -1, -1, -1, -1, -1, 8, 0, -1, 4, 10, 9, -1, -1, 5, 2, 12, 6, -1, 3, -1, 7];
    let n = s.len();
    if n > 21 {
        return -1;
    }
    let k = if n == 12 {
        match s[0] {
            b'C' => 1,
            b'T' => 11,
            b'Z' => 13,
            _ => return -1,
        }
    } else {
        INDEX[n]
    };
    if k > -1 && NAMES[k as usize] == s {
        return k;
    }
    -1
}

fn is_cfg_comment(c: u8) -> bool {
    c == 10 || c == b'*' || c == b'#' || c == b';' || c == b'%'
}

impl Globals {
    pub fn fm(&self, i: usize) -> &FmEntry {
        &self.rt.fonts.map.fm[i]
    }
    pub fn fm_mut(&mut self, i: usize) -> &mut FmEntry {
        &mut self.rt.fonts.map.fm[i]
    }

    /// Put an fm_entry into the arena (xtalloc).
    pub fn fm_alloc(&mut self, fm: FmEntry) -> usize {
        let a = &mut self.rt.fonts.map.fm;
        if a.is_empty() {
            a.push(FmEntry::default());
        }
        a.push(fm);
        a.len() - 1
    }

    /// mapfile.c create_avl_trees
    fn create_avl_trees(&mut self) {
        let m = &mut self.rt.fonts.map;
        m.trees = true;
        m.tfm_tree.clear();
        m.ps_tree.clear();
        m.ff_tree.clear();
        m.encname_tree.clear();
        m.tfm_tree.reserve(8192);
        m.fm.reserve(8192);
        if m.fm.is_empty() {
            m.fm.push(FmEntry::default());
        }
    }

    /// mapfile.c avl_do_entry: returns 1 if the fm_entry is to be deallocated.
    pub fn avl_do_entry(&mut self, fm: usize, mode: i32) -> i32 {
        let suppress_warn = self.getpdfsuppresswarningdupmap() > 0;
        'exit: {
            // handle tfm_name link
            if self.fm(fm).tfm() != NONTFM {
                let found = {
                    let m = &self.rt.fonts.map;
                    m.tfm_tree.get(m.fm[fm].tfm())
                };
                if let Some(p) = found {
                    match mode {
                        FM_DUPIGNORE => {
                            if !suppress_warn {
                                let tfm = self.fm(fm).tfm().to_vec();
                                self.pdftex_warn(&format!("fontmap entry for `{}' already exists, duplicates ignored", bs(&tfm)));
                            }
                            break 'exit;
                        }
                        FM_REPLACE | FM_DELETE => {
                            if self.fm(p).in_use {
                                let tfm = self.fm(fm).tfm().to_vec();
                                self.pdftex_warn(&format!("fontmap entry for `{}' has been used, replace/delete not allowed", bs(&tfm)));
                                break 'exit;
                            }
                            let m = &mut self.rt.fonts.map;
                            m.tfm_tree.remove(m.fm[fm].tfm());
                            self.fm_mut(p).links &= !LINK_TFM;
                        }
                        _ => unreachable!(),
                    }
                }
                if mode != FM_DELETE {
                    // avl_probe: the key is absent at this point (not found, or just removed)
                    let m = &mut self.rt.fonts.map;
                    if !m.tfm_tree.contains_key(m.fm[fm].tfm()) {
                        let key = m.fm[fm].tfm().to_vec();
                        m.tfm_tree.insert(key, fm);
                    }
                    self.fm_mut(fm).links |= LINK_TFM;
                }
            }
            // handle ps_name link
            let (has_ps, slant, extend) = {
                let e = self.fm(fm);
                (e.ps_name.is_some(), e.slant, e.extend)
            };
            if has_ps {
                let found = {
                    let m = &mut self.rt.fonts.map;
                    let name = m.fm[fm].ps_name.clone().unwrap();
                    m.ps_tree.get(&name, slant, extend)
                };
                if let Some(p) = found {
                    match mode {
                        FM_DUPIGNORE => break 'exit,
                        FM_REPLACE | FM_DELETE => {
                            if self.fm(p).in_use {
                                break 'exit;
                            }
                            let m = &mut self.rt.fonts.map;
                            let name = m.fm[fm].ps_name.clone().unwrap();
                            m.ps_tree.remove(&name, slant, extend);
                            self.fm_mut(p).links &= !LINK_PS;
                        }
                        _ => unreachable!(),
                    }
                }
                let e = self.fm(fm);
                if mode != FM_DELETE && e.is_t1fontfile() && e.is_included() {
                    let m = &mut self.rt.fonts.map;
                    let name = m.fm[fm].ps_name.clone().unwrap();
                    m.ps_tree.insert_if_absent(&name, slant, extend, fm);
                    self.fm_mut(fm).links |= LINK_PS;
                }
            }
        }
        let e = self.fm(fm);
        if !e.has_tfmlink() && !e.has_pslink() {
            1
        } else {
            0
        }
    }

    /// mapfile.c add_encname
    fn add_encname(&mut self, s: &[u8]) -> Vec<u8> {
        if !self.rt.fonts.map.encname_tree.contains(s) {
            self.rt.fonts.map.encname_tree.insert(s.to_vec());
        }
        s.to_vec()
    }

    /// mapfile.c check_fm_entry: zero if ok, nonzero bits of warnings otherwise.
    fn check_fm_entry(&mut self, fm: &mut FmEntry, warn: bool) -> i32 {
        let mut a = 0;
        if fm.is_fontfile() && !fm.is_included() {
            if warn {
                self.pdftex_warn(&format!(
                    "ambiguous entry for `{}': font file present but not included, will be treated as font file not present",
                    bs(fm.tfm())
                ));
            }
            fm.ff_name = None;
        }
        if fm.tfm().is_empty() {
            if warn {
                self.pdftex_warn("invalid map entry: tfm missing");
            }
            a += 1;
        }
        if fm.is_truetype() && fm.is_reencoded() && !fm.is_subsetted() {
            if warn {
                self.pdftex_warn(&format!("invalid entry for `{}': only subsetted TrueType fonts can be reencoded", bs(fm.tfm())));
            }
            a += 2;
        }
        if (fm.slant != 0 || fm.extend != 0) && (fm.tfm().is_empty() || !(fm.is_t1fontfile() && fm.is_included())) {
            if warn {
                self.pdftex_warn(&format!(
                    "invalid entry for `{}': SlantFont/ExtendFont can be used only with embedded Type1 fonts",
                    bs(fm.tfm())
                ));
            }
            a += 4;
        }
        if fm.slant.wrapping_abs() > 1000 {
            if warn {
                self.pdftex_warn(&format!("invalid entry for `{}': SlantFont value too big: {}", bs(fm.tfm()), fmt_g(fm.slant as f64 / 1000.0)));
            }
            a += 8;
        }
        if fm.extend.wrapping_abs() > 2000 {
            if warn {
                self.pdftex_warn(&format!("invalid entry for `{}': ExtendFont value too big: {}", bs(fm.tfm()), fmt_g(fm.extend as f64 / 1000.0)));
            }
            a += 16;
        }
        if fm.pid != -1 && !(fm.is_truetype() && fm.is_subsetted() && !fm.is_reencoded()) {
            if warn {
                self.pdftex_warn(&format!(
                    "invalid entry for `{}': PidEid can be used only with subsetted non-reencoded TrueType fonts",
                    bs(fm.tfm())
                ));
            }
            a += 32;
        }
        if fm.is_fontfile() && fm.is_pk() {
            if warn {
                self.pdftex_warn(&format!(
                    "invalid entry for `{}': FontFile cannot be specified for bitmap PK font: {}",
                    bs(fm.tfm()),
                    bs(fm.ff())
                ));
            }
            a += 64;
        }
        if fm.ps_name.is_some() && fm.is_pk() {
            if warn {
                self.pdftex_warn(&format!(
                    "invalid entry for `{}': PsName cannot be specified for bitmap PK font: {}",
                    bs(fm.tfm()),
                    bs(fm.ps_name.as_deref().unwrap())
                ));
            }
            a += 128;
        }
        a
    }

    /// read_field(r, q, buf, FM_BUF_SIZE)
    fn fm_read_field(&mut self, s: &[u8], r: &mut usize) -> Vec<u8> {
        let mut buf = Vec::new();
        while !matches!(at(s, *r), b' ' | b'<' | b'"' | 0) {
            if buf.len() + 1 > FM_BUF_SIZE {
                self.check_buf_fail("mapfile.c", 53);
            }
            buf.push(s[*r]);
            *r += 1;
        }
        if at(s, *r) == b' ' {
            *r += 1;
        }
        buf
    }

    /// mapfile.c fm_scan_line
    fn fm_scan_line(&mut self) {
        let (mode, type_) = {
            let mi = self.rt.fonts.map.mitem.as_ref().expect("mitem");
            (mi.mode, mi.type_)
        };
        // the line buffer is reused from line to line (this runs for every map file line)
        let mut s = std::mem::take(&mut self.rt.fonts.map.line_buf);
        s.clear();
        match type_ {
            MAPFILE => {
                let f = self.rt.fonts.map.fm_file.as_mut().unwrap();
                let mut overflow = false;
                loop {
                    let mut c = f.getc();
                    // append_char_to_buf(c, p, fm_line, FM_BUF_SIZE)
                    if c == 9 {
                        c = 32;
                    }
                    if c == 13 || c == -1 {
                        c = 10;
                    }
                    if c != b' ' as i32 || (!s.is_empty() && *s.last().unwrap() != 32) {
                        if s.len() + 1 > FM_BUF_SIZE {
                            overflow = true;
                            break;
                        }
                        s.push(c as u8);
                    }
                    if c == 10 {
                        break;
                    }
                }
                if overflow {
                    self.check_buf_fail("mapfile.c", 440);
                }
                s.pop();
            }
            _ => {
                if let Some(l) = self.rt.fonts.map.mitem.as_ref().and_then(|m| m.line.as_ref()) {
                    s.extend_from_slice(l);
                }
            }
        }
        if let Some(z) = s.iter().position(|&c| c == 0) {
            s.truncate(z);
        }
        s.push(0);
        self.fm_scan_line_parse(&s, mode);
        self.rt.fonts.map.line_buf = s;
    }

    /// The parsing part of mapfile.c fm_scan_line, for the NUL-terminated line `s`.
    #[inline(never)]
    fn fm_scan_line_parse(&mut self, s: &[u8], mode: i32) {
        let mut r = 0usize;
        if at(s, r) == 0 || is_cfg_comment(at(s, r)) {
            return;
        }
        let mut fm = new_fm_entry();
        let mut u: u8 = 0;
        let mut v: u8 = 0;
        let mut bad = false;
        'done: {
            let buf = self.fm_read_field(s, &mut r);
            if !buf.is_empty() {
                fm.tfm_name = Some(buf);
            }
            if at(s, r) == 0 {
                break 'done;
            }
            if fm.tfm_name.is_none() {
                fm.tfm_name = Some(Vec::new());
            }
            if !at(s, r).is_ascii_digit() {
                let buf = self.fm_read_field(s, &mut r);
                if !buf.is_empty() {
                    fm.ps_name = Some(buf);
                }
                if at(s, r) == 0 {
                    break 'done;
                }
            }
            if at(s, r).is_ascii_digit() {
                let mut t = r;
                while at(s, t).is_ascii_digit() {
                    t += 1;
                }
                if matches!(at(s, t), b' ' | b'"' | b'<' | 0) {
                    fm.fd_flags = atoi(s, r);
                    while at(s, r).is_ascii_digit() {
                        r += 1;
                    }
                }
            }
            loop {
                if at(s, r) == b' ' {
                    r += 1;
                }
                match at(s, r) {
                    0 => break 'done,
                    b'"' => {
                        r += 1;
                        u = 0;
                        v = 0;
                        loop {
                            if at(s, r) == b' ' {
                                r += 1;
                            }
                            if let Some((d, e)) = scan_float(s, r) {
                                let sp = skip_ws(s, e);
                                if str_prefix(&s[sp..], b"SlantFont") {
                                    let d = (d as f64 * 1000.0) as f32;
                                    fm.slant = (if d > 0.0 { d as f64 + 0.5 } else { d as f64 - 0.5 }) as i32;
                                    r = sp + 9;
                                } else if str_prefix(&s[sp..], b"ExtendFont") {
                                    let d = (d as f64 * 1000.0) as f32;
                                    fm.extend = (if d > 0.0 { d as f64 + 0.5 } else { d as f64 - 0.5 }) as i32;
                                    if fm.extend == 1000 {
                                        fm.extend = 0;
                                    }
                                    r = sp + 10;
                                } else {
                                    r = sp;
                                    while !matches!(at(s, r), b' ' | b'"' | 0) {
                                        r += 1;
                                    }
                                    let msg = format!(
                                        "invalid entry for `{}': unknown name `{}' ignored",
                                        bs(fm.tfm()),
                                        bs(&s[sp..r])
                                    );
                                    self.pdftex_warn(&msg);
                                }
                            } else {
                                while !matches!(at(s, r), b' ' | b'"' | 0) {
                                    r += 1;
                                }
                            }
                            if at(s, r) != b' ' {
                                break;
                            }
                        }
                        if at(s, r) == b'"' {
                            r += 1;
                        } else {
                            let msg = format!("invalid entry for `{}': closing quote missing", bs(fm.tfm()));
                            self.pdftex_warn(&msg);
                            bad = true;
                            break 'done;
                        }
                    }
                    c => {
                        if c == b'P' {
                            // sscanf(r, "PidEid=%i, %i %n", &a, &b, &c) >= 2
                            let m = scan_lit(s, r, b"PidEid=").and_then(|i1| {
                                let (a, i2) = scan_int(s, i1, 0)?;
                                let i3 = scan_lit(s, i2, b",")?;
                                let (b, i4) = scan_int(s, skip_ws(s, i3), 0)?;
                                Some((a, b, skip_ws(s, i4)))
                            });
                            if let Some((a, b, e)) = m {
                                fm.pid = a as i32 as i16;
                                fm.eid = b as i32 as i16;
                                r = e;
                                continue;
                            }
                        }
                        let mut a: u8 = 0;
                        let mut b: u8 = 0;
                        if at(s, r) == b'<' {
                            a = b'<';
                            r += 1;
                            if at(s, r) == b'<' || at(s, r) == b'[' {
                                b = at(s, r);
                                r += 1;
                            }
                        }
                        let buf = self.fm_read_field(s, &mut r);
                        if buf.len() > 4 && strcasecmp_eq(&buf[buf.len() - 4..], b".enc") {
                            fm.encname = Some(self.add_encname(&buf));
                            u = 0;
                            v = 0;
                        } else if !buf.is_empty() {
                            if a == b'<' || u == b'<' {
                                fm.type_ |= F_INCLUDED;
                                if (a == b'<' && b == 0) || (a == 0 && v == 0) {
                                    fm.type_ |= F_SUBSETTED;
                                }
                            }
                            fm.ff_name = Some(buf);
                            if at(s, r) == 0 {
                                break 'done;
                            }
                            u = 0;
                            v = 0;
                        } else {
                            u = a;
                            v = b;
                        }
                    }
                }
            }
        }
        if !bad {
            if let Some(ps) = &fm.ps_name {
                if check_std_t1font(ps) >= 0 {
                    fm.type_ |= F_STDT1FONT;
                }
            }
            if fm.is_fontfile() && fm.ff().len() > 3 {
                let ff = fm.ff();
                let ext = &ff[ff.len() - 4..];
                if strcasecmp_eq(ext, b".ttf") || strcasecmp_eq(ext, b".ttc") {
                    fm.type_ |= F_TRUETYPE;
                } else if strcasecmp_eq(ext, b".otf") {
                    fm.type_ |= F_OTF;
                } else {
                    fm.type_ |= F_TYPE1;
                }
            } else if fm.ps_name.is_none() {
                fm.type_ |= F_PK;
            } else {
                fm.type_ |= F_TYPE1;
            }
            if self.check_fm_entry(&mut fm, true) != 0 {
                return; // bad_line: delete_fm_entry(fm)
            }
            let fm = match self.handle_subfont_fm(fm, mode) {
                None => return,
                Some(fm) => fm,
            };
            let idx = self.fm_alloc(fm);
            if self.avl_do_entry(idx, mode) == 0 {
                return;
            }
        }
        // bad_line: delete_fm_entry(fm)
    }

    /// mapfile.c fm_read_info
    pub fn fm_read_info(&mut self) {
        crate::rt::phase::mark("fm_read_info");
        if !self.rt.fonts.map.trees {
            self.create_avl_trees();
        }
        let mi = self.rt.fonts.map.mitem.clone().expect("mitem");
        let Some(line) = mi.line.clone() else { return };
        self.rt.fonts.map.mitem.as_mut().unwrap().lineno = 1;
        match mi.type_ {
            MAPFILE => {
                self.set_cur_file_name(&line);
                crate::rt::phase::mark("map: open");
                match self.font_open_input(kpse::FONTMAP_FORMAT) {
                    None => self.pdftex_warn("cannot open font map file"),
                    Some(f) => {
                        crate::rt::phase::mark("map: opened+read");
                        self.cur_file_name_from_nameoffile();
                        let n = self.name_of_file();
                        let mut msg = b"{".to_vec();
                        msg.extend_from_slice(&n);
                        self.tex_printf_b(&msg);
                        // the first map file of the run: its parse can come from the map
                        // cache (mapcache.rs)
                        let fresh = {
                            let m = &self.rt.fonts.map;
                            m.fm.len() <= 1 && m.tfm_tree.is_empty() && m.ps_tree.is_empty() && m.encname_tree.is_empty()
                        };
                        let key = if fresh && super::mapcache::dir().is_some() {
                            Some(super::mapcache::key(&f.data, mi.mode, self.getpdfsuppresswarningdupmap()))
                        } else {
                            None
                        };
                        crate::rt::phase::mark("map: key");
                        if let Some(p) = key.and_then(super::mapcache::load) {
                            crate::rt::phase::mark("map: cache decoded");
                            let m = &mut self.rt.fonts.map;
                            m.fm.set_lazy(p.lazy.clone());
                            m.tfm_tree.set_base(p.lazy.clone());
                            m.ps_tree.set_lazy(p.lazy.clone());
                            for e in p.encnames {
                                m.encname_tree.insert(e);
                            }
                            if let Some(mi) = m.mitem.as_mut() {
                                mi.lineno = p.lineno;
                            }
                            for w in &p.warnings {
                                self.pdftex_warn(w);
                            }
                        } else {
                            if key.is_some() {
                                self.rt.fonts.map.capture = Some(Vec::new());
                                self.rt.fonts.map.uncacheable = false;
                            }
                            self.rt.fonts.map.fm_file = Some(f);
                            while !self.rt.fonts.map.fm_file.as_ref().unwrap().feof() {
                                self.fm_scan_line();
                                if let Some(m) = self.rt.fonts.map.mitem.as_mut() {
                                    m.lineno += 1;
                                }
                            }
                            self.rt.fonts.map.fm_file = None;
                            if let Some(k) = key {
                                let warnings = self.rt.fonts.map.capture.take().unwrap_or_default();
                                if !self.rt.fonts.map.uncacheable {
                                    let lineno = self.rt.fonts.map.mitem.as_ref().map_or(0, |m| m.lineno);
                                    super::mapcache::save(k, &mut self.rt.fonts.map, lineno, &warnings);
                                }
                            }
                        }
                        crate::rt::phase::mark("map file parsed");
                        self.tex_printf("}");
                    }
                }
            }
            _ => {
                self.set_cur_file_name_none();
                self.fm_scan_line();
            }
        }
        self.rt.fonts.map.mitem.as_mut().unwrap().line = None;
        self.set_cur_file_name_none();
    }

    /// mapfile.c fmlookup
    fn fmlookup(&mut self, f: i32) -> usize {
        if !self.rt.fonts.map.trees {
            self.fm_read_info();
        }
        let mut tfm = self.tex_string(*ix(self.fontname, f));
        if let Some(z) = tfm.iter().position(|&c| c == 0) {
            tfm.truncate(z);
        }
        if let Some(fm) = self.rt.fonts.map.tfm_tree.get(&tfm) {
            self.fm_mut(fm).in_use = true;
            return fm;
        }
        DUMMY_FM
    }

    /// The fm_entry index pdffontmap[f] points to.
    pub fn font_fm(&self, f: i32) -> usize {
        fm_from_ptr(*ix(self.pdffontmap, f))
    }

    /// mapfile.c hasfmentry
    pub fn hasfmentry(&mut self, f: i32) -> bool {
        if ix(self.pdffontmap, f).is_null() {
            let fm = self.fmlookup(f);
            *ix(self.pdffontmap, f) = fm_to_ptr(fm);
        }
        self.font_fm(f) != DUMMY_FM
    }

    /// mapfile.c isscalable
    pub fn isscalable(&mut self, f: i32) -> bool {
        self.hasfmentry(f) && !self.fm(self.font_fm(f)).is_pk()
    }

    /// mapfile.c hasspacechar
    pub fn hasspacechar(&mut self, f: i32) -> bool {
        if !self.isscalable(f) {
            return false;
        }
        let fm = self.font_fm(f);
        if let Some(enc) = self.fm(fm).encname.clone() {
            if let Some(fe) = self.get_fe_entry(&enc) {
                if gname(&self.rt.fonts.enc.fe[fe].glyph_names[32]) == b"space" {
                    return true;
                }
            }
        }
        false
    }

    /// mapfile.c fm_valid_for_font_replacement
    #[allow(dead_code)]
    fn fm_valid_for_font_replacement(&mut self, fm: usize) -> bool {
        let ff = self.fm(fm).ff().to_vec();
        self.check_ff_exist(&ff, false).is_some()
    }

    /// mapfile.c lookup_fontmap: lookup by ps_name (Type1 font replacement when embedding
    /// PDF files). Returns an fm_entry index. (For the epdf/pdftoepdf port.)
    #[allow(dead_code)]
    pub fn lookup_fontmap(&mut self, ps_name: &[u8]) -> Option<usize> {
        if !self.rt.fonts.map.trees {
            self.fm_read_info();
        }
        let mut s: &[u8] = ps_name;
        if ps_name.len() > 7 {
            let mut i = 0;
            while i < 6 && (b'A'..=b'Z').contains(&ps_name[i]) {
                i += 1;
            }
            if i == 6 && ps_name[6] == b'+' {
                s = &ps_name[7..];
            }
        }
        let mut name = s.to_vec();
        let (mut slant, mut extend) = (0, 0);
        let find = |h: &[u8], pat: &[u8]| h.windows(pat.len()).position(|w| w == pat);
        let strtol = |t: &[u8]| -> (i64, usize) {
            match scan_int(t, 0, 10) {
                Some((v, e)) => (v, e),
                None => (0, 0),
            }
        };
        if let Some(a) = find(&name, b"-Slant_") {
            let b = &name[a + 7..];
            let (sl, e) = strtol(b);
            if e != 0 && e == b.len() {
                slant = sl as i32;
                name.truncate(a);
            } else if e != 0 {
                let rest = &b[e..];
                if let Some(c) = find(rest, b"-Extend_") {
                    let d = &rest[c + 8..];
                    let (ex, e2) = strtol(d);
                    if e2 != 0 && e2 == d.len() {
                        slant = sl as i32;
                        extend = ex as i32;
                        name.truncate(a);
                    }
                }
            }
        } else if let Some(a) = find(&name, b"-Extend_") {
            let b = &name[a + 8..];
            let (ex, e) = strtol(b);
            if e != 0 && e == b.len() {
                extend = ex as i32;
                name.truncate(a);
            }
        }
        let cands: Vec<usize> = self.rt.fonts.map.ps_tree.get(&name, slant, extend).into_iter().collect();
        for fm in cands {
            if self.fm_valid_for_font_replacement(fm) {
                return Some(fm);
            }
        }
        None
    }

    /// mapfile.c process_map_item
    fn process_map_item(&mut self, s: &[u8], type_: i32) {
        let mut i = 0usize;
        let mut s = s.to_vec();
        if let Some(z) = s.iter().position(|&c| c == 0) {
            s.truncate(z);
        }
        if at(&s, i) == b' ' {
            i += 1;
        }
        let mode = match at(&s, i) {
            b'+' => {
                i += 1;
                FM_DUPIGNORE
            }
            b'=' => {
                i += 1;
                FM_REPLACE
            }
            b'-' => {
                i += 1;
                FM_DELETE
            }
            _ => {
                self.rt.fonts.map.mitem.as_mut().unwrap().line = None;
                FM_DUPIGNORE
            }
        };
        if at(&s, i) == b' ' {
            i += 1;
        }
        let mut item = s[i.min(s.len())..].to_vec();
        if type_ == MAPFILE {
            if let Some(p) = item.iter().position(|&c| c == b' ') {
                item.truncate(p);
            }
        }
        if self.rt.fonts.map.mitem.as_ref().unwrap().line.is_some() {
            self.fm_read_info();
        }
        if !item.is_empty() {
            let m = self.rt.fonts.map.mitem.as_mut().unwrap();
            m.mode = mode;
            m.type_ = type_;
            m.line = Some(item);
            self.fm_read_info();
        }
    }

    /// mapfile.c pdfmapfile
    pub fn pdfmapfile(&mut self, t: i32) {
        let s = self.tokenstostring(t);
        let b = self.tex_string(s);
        self.process_map_item(&b, MAPFILE);
        let l = self.lasttokensstring;
        self.flushstr(l);
    }

    /// mapfile.c pdfmapline
    pub fn pdfmapline(&mut self, t: i32) {
        let s = self.tokenstostring(t);
        let b = self.tex_string(s);
        self.process_map_item(&b, MAPLINE);
        let l = self.lasttokensstring;
        self.flushstr(l);
    }

    /// mapfile.c pdfmaplinesp
    pub fn pdfmaplinesp(&mut self) {
        self.process_map_item(b"=pdftexspace PdfTeX-Space <pdftexspace.pfb", MAPLINE);
    }

    /// mapfile.c pdfinitmapfile
    pub fn pdfinitmapfile(&mut self, map_name: &[u8]) {
        assert!(self.rt.fonts.map.mitem.is_none());
        self.rt.fonts.map.mitem = Some(MapItem { mode: FM_DUPIGNORE, type_: MAPFILE, line: Some(map_name.to_vec()), lineno: 0 });
    }

    /// mapfile.c check_ff_exist: the full path of the font file (cached in ff_tree).
    pub fn check_ff_exist(&mut self, ff_name: &[u8], is_tt: bool) -> Option<Vec<u8>> {
        if let Some(p) = self.rt.fonts.map.ff_tree.get(ff_name) {
            return p.clone();
        }
        let fmt = if is_tt { kpse::TRUETYPE_FORMAT } else { kpse::TYPE1_FORMAT };
        let path = self.rt.kpse().find_file(ff_name, fmt, false);
        self.rt.fonts.map.ff_tree.insert(ff_name.to_vec(), path.clone());
        path
    }

    /// mapfile.c fm_free
    pub fn fm_free(&mut self) {
        let m = &mut self.rt.fonts.map;
        m.trees = false;
        m.tfm_tree.clear();
        m.ps_tree.clear();
        m.ff_tree.clear();
    }
}
