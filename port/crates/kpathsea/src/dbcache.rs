//! Persistent cache of the parsed ls-R databases.
//!
//! Building the in-memory index from ls-R (db.c `kpathsea_init_db`) is most of
//! kpathsea's start-up time. The index is plain data (the lower-cased file text,
//! directory names, entries and the open-addressing table), so it is written once to
//! a cache file and memory-mapped by later runs. An entry is used only while every ls-R
//! file it was built from still has the same path, size and modification time; it is
//! never used with KPSE_DEBUG_HASH on (the build prints traces), nor written when a
//! database produced warnings (those have to be printed again by the next run).
//! Disable with TEXRS_KPSE_CACHE=0; location %LOCALAPPDATA%\texrs\kpse-cache (or
//! TEXRS_KPSE_CACHE_DIR).

use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"KPSEDB02";
const MAX_FILES: usize = 8;

pub(crate) fn dir() -> Option<PathBuf> {
    if std::env::var("TEXRS_KPSE_CACHE").as_deref() == Ok("0") {
        return None;
    }
    if let Ok(d) = std::env::var("TEXRS_KPSE_CACHE_DIR") {
        return Some(PathBuf::from(d));
    }
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("texrs").join("kpse-cache"))
}

fn fnv(data: &[u8], mut h: u64) -> u64 {
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Identity of a set of database files: their paths, sizes, modification times and
/// first and last 4 KB (so that a file rewritten within the timestamp resolution with
/// the same size is still noticed).
pub(crate) fn stamp(files: &[PathBuf], cp932: u32) -> Option<u64> {
    use std::io::{Read, Seek, SeekFrom};
    let mut h = fnv(MAGIC, 0xcbf29ce484222325);
    h = fnv(&cp932.to_le_bytes(), h);
    let mut buf = vec![0u8; 4096];
    for f in files {
        let mut file = std::fs::File::open(f).ok()?;
        let md = file.metadata().ok()?;
        let t = md.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
        h = fnv(f.as_os_str().as_encoded_bytes(), h);
        h = fnv(&md.len().to_le_bytes(), h);
        h = fnv(&t.to_le_bytes(), h);
        let n = file.read(&mut buf).ok()?;
        h = fnv(&buf[..n], h);
        if md.len() > 8192 {
            file.seek(SeekFrom::End(-4096)).ok()?;
            let n = file.read(&mut buf).ok()?;
            h = fnv(&buf[..n], h);
        }
    }
    Some(h)
}

pub(crate) fn file_for(dir: &Path, stamp: u64) -> PathBuf {
    dir.join(format!("{:016x}.kpsedb", stamp))
}

/// Section writer: 8-byte aligned blobs after a header.
pub(crate) struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new(stamp: u64) -> Writer {
        let mut buf = MAGIC.to_vec();
        buf.extend_from_slice(&stamp.to_le_bytes());
        Writer { buf }
    }
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    /// A length-prefixed blob, padded to 8 bytes.
    pub fn blob(&mut self, b: &[u8]) {
        self.u64(b.len() as u64);
        self.buf.extend_from_slice(b);
        while self.buf.len() % 8 != 0 {
            self.buf.push(0);
        }
    }
}

/// Section reader over a cache file (memory-mapped where possible).
pub(crate) struct Reader<'a> {
    pub b: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8], stamp: u64) -> Option<Reader<'a>> {
        if b.len() < 16 || &b[..8] != MAGIC || b[8..16] != stamp.to_le_bytes() {
            return None;
        }
        Some(Reader { b, pos: 16 })
    }
    pub fn u64(&mut self) -> Option<u64> {
        let v = u64::from_le_bytes(self.b.get(self.pos..self.pos + 8)?.try_into().ok()?);
        self.pos += 8;
        Some(v)
    }
    /// (offset, length) of the next blob within the file.
    pub fn blob(&mut self) -> Option<(usize, usize)> {
        let n = self.u64()? as usize;
        let off = self.pos;
        if off.checked_add(n)? > self.b.len() {
            return None;
        }
        self.pos = (off + n + 7) & !7;
        Some((off, n))
    }
}

/// Write the cache file atomically and keep at most MAX_FILES of them.
pub(crate) fn save(dir: &Path, stamp: u64, data: &[u8]) {
    let _ = std::fs::create_dir_all(dir);
    let file = file_for(dir, stamp);
    let tmp = dir.join(format!("{:016x}.{}.tmp", stamp, std::process::id()));
    if std::fs::write(&tmp, data).is_ok() && std::fs::rename(&tmp, &file).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut v: Vec<(std::time::SystemTime, PathBuf)> = rd
            .flatten()
            .filter(|e| e.path().extension().map_or(false, |x| x == "kpsedb"))
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

/// A read-only view of a whole file.
pub(crate) struct Map {
    ptr: *const u8,
    len: usize,
    #[cfg(not(windows))]
    owned: Vec<u8>,
}

impl Map {
    pub fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    #[cfg(not(windows))]
    pub fn open(path: &Path) -> Option<Map> {
        let owned = std::fs::read(path).ok()?;
        Some(Map { ptr: owned.as_ptr(), len: owned.len(), owned })
    }

    #[cfg(windows)]
    pub fn open(path: &Path) -> Option<Map> {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut u8, disp: u32, flags: u32, t: isize) -> isize;
            fn GetFileSizeEx(h: isize, size: *mut i64) -> i32;
            fn CreateFileMappingW(h: isize, sa: *mut u8, protect: u32, hi: u32, lo: u32, name: *const u16) -> isize;
            fn MapViewOfFile(h: isize, access: u32, hi: u32, lo: u32, n: usize) -> *const u8;
            fn CloseHandle(h: isize) -> i32;
        }
        let w: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let h = CreateFileW(w.as_ptr(), 0x8000_0000, 0x7, std::ptr::null_mut(), 3, 0, 0);
            if h == -1 {
                return None;
            }
            let mut size = 0i64;
            let mut out = None;
            if GetFileSizeEx(h, &mut size) != 0 && size > 0 {
                let m = CreateFileMappingW(h, std::ptr::null_mut(), 0x02, 0, 0, std::ptr::null());
                if m != 0 {
                    let v = MapViewOfFile(m, 0x4, 0, 0, 0);
                    CloseHandle(m);
                    if !v.is_null() {
                        out = Some(Map { ptr: v, len: size as usize });
                    }
                }
            }
            CloseHandle(h);
            out
        }
    }
}

#[cfg(windows)]
impl Drop for Map {
    fn drop(&mut self) {
        #[link(name = "kernel32")]
        extern "system" {
            fn UnmapViewOfFile(p: *const u8) -> i32;
        }
        unsafe {
            UnmapViewOfFile(self.ptr);
        }
    }
}
