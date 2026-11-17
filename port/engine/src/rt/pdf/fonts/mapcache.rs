//! Cache of parsed font map files (an engine speed-up, not part of pdfTeX).
//!
//! pdfTeX parses the whole of pdftex.map (thousands of lines) in every run that writes a
//! PDF with fonts, to use a handful of entries. The parse result depends only on the
//! file's bytes, the map mode and \pdfsuppresswarningdupmap, provided the map tables are
//! still empty when it starts (the first map file of the run). So the resulting tables,
//! and the warnings the parse printed, are cached under a hash of exactly those inputs;
//! a later run with the same inputs restores the tables and prints the same warnings at
//! the same point (pdftex_warn with the same messages), so the output is unchanged.
//!
//! Restoring is lazy: the cache file holds every entry's encoding with an offset table,
//! and the tfm-name index sorted by name, so a run decodes only the entries it looks at
//! (`LazyMap`, used by mapfile.rs `FmVec`, `TfmTree` and `PsTree`).
//!
//! Parses that read subfont definition files (.sfd, which involves further file lookups)
//! are not cached. Disable with TEXRS_MAP_CACHE=0; location %LOCALAPPDATA%\texrs\map-cache
//! (or TEXRS_MAP_CACHE_DIR).

use std::path::PathBuf;
use std::rc::Rc;

use super::mapfile::MapState;
use super::FmEntry;

const MAGIC: &[u8; 8] = b"TEXRSMC2";
const MAX_FILES: usize = 16;

pub fn dir() -> Option<PathBuf> {
    if std::env::var("TEXRS_MAP_CACHE").as_deref() == Ok("0") {
        return None;
    }
    if let Ok(d) = std::env::var("TEXRS_MAP_CACHE_DIR") {
        return Some(PathBuf::from(d));
    }
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("texrs").join("map-cache"))
}

/// 128-bit key of the parse inputs.
pub fn key(data: &[u8], mode: i32, suppress_dup: i32) -> (u64, u64) {
    let mut a: u64 = 0x9e37_79b9_7f4a_7c15 ^ data.len() as u64;
    let mut b: u64 = 0xc2b2_ae3d_27d4_eb4f ^ ((mode as u64) << 32 | suppress_dup as u32 as u64);
    let mut chunks = data.chunks_exact(8);
    for c in &mut chunks {
        let w = u64::from_le_bytes(c.try_into().unwrap());
        a = (a.rotate_left(5) ^ w).wrapping_mul(0x517c_c1b7_2722_0a95);
        b = (b ^ w).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(29);
    }
    for &x in chunks.remainder() {
        a = (a.rotate_left(5) ^ x as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
        b = (b ^ x as u64).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(29);
    }
    (a, b)
}

fn file_for(dir: &std::path::Path, k: (u64, u64)) -> PathBuf {
    dir.join(format!("{:016x}{:016x}.fmc", k.0, k.1))
}

struct W(Vec<u8>);

impl W {
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.u32(v as u32);
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
    }
    fn opt(&mut self, b: &Option<Vec<u8>>) {
        match b {
            None => self.u32(u32::MAX),
            Some(v) => self.bytes(v),
        }
    }
}

struct R<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> R<'a> {
    fn u32(&mut self) -> Option<u32> {
        let v = u32::from_le_bytes(self.b.get(self.p..self.p + 4)?.try_into().ok()?);
        self.p += 4;
        Some(v)
    }
    fn i32(&mut self) -> Option<i32> {
        Some(self.u32()? as i32)
    }
    fn slice(&mut self, n: usize) -> Option<&'a [u8]> {
        let v = self.b.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(v)
    }
    fn bytes(&mut self) -> Option<Vec<u8>> {
        let n = self.u32()? as usize;
        Some(self.slice(n)?.to_vec())
    }
    fn opt(&mut self) -> Option<Option<Vec<u8>>> {
        let n = self.u32()?;
        if n == u32::MAX {
            return Some(None);
        }
        self.p -= 4;
        Some(Some(self.bytes()?))
    }
}

fn encode_entry(w: &mut W, e: &FmEntry) {
    w.opt(&e.tfm_name);
    w.opt(&e.ps_name);
    w.i32(e.fd_flags);
    w.i32(e.slant);
    w.i32(e.extend);
    w.opt(&e.encname);
    w.opt(&e.ff_name);
    w.u32(e.type_ as u32);
    w.i32(e.pid as i32);
    w.i32(e.eid as i32);
    w.u32(e.links as u32);
    w.u32(e.in_use as u32);
}

fn decode_entry(r: &mut R) -> Option<FmEntry> {
    Some(FmEntry {
        tfm_name: r.opt()?,
        ps_name: r.opt()?,
        fd_flags: r.i32()?,
        slant: r.i32()?,
        extend: r.i32()?,
        encname: r.opt()?,
        ff_name: r.opt()?,
        type_: r.u32()? as u16,
        pid: r.i32()? as i16,
        eid: r.i32()? as i16,
        subfont: None,
        links: r.u32()? as u16,
        in_use: r.u32()? != 0,
    })
}

/// A restored map: the cache file's bytes and where its sections are.
pub struct LazyMap {
    data: Vec<u8>,
    /// number of entries (fm[0..n))
    pub n: usize,
    /// u32 offsets of the entries, relative to `entries`
    offs: usize,
    entries: usize,
    /// sorted (name offset, name length, index) triples, names relative to `names`
    tfm: usize,
    ntfm: usize,
    names: usize,
    /// ps_tree entries: (name, slant, extend, index) in insertion order
    ps: usize,
    nps: usize,
}

impl LazyMap {
    fn u32_at(&self, p: usize) -> u32 {
        u32::from_le_bytes(self.data[p..p + 4].try_into().unwrap())
    }

    /// Entry i (0 <= i < n), decoded.
    pub fn entry(&self, i: usize) -> FmEntry {
        let off = self.u32_at(self.offs + 4 * i) as usize;
        let mut r = R { b: &self.data, p: self.entries + off };
        decode_entry(&mut r).expect("map cache entry")
    }

    fn tfm_name(&self, k: usize) -> &[u8] {
        let t = self.tfm + 12 * k;
        let o = self.u32_at(t) as usize;
        let l = self.u32_at(t + 4) as usize;
        &self.data[self.names + o..self.names + o + l]
    }

    /// The index of tfm name `name` in the restored tfm_tree.
    pub fn tfm_get(&self, name: &[u8]) -> Option<usize> {
        let (mut lo, mut hi) = (0usize, self.ntfm);
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.tfm_name(mid).cmp(name) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(self.u32_at(self.tfm + 12 * mid + 8) as usize),
            }
        }
        None
    }

    /// All (name, index) pairs of the restored tfm_tree.
    pub fn tfm_all(&self) -> Vec<(Vec<u8>, usize)> {
        (0..self.ntfm).map(|k| (self.tfm_name(k).to_vec(), self.u32_at(self.tfm + 12 * k + 8) as usize)).collect()
    }

    pub fn tfm_count(&self) -> usize {
        self.ntfm
    }

    /// The restored ps_tree entries, in their original insertion order.
    pub fn ps_entries(&self) -> Vec<(Vec<u8>, i32, i32, usize)> {
        let mut r = R { b: &self.data, p: self.ps };
        let mut v = Vec::with_capacity(self.nps);
        for _ in 0..self.nps {
            let name = r.bytes().expect("map cache");
            let s = r.i32().expect("map cache");
            let e = r.i32().expect("map cache");
            v.push((name, s, e, r.u32().expect("map cache") as usize));
        }
        v
    }
}

/// The cached result: lazy tables, the encoding names, the final line number and the
/// warnings to replay.
pub struct Parsed {
    pub lazy: Rc<LazyMap>,
    pub encnames: Vec<Vec<u8>>,
    pub lineno: i32,
    pub warnings: Vec<String>,
}

pub fn save(k: (u64, u64), m: &mut MapState, lineno: i32, warnings: &[String]) {
    let Some(dir) = dir() else { return };
    let n = m.fm.len();
    let mut entries = W(Vec::new());
    let mut offs = W(Vec::new());
    for i in 0..n {
        let e = &m.fm[i];
        if e.subfont.is_some() {
            return; // never cached (see module comment)
        }
        offs.u32(entries.0.len() as u32);
        encode_entry(&mut entries, e);
    }
    let mut tfm = m.tfm_tree.entries();
    tfm.sort();
    let mut names = Vec::new();
    let mut table = W(Vec::new());
    for (name, i) in &tfm {
        table.u32(names.len() as u32);
        table.u32(name.len() as u32);
        table.u32(*i as u32);
        names.extend_from_slice(name);
    }
    let ps = m.ps_tree.entries();
    let mut w = W(MAGIC.to_vec());
    w.0.extend_from_slice(&k.0.to_le_bytes());
    w.0.extend_from_slice(&k.1.to_le_bytes());
    w.i32(lineno);
    w.u32(warnings.len() as u32);
    for s in warnings {
        w.bytes(s.as_bytes());
    }
    w.u32(m.encname_tree.len() as u32);
    for e in &m.encname_tree {
        w.bytes(e);
    }
    w.u32(n as u32);
    w.bytes(&offs.0);
    w.bytes(&entries.0);
    w.u32(tfm.len() as u32);
    w.bytes(&table.0);
    w.bytes(&names);
    w.u32(ps.len() as u32);
    for (name, slant, extend, i) in &ps {
        w.bytes(name);
        w.i32(*slant);
        w.i32(*extend);
        w.u32(*i as u32);
    }
    let _ = std::fs::create_dir_all(&dir);
    let file = file_for(&dir, k);
    let tmp = dir.join(format!("{:016x}.{}.tmp", k.0, std::process::id()));
    if std::fs::write(&tmp, &w.0).is_ok() && std::fs::rename(&tmp, &file).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut v: Vec<(std::time::SystemTime, PathBuf)> = rd
            .flatten()
            .filter(|e| e.path().extension().map_or(false, |x| x == "fmc"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        if v.len() > MAX_FILES {
            v.sort();
            for (_, p) in &v[..v.len() - MAX_FILES] {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

pub fn load(k: (u64, u64)) -> Option<Parsed> {
    let dir = dir()?;
    let data = std::fs::read(file_for(&dir, k)).ok()?;
    if data.len() < 24 || &data[..8] != MAGIC || data[8..16] != k.0.to_le_bytes() || data[16..24] != k.1.to_le_bytes() {
        return None;
    }
    let mut r = R { b: &data, p: 24 };
    let lineno = r.i32()?;
    let nw = r.u32()? as usize;
    let mut warnings = Vec::with_capacity(nw.min(1 << 16));
    for _ in 0..nw {
        warnings.push(String::from_utf8(r.bytes()?).ok()?);
    }
    let ne = r.u32()? as usize;
    let mut encnames = Vec::with_capacity(ne.min(1 << 16));
    for _ in 0..ne {
        encnames.push(r.bytes()?);
    }
    let n = r.u32()? as usize;
    let lo = r.u32()? as usize;
    let offs = r.p;
    if lo != 4 * n {
        return None;
    }
    r.slice(lo)?;
    let le = r.u32()? as usize;
    let entries = r.p;
    r.slice(le)?;
    let ntfm = r.u32()? as usize;
    let lt = r.u32()? as usize;
    let tfm = r.p;
    if lt != 12 * ntfm {
        return None;
    }
    r.slice(lt)?;
    let ln = r.u32()? as usize;
    let names = r.p;
    r.slice(ln)?;
    let nps = r.u32()? as usize;
    let ps = r.p;
    // validate the ps section (and that the file ends there)
    for _ in 0..nps {
        let l = r.u32()? as usize;
        r.slice(l + 12)?;
    }
    if r.p != data.len() {
        return None;
    }
    let lazy = Rc::new(LazyMap { data, n, offs, entries, tfm, ntfm, names, ps, nps });
    Some(Parsed { lazy, encnames, lineno, warnings })
}
