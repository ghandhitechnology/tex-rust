//! Format (.fmt) dumping: texmfmp.h dumpthings/undumpthings and texmfmp.c do_dump/do_undump.
//! Items are written big-endian (each item's bytes reversed on this little-endian
//! machine) through a gzip stream opened with level 1 (texmfmp.h wopenout).

use super::*;
use crate::gen::globals::Globals;

pub trait FmtWriter {
    fn write(&mut self, bytes: &[u8]);
    fn finish(&mut self);
}

pub trait FmtReader {
    fn read(&mut self, buf: &mut [u8]) -> usize;
    /// The whole remaining input, if it is in memory: (buffer, current position).
    fn in_memory(&self) -> Option<(*const u8, usize, usize)> {
        None
    }
    /// A file handle whose contents are the in-memory buffer (byte 0 = buffer[0]).
    fn file_handle(&self) -> isize {
        -1
    }
}

/// gzdopen(fd, "wb") + gzsetparams(f, 1, Z_DEFAULT_STRATEGY): one gzwrite per do_dump call.
struct GzFmtWriter {
    gz: Option<texrs_zlib::gz::GzWriter<std::io::BufWriter<std::fs::File>>>,
}

impl FmtWriter for GzFmtWriter {
    fn write(&mut self, bytes: &[u8]) {
        let gz = self.gz.as_mut().expect("format file closed");
        if gz.gzwrite(bytes) != bytes.len() as i32 {
            texmfmp::fatal("could not write format file");
        }
    }
    fn finish(&mut self) {
        if let Some(gz) = self.gz.take() {
            let _ = gz.gzclose();
        }
    }
}

/// gzread semantics: gzip data is inflated, anything else is read transparently.
struct GzFmtReader {
    gz: texrs_zlib::gz::GzReader<std::io::Cursor<Vec<u8>>>,
}

impl FmtReader for GzFmtReader {
    fn read(&mut self, buf: &mut [u8]) -> usize {
        self.gz.gzread(buf).max(0) as usize
    }
}

/// Element types that can be (un)dumped and range-checked.
pub trait DumpItem: Copy {
    fn as_i64(self) -> i64 {
        0
    }
}
macro_rules! dump_int_item {
    ($($t:ty),*) => { $(impl DumpItem for $t { fn as_i64(self) -> i64 { self as i64 } })* };
}
dump_int_item!(u8, i8, i16, u16, i32, u32, i64);
impl DumpItem for bool {
    fn as_i64(self) -> i64 {
        self as i64
    }
}
impl DumpItem for MemoryWord {}
impl DumpItem for TwoHalves {}
impl DumpItem for FourQuarters {}
impl DumpItem for FMemoryWord {}
impl DumpItem for f64 {}
impl<T> DumpItem for Ptr<T> {}

fn swap_items(bytes: &mut [u8], size: usize) {
    // format items are big-endian; the common sizes get loops LLVM can vectorize
    match size {
        0 | 1 => {}
        2 => {
            for c in bytes.chunks_exact_mut(2) {
                c.swap(0, 1);
            }
        }
        4 => {
            for c in bytes.chunks_exact_mut(4) {
                let v = u32::from_ne_bytes([c[0], c[1], c[2], c[3]]).swap_bytes();
                c.copy_from_slice(&v.to_ne_bytes());
            }
        }
        8 => {
            for c in bytes.chunks_exact_mut(8) {
                let v = u64::from_ne_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]).swap_bytes();
                c.copy_from_slice(&v.to_ne_bytes());
            }
        }
        _ => {
            for item in bytes.chunks_exact_mut(size) {
                item.reverse();
            }
        }
    }
}

/// Direct read position in an in-memory format (cache hit or fully inflated format):
/// undump reads from it without going through the file table and a trait object. The
/// buffer belongs to the reader in the format file's slot, which outlives the cursor
/// (wclose drops the cursor first).
pub struct FmtCursor {
    base: *const u8,
    len: usize,
    pos: usize,
    /// the cache file (or -1): large ranges are read from it with ReadFile, straight into
    /// the destination array, instead of copying out of the mapped view (fewer page faults)
    file: isize,
}

const READFILE_MIN: usize = 64 * 1024;

#[allow(non_snake_case)]
#[link(name = "kernel32")]
extern "system" {
    fn ReadFile(h: isize, buf: *mut u8, n: u32, got: *mut u32, ov: *mut Overlapped) -> i32;
}

#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: isize,
}

/// Read exactly dst.len() bytes at file offset `off`; false on any failure.
fn read_at(h: isize, off: u64, dst: &mut [u8]) -> bool {
    let mut done = 0usize;
    while done < dst.len() {
        let n = (dst.len() - done).min(1 << 30) as u32;
        let at = off + done as u64;
        let mut ov = Overlapped { internal: 0, internal_high: 0, offset: at as u32, offset_high: (at >> 32) as u32, event: 0 };
        let mut got = 0u32;
        let ok = unsafe { ReadFile(h, dst.as_mut_ptr().add(done), n, &mut got, &mut ov) };
        if ok == 0 || got == 0 {
            return false;
        }
        done += got as usize;
    }
    true
}

impl Globals {
    pub fn wopenout(&mut self) -> (FileRef, bool) {
        let (f, ok) = self.open_output();
        if !ok {
            return (FileRef::NULL, false);
        }
        // Swap the plain output handle for a gzip writer on the same file.
        let h = self.rt.files.slots[f.0 as usize].take();
        let file = match h {
            Some(io::Handle::Out { w, .. }) => w.into_inner().unwrap_or_else(|_| texmfmp::fatal("format file")),
            _ => return (FileRef::NULL, false),
        };
        // texmfmp.h wopenout: gzdopen(fd, "wb") && gzsetparams(f, 1, Z_DEFAULT_STRATEGY) == Z_OK
        let mut gz = match texrs_zlib::gz::GzWriter::with_mode(std::io::BufWriter::new(file), "wb", "") {
            Some(g) => g,
            None => return (FileRef::NULL, false),
        };
        if gz.gzsetparams(1, texrs_zlib::Z_DEFAULT_STRATEGY) != 0 {
            return (FileRef::NULL, false);
        }
        self.rt.files.slots[f.0 as usize] = Some(io::Handle::FmtOut(Box::new(GzFmtWriter { gz: Some(gz) })));
        self.rt.dump_file = f;
        (f, true)
    }

    pub fn wopenin(&mut self) -> (FileRef, bool) {
        crate::rt::phase::mark("fmt open");
        // the format replaces the hash table: the id_lookup index starts over
        self.rt.csindex = Default::default();
        let (f, ok) = self.open_input(kpse::FMT_FORMAT);
        if !ok {
            return (FileRef::NULL, false);
        }
        let h = self.rt.files.slots[f.0 as usize].take();
        // open_input read only the start of the format file (io::FMT_HEAD bytes): enough
        // for the cache key; the rest is read only if the cache cannot serve it
        let head = match h {
            Some(io::Handle::In { data, .. }) => data,
            _ => return (FileRef::NULL, false),
        };
        let name = self.name_of_file();
        let os = self.os_path(&name);
        let reader: Box<dyn FmtReader> = match fmt_cache::open(&os, &head) {
            Some(r) => r,
            None => {
                let data = if head.len() < io::FMT_HEAD {
                    head
                } else {
                    match std::fs::read(&os) {
                        Ok(d) => d,
                        Err(_) => texmfmp::fatal_perror(&bytes_to_string(&name)),
                    }
                };
                match fmt_cache::fill(&os, &data) {
                    Some((raw, start)) => Box::new(RawFmtReader { data: raw, pos: start }),
                    None => {
                        let gz = match texrs_zlib::gz::GzReader::with_mode(std::io::Cursor::new(data), "rb", "") {
                            Some(g) => g,
                            None => return (FileRef::NULL, false),
                        };
                        Box::new(GzFmtReader { gz })
                    }
                }
            }
        };
        let fh = reader.file_handle();
        self.rt.fmt_cursor = reader.in_memory().map(|(base, len, pos)| FmtCursor { base, len, pos, file: fh });
        self.rt.files.slots[f.0 as usize] = Some(io::Handle::FmtIn(reader));
        self.rt.dump_file = f;
        (f, true)
    }

    pub fn wclose(&mut self, f: FileRef) {
        crate::rt::phase::mark("fmt close");
        if f == self.rt.dump_file {
            self.rt.fmt_cursor = None;
            if let Some(Some(io::Handle::FmtIn(_))) = self.rt.files.slots.get(f.0 as usize) {
                self.scan_outer_meanings();
            }
        }
        self.rt.files.close(f);
    }

    fn dump_bytes(&mut self, bytes: &[u8], nitems: i32, item_size: usize) {
        self.dump_bytes_l(bytes, nitems, item_size, "");
    }

    fn dump_bytes_l(&mut self, bytes: &[u8], nitems: i32, item_size: usize, label: &str) {
        if std::env::var_os("TEXRS_DUMPTRACE").is_some() {
            eprintln!("DUMP off={} n={} size={} {}", self.rt.dump_offset, nitems, item_size, label);
        }
        self.rt.dump_offset += bytes.len() as u64;
        let f = self.fmtfile;
        match self.rt.files.slots.get_mut(f.0 as usize) {
            Some(Some(io::Handle::FmtOut(w))) => w.write(bytes),
            _ => {
                let name = self.name_of_file_str();
                eprint!("! Could not write {} {}-byte item(s) to {}.\r\n", nitems, item_size, name);
                self.x_uexit(1);
            }
        }
    }

    /// dumpthings with the Pascal array name, for TEXRS_DUMPTRACE format maps.
    pub fn dump_things_l<T: DumpItem>(&mut self, p: *mut T, n: i32, label: &str) {
        let size = std::mem::size_of::<T>();
        let len = size * n.max(0) as usize;
        let src = unsafe { std::slice::from_raw_parts(p as *const u8, len) };
        let mut buf = src.to_vec();
        swap_items(&mut buf, size);
        self.dump_bytes_l(&buf, n, size, label);
    }

    pub fn dump_things<T: DumpItem>(&mut self, p: *mut T, n: i32) {
        let size = std::mem::size_of::<T>();
        let len = size * n.max(0) as usize;
        let src = unsafe { std::slice::from_raw_parts(p as *const u8, len) };
        let mut buf = src.to_vec();
        swap_items(&mut buf, size);
        self.dump_bytes(&buf, n, size);
    }

    #[inline]
    pub fn undump_things<T: DumpItem>(&mut self, p: *mut T, n: i32) {
        let size = std::mem::size_of::<T>();
        let len = size * n.max(0) as usize;
        let dst = unsafe { std::slice::from_raw_parts_mut(p as *mut u8, len) };
        if let Some(c) = self.rt.fmt_cursor.as_mut() {
            let got = len.min(c.len - c.pos);
            if !(got >= READFILE_MIN && c.file != -1 && read_at(c.file, c.pos as u64, &mut dst[..got])) {
                unsafe { std::ptr::copy_nonoverlapping(c.base.add(c.pos), dst.as_mut_ptr(), got) };
            }
            c.pos += got;
            if got != len {
                let name = self.name_of_file_str();
                texmfmp::fatal(&format!("Could not undump {} {}-byte item(s) from {}", n, size, name));
            }
            swap_items(dst, size);
            return;
        }
        let f = self.fmtfile;
        let got = match self.rt.files.slots.get_mut(f.0 as usize) {
            Some(Some(io::Handle::FmtIn(r))) => {
                let mut got = 0;
                while got < len {
                    let k = r.read(&mut dst[got..]);
                    if k == 0 {
                        break;
                    }
                    got += k;
                }
                got
            }
            _ => 0,
        };
        if got != len {
            let name = self.name_of_file_str();
            texmfmp::fatal(&format!("Could not undump {} {}-byte item(s) from {}", n, size, name));
        }
        swap_items(dst, size);
    }

    pub fn dump_int(&mut self, v: i32) {
        let mut x = v;
        self.dump_things(&raw mut x, 1);
    }

    pub fn undump_int(&mut self) -> i32 {
        let mut x: i32 = 0;
        self.undump_things(&raw mut x, 1);
        x
    }

    pub fn undump_checked_things<T: DumpItem>(&mut self, low: i32, high: i32, p: *mut T, n: i32) {
        self.undump_things(p, n);
        for i in 0..n.max(0) as usize {
            let v = unsafe { *p.add(i) }.as_i64();
            if v < low as i64 || v > high as i64 {
                texmfmp::fatal(&format!(
                    "Item {} (={}) of .fmt array at {:x} <{} or >{}",
                    i, v, p as usize, low, high
                ));
            }
        }
    }

    pub fn undump_upper_check_things<T: DumpItem>(&mut self, high: i32, p: *mut T, n: i32) {
        self.undump_things(p, n);
        for i in 0..n.max(0) as usize {
            let v = unsafe { *p.add(i) }.as_i64();
            if v > high as i64 {
                texmfmp::fatal(&format!("Item {} (={}) of .fmt array at {:x} >{}", i, v, p as usize, high));
            }
        }
    }
}

/// An already-inflated format (from the format cache).
struct RawFmtReader {
    data: Vec<u8>,
    pos: usize,
}

impl FmtReader for RawFmtReader {
    fn in_memory(&self) -> Option<(*const u8, usize, usize)> {
        Some((self.data.as_ptr(), self.data.len(), self.pos))
    }
    fn read(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        n
    }
}

/// Format cache: an inflated copy of each format file that was loaded, so later runs
/// skip gzip decompression (about 20 ms for pdflatex.fmt). Purely a speed-up: the bytes
/// TeX undumps are exactly what gzread would return. There is one entry per format path
/// (rebuilding a format replaces its entry); an entry is used only while the format's
/// size, modification time and first 4 KB still match the stamp stored in it. At most
/// MAX_ENTRIES entries are kept, least recently used first out. Disable with
/// TEXRS_FMT_CACHE=0; the location is %LOCALAPPDATA%\texrs\fmt-cache (or
/// TEXRS_FMT_CACHE_DIR).
mod fmt_cache {
    use std::path::{Path, PathBuf};

    const MAGIC: &[u8; 8] = b"TEXRSFC2";
    const HEADER: usize = 16; // MAGIC, then the u64 stamp
    const MAX_ENTRIES: usize = 24;

    fn dir() -> Option<PathBuf> {
        if std::env::var("TEXRS_FMT_CACHE").as_deref() == Ok("0") {
            return None;
        }
        if let Ok(d) = std::env::var("TEXRS_FMT_CACHE_DIR") {
            return Some(PathBuf::from(d));
        }
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("texrs").join("fmt-cache"))
    }

    fn fnv(data: &[u8], mut h: u64) -> u64 {
        for &b in data {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    /// (name of the cache entry, stamp of the format it must match). `head` is the start
    /// of the format file (up to 4 KB).
    fn key(path: &Path, head: &[u8]) -> Option<(u64, u64)> {
        let abs = std::path::absolute(path).ok()?;
        let md = std::fs::metadata(&abs).ok()?;
        let mtime = md.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
        let name = fnv(abs.to_string_lossy().as_bytes(), 0xcbf29ce484222325);
        let mut stamp = fnv(&md.len().to_le_bytes(), 0xcbf29ce484222325);
        stamp = fnv(&mtime.to_le_bytes(), stamp);
        stamp = fnv(&head[..head.len().min(4096)], stamp);
        Some((name, stamp))
    }

    /// A reader over the cached inflated format, if the cache has a valid entry. The
    /// entry is memory-mapped, so nothing is copied before TeX undumps it.
    pub fn open(path: &Path, head: &[u8]) -> Option<Box<dyn super::FmtReader>> {
        let dir = dir()?;
        let (name, stamp) = key(path, head)?;
        let map = Mapped::open(&dir.join(format!("{:016x}.fmtraw", name)))?;
        let b = map.bytes();
        if b.len() >= HEADER && &b[..8] == MAGIC && b[8..16] == stamp.to_le_bytes() {
            Some(Box::new(MappedFmtReader { map, pos: HEADER }))
        } else {
            None
        }
    }

    /// Inflate the whole format `gz` and cache it: (buffer, offset of the format's first
    /// byte). None: caching disabled or the gzip data is not fully readable (the caller
    /// then streams it through gzread so errors surface exactly where C reports them).
    pub fn fill(path: &Path, gz: &[u8]) -> Option<(Vec<u8>, usize)> {
        let dir = dir()?;
        let (name, stamp) = key(path, gz)?;
        let mut r = texrs_zlib::gz::GzReader::with_mode(std::io::Cursor::new(gz.to_vec()), "rb", "")?;
        let mut out = Vec::with_capacity(HEADER + gz.len() * 4);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&stamp.to_le_bytes());
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = r.gzread(&mut buf);
            if n < 0 {
                return None;
            }
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
        }
        store(&dir, &dir.join(format!("{:016x}.fmtraw", name)), name, &out);
        Some((out, HEADER))
    }

    struct MappedFmtReader {
        map: Mapped,
        pos: usize,
    }

    impl super::FmtReader for MappedFmtReader {
        fn in_memory(&self) -> Option<(*const u8, usize, usize)> {
            let d = self.map.bytes();
            Some((d.as_ptr(), d.len(), self.pos))
        }
        fn file_handle(&self) -> isize {
            self.map.file
        }
        fn read(&mut self, buf: &mut [u8]) -> usize {
            let d = self.map.bytes();
            let n = buf.len().min(d.len() - self.pos);
            buf[..n].copy_from_slice(&d[self.pos..self.pos + n]);
            self.pos += n;
            n
        }
    }

    /// A read-only view of a whole file (the cache entry). Opening it also refreshes the
    /// entry's modification time (the LRU order of `prune`) when that is over an hour old.
    struct Mapped {
        view: *const u8,
        len: usize,
        /// the file, kept open for ReadFile (FmtCursor)
        file: isize,
    }

    #[allow(non_snake_case)]
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut u8, disp: u32, flags: u32, tmpl: isize) -> isize;
        fn GetFileSizeEx(h: isize, size: *mut i64) -> i32;
        fn CreateFileMappingW(h: isize, sa: *mut u8, protect: u32, hi: u32, lo: u32, name: *const u16) -> isize;
        fn MapViewOfFile(h: isize, access: u32, hi: u32, lo: u32, n: usize) -> *const u8;
        fn UnmapViewOfFile(p: *const u8) -> i32;
        fn CloseHandle(h: isize) -> i32;
        fn GetFileTime(h: isize, c: *mut u64, a: *mut u64, w: *mut u64) -> i32;
        fn SetFileTime(h: isize, c: *const u64, a: *const u64, w: *const u64) -> i32;
        fn GetSystemTimeAsFileTime(t: *mut u64);
    }

    impl Mapped {
        fn open(path: &Path) -> Option<Mapped> {
            use std::os::windows::ffi::OsStrExt;
            let w: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            const GENERIC_READ: u32 = 0x8000_0000;
            const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
            const SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
            const OPEN_EXISTING: u32 = 3;
            unsafe {
                let mut h = CreateFileW(w.as_ptr(), GENERIC_READ | FILE_WRITE_ATTRIBUTES, SHARE_ALL, std::ptr::null_mut(), OPEN_EXISTING, 0, 0);
                let mut touch = true;
                if h == -1 {
                    h = CreateFileW(w.as_ptr(), GENERIC_READ, SHARE_ALL, std::ptr::null_mut(), OPEN_EXISTING, 0, 0);
                    touch = false;
                    if h == -1 {
                        return None;
                    }
                }
                let mut size = 0i64;
                let mut out = None;
                if GetFileSizeEx(h, &mut size) != 0 && size > 0 {
                    if touch {
                        let mut wt = 0u64;
                        let mut now = 0u64;
                        GetSystemTimeAsFileTime(&mut now);
                        if GetFileTime(h, std::ptr::null_mut(), std::ptr::null_mut(), &mut wt) != 0
                            && now.saturating_sub(wt) > 3600 * 10_000_000
                        {
                            SetFileTime(h, std::ptr::null(), std::ptr::null(), &now);
                        }
                    }
                    let m = CreateFileMappingW(h, std::ptr::null_mut(), 0x02 /* PAGE_READONLY */, 0, 0, std::ptr::null());
                    if m != 0 {
                        let v = MapViewOfFile(m, 0x4 /* FILE_MAP_READ */, 0, 0, 0);
                        CloseHandle(m);
                        if !v.is_null() {
                            out = Some(Mapped { view: v, len: size as usize, file: h });
                        }
                    }
                }
                if out.is_none() {
                    CloseHandle(h);
                }
                out
            }
        }
        fn bytes(&self) -> &[u8] {
            unsafe { std::slice::from_raw_parts(self.view, self.len) }
        }
    }

    impl Drop for Mapped {
        fn drop(&mut self) {
            unsafe {
                UnmapViewOfFile(self.view);
                CloseHandle(self.file);
            }
        }
    }

    /// Write an entry atomically (temporary file, then rename over the old entry), then
    /// prune the cache. Failures only cost the speed-up.
    fn store(dir: &Path, file: &Path, name: u64, data: &[u8]) {
        let _ = std::fs::create_dir_all(dir);
        let tmp = dir.join(format!("{:016x}.{}.tmp", name, std::process::id()));
        if std::fs::write(&tmp, data).is_ok() && std::fs::rename(&tmp, file).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        prune(dir);
    }

    /// Keep the MAX_ENTRIES most recently used entries. Also removes entries of the
    /// earlier cache layout (*.raw) and temporary files abandoned over an hour ago.
    fn prune(dir: &Path) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let now = std::time::SystemTime::now();
        let mut entries = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            let Ok(md) = e.metadata() else { continue };
            let t = md.modified().unwrap_or(std::time::UNIX_EPOCH);
            match p.extension().and_then(|x| x.to_str()) {
                Some("fmtraw") => entries.push((t, p)),
                Some("raw") => {
                    let _ = std::fs::remove_file(&p);
                }
                Some("tmp") if now.duration_since(t).map_or(false, |d| d.as_secs() > 3600) => {
                    let _ = std::fs::remove_file(&p);
                }
                _ => {}
            }
        }
        if entries.len() > MAX_ENTRIES {
            entries.sort();
            for (_, p) in &entries[..entries.len() - MAX_ENTRIES] {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}
