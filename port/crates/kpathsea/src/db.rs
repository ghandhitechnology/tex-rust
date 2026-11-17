//! db.c: the ls-R filename databases and the `aliases` files.
//!
//! The C code reads every ls-R line into a 64007-bucket chained hash
//! table.  We parse the (lower-cased) file contents in one pass into a
//! compact open-addressing index that references the file buffer instead
//! of allocating one string per entry; look-ups return the directories in
//! the same order as C's `hash_lookup`.  C's hashing quirk (entries are
//! hashed with signed `char`s, look-ups with unsigned ones, so names with
//! bytes >= 0x80 are never found) is reproduced in [`Db::lookup`].

use std::collections::HashMap;

use crate::debug::{KPSE_DEBUG_HASH, KPSE_DEBUG_SEARCH};
use crate::hash::{c_hash, c_hash_normalized, hash_summary_line, transform, CHash};
use crate::sys;
use crate::util::*;
use crate::{FileFormat, Kpse};

const DB_HASH_SIZE: u32 = 64007;
const ALIAS_HASH_SIZE: u32 = 1009;
const DB_NAMES: [&[u8]; 2] = [b"ls-r", b"ls-R"];
const ALIAS_NAME: &[u8] = b"aliases";

#[derive(Clone, Copy)]
#[repr(C)]
struct Entry {
    buf: u32,
    off: u32,
    len: u32,
    dir: u32,
    hash: u32,
}

/// All ls-R databases (`kpse->db`).
pub(crate) struct Db {
    bufs: Vec<Vec<u8>>,
    /// All directory names, concatenated; `dirs` holds (offset, length).
    dir_buf: Vec<u8>,
    dirs: Vec<(u32, u32)>,
    entries: Vec<Entry>,
    table: Vec<u32>,
    mask: usize,
    /// Entries added by `kpathsea_db_insert` (after mktex*):
    /// lower-cased key -> directories, in insertion order.
    extra: HashMap<Vec<u8>, Vec<Vec<u8>>>,
    /// Bucket occupancy of the C table, only tracked for `-debug=2`.
    c_buckets: Option<Vec<u32>>,
    /// The index as loaded from the ls-R cache (dbcache.rs) instead of the fields above
    mapped: Option<Mapped>,
}

/// Sections of a memory-mapped cache file: (byte offset, byte length).
struct Mapped {
    map: crate::dbcache::Map,
    bufs: Vec<(usize, usize)>,
    dir_buf: (usize, usize),
    dirs: (usize, usize),
    entries: (usize, usize),
    table: (usize, usize),
}

unsafe fn view<T>(b: &[u8], (off, len): (usize, usize)) -> &[T] {
    std::slice::from_raw_parts(b.as_ptr().add(off) as *const T, len / std::mem::size_of::<T>())
}

fn bytes_of<T>(v: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

#[inline]
fn fx(key: &[u8]) -> u32 {
    // Hash for the in-memory index, 8 bytes at a time.
    const K: u64 = 0x517c_c1b7_2722_0a95;
    let mut h: u64 = key.len() as u64;
    let mut chunks = key.chunks_exact(8);
    for c in &mut chunks {
        let w = u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
        h = (h.rotate_left(5) ^ w).wrapping_mul(K);
    }
    let r = chunks.remainder();
    if !r.is_empty() {
        let mut b = [0u8; 8];
        b[..r.len()].copy_from_slice(r);
        h = (h.rotate_left(5) ^ u64::from_le_bytes(b)).wrapping_mul(K);
    }
    ((h ^ (h >> 29)).wrapping_mul(K) >> 32) as u32
}

/// Index of the first CR or LF at or after I (word-at-a-time scan).
#[inline]
fn find_eol(d: &[u8], mut i: usize) -> usize {
    const LO: u64 = 0x0101_0101_0101_0101;
    const HI: u64 = 0x8080_8080_8080_8080;
    let n = d.len();
    while i + 8 <= n {
        let w = u64::from_le_bytes(d[i..i + 8].try_into().unwrap());
        let a = w ^ (LO * b'\n' as u64);
        let b = w ^ (LO * b'\r' as u64);
        let z = (a.wrapping_sub(LO) & !a & HI) | (b.wrapping_sub(LO) & !b & HI);
        if z != 0 {
            return i + (z.trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    while i < n && d[i] != b'\n' && d[i] != b'\r' {
        i += 1;
    }
    i
}

impl Db {
    fn new(track_c_buckets: bool) -> Db {
        Db {
            bufs: Vec::new(),
            dir_buf: Vec::new(),
            dirs: Vec::new(),
            entries: Vec::new(),
            table: Vec::new(),
            mask: 0,
            extra: HashMap::new(),
            c_buckets: if track_c_buckets { Some(vec![0; DB_HASH_SIZE as usize]) } else { None },
            mapped: None,
        }
    }

    fn buf(&self, i: usize) -> &[u8] {
        match &self.mapped {
            Some(m) => {
                let (o, n) = m.bufs[i];
                &m.map.bytes()[o..o + n]
            }
            None => &self.bufs[i],
        }
    }
    fn entries_s(&self) -> &[Entry] {
        match &self.mapped {
            Some(m) => unsafe { view(m.map.bytes(), m.entries) },
            None => &self.entries,
        }
    }
    fn table_s(&self) -> &[u32] {
        match &self.mapped {
            Some(m) => unsafe { view(m.map.bytes(), m.table) },
            None => &self.table,
        }
    }
    fn dir(&self, i: usize) -> &[u8] {
        let (o, l) = match &self.mapped {
            Some(m) => {
                let d: &[u32] = unsafe { view(m.map.bytes(), m.dirs) };
                (d[2 * i], d[2 * i + 1])
            }
            None => self.dirs[i],
        };
        let b: &[u8] = match &self.mapped {
            Some(m) => &m.map.bytes()[m.dir_buf.0..m.dir_buf.0 + m.dir_buf.1],
            None => &self.dir_buf,
        };
        &b[o as usize..(o + l) as usize]
    }

    fn name(&self, e: &Entry) -> &[u8] {
        &self.buf(e.buf as usize)[e.off as usize..(e.off + e.len) as usize]
    }

    /// The cache file contents for this (freshly built) database.
    fn serialize(&self, stamp: u64, dir_list: &[Vec<u8>]) -> Vec<u8> {
        let mut w = crate::dbcache::Writer::new(stamp);
        w.u64(self.bufs.len() as u64);
        for b in &self.bufs {
            w.blob(b);
        }
        w.blob(&self.dir_buf);
        let flat: Vec<u32> = self.dirs.iter().flat_map(|&(o, l)| [o, l]).collect();
        w.blob(bytes_of(&flat));
        w.blob(bytes_of(&self.entries));
        w.blob(bytes_of(&self.table));
        w.u64(self.mask as u64);
        w.u64(dir_list.len() as u64);
        for d in dir_list {
            w.blob(d);
        }
        w.buf
    }

    /// A database from a cache file, and the db_dir_list entries it adds.
    fn load(path: &std::path::Path, stamp: u64) -> Option<(Db, Vec<Vec<u8>>)> {
        let map = crate::dbcache::Map::open(path)?;
        let (bufs, dir_buf, dirs, entries, table, mask, dir_list) = {
            let mut r = crate::dbcache::Reader::new(map.bytes(), stamp)?;
            let nb = r.u64()? as usize;
            let mut bufs = Vec::with_capacity(nb);
            for _ in 0..nb {
                bufs.push(r.blob()?);
            }
            let dir_buf = r.blob()?;
            let dirs = r.blob()?;
            let entries = r.blob()?;
            let table = r.blob()?;
            let mask = r.u64()? as usize;
            let nd = r.u64()? as usize;
            let mut dir_list = Vec::with_capacity(nd);
            for _ in 0..nd {
                let (o, n) = r.blob()?;
                dir_list.push(map.bytes()[o..o + n].to_vec());
            }
            if entries.1 % std::mem::size_of::<Entry>() != 0 || dirs.1 % 8 != 0 || table.1 != (mask + 1) * 4 {
                return None;
            }
            (bufs, dir_buf, dirs, entries, table, mask, dir_list)
        };
        let mut db = Db::new(false);
        db.mask = mask;
        db.mapped = Some(Mapped { map, bufs, dir_buf, dirs, entries, table });
        Some((db, dir_list))
    }

    fn build_table(&mut self) {
        let n = self.entries.len();
        let mut size = 16usize;
        while size < n * 2 {
            size <<= 1;
        }
        self.mask = size - 1;
        self.table = vec![0u32; size];
        for (i, e) in self.entries.iter().enumerate() {
            let mut p = e.hash as usize & self.mask;
            while self.table[p] != 0 {
                p = (p + 1) & self.mask;
            }
            self.table[p] = i as u32 + 1;
        }
    }

    /// `hash_lookup (kpse->db, key)`: directories of all entries for KEY.
    pub(crate) fn lookup(&self, key: &[u8]) -> Vec<&[u8]> {
        let lower = key.to_ascii_lowercase();
        let mut out: Vec<&[u8]> = Vec::new();
        let table = self.table_s();
        let entries = self.entries_s();
        if !table.is_empty() {
            let h = fx(&lower);
            let high = !key.is_ascii();
            let c_bucket = if high { c_hash(DB_HASH_SIZE, key) } else { 0 };
            let mut p = h as usize & self.mask;
            loop {
                let t = table[p];
                if t == 0 {
                    break;
                }
                let e = &entries[t as usize - 1];
                if e.hash == h && self.name(e) == lower.as_slice() {
                    // C stored the entry in bucket hash_normalized(name);
                    // the look-up searches bucket hash(key).
                    if !high || c_hash_normalized(DB_HASH_SIZE, self.name(e)) == c_bucket {
                        out.push(self.dir(e.dir as usize));
                    }
                }
                p = (p + 1) & self.mask;
            }
        }
        if let Some(v) = self.extra.get(&lower) {
            for d in v {
                out.push(d);
            }
        }
        out
    }
}

/// db.c `ignore_dir_p`.
pub(crate) fn ignore_dir_p(dirname: &[u8]) -> bool {
    let mut i = 1usize;
    while i < dirname.len() {
        if dirname[i] == b'.'
            && is_dir_sep_ch(dirname[i - 1])
            && at(dirname, i + 1) != 0
            && !is_dir_sep_ch(at(dirname, i + 1))
        {
            return true;
        }
        i += 1;
    }
    false
}

/// db.c `match`: could FILENAME be in PATH_ELT (with `//` wildcards)?
pub(crate) fn db_match(filename: &[u8], path_elt: &[u8]) -> bool {
    let fc = |i: usize| at(filename, i);
    let pc = |i: usize| at(path_elt, i);
    let mut f = 0usize;
    let mut p = 0usize;
    let mut matched = false;
    while fc(f) != 0 && pc(p) != 0 {
        if file_char_case_eq(fc(f), pc(p)) {
            // normal character match
        } else if is_dir_sep_ch(pc(p)) && f > 0 && is_dir_sep_ch(pc(p - 1)) {
            while is_dir_sep_ch(pc(p)) {
                p += 1;
            }
            if pc(p) == 0 {
                matched = true;
                break;
            } else {
                while !matched && fc(f) != 0 {
                    if is_dir_sep_ch(fc(f - 1)) && file_char_case_eq(fc(f), pc(p)) {
                        matched = db_match(&filename[f..], &path_elt[p..]);
                    }
                    f += 1;
                }
                break;
            }
        } else {
            break;
        }
        f += 1;
        p += 1;
    }
    if !matched && pc(p) == 0 {
        if is_dir_sep_ch(fc(f)) {
            f += 1;
        }
        if f == 0 || is_dir_sep_ch(fc(f - 1)) {
            while fc(f) != 0 && !is_dir_sep_ch(fc(f)) {
                f += 1;
            }
            matched = fc(f) == 0;
        }
    }
    matched
}

/// db.c `elt_in_db`: is DB_DIR a (case-insensitive) prefix of PATH_ELT?
pub(crate) fn elt_in_db(db_dir: &[u8], path_elt: &[u8]) -> bool {
    if db_dir.is_empty() || path_elt.is_empty() {
        return false;
    }
    let mut i = 0usize;
    loop {
        if !file_char_case_eq(at(db_dir, i), at(path_elt, i)) {
            return false;
        }
        i += 1;
        if at(db_dir, i) == 0 {
            return true;
        }
        if at(path_elt, i) == 0 {
            return false;
        }
    }
}

impl Kpse {
    /// db.c `db_build`: add the entries of DB_FILENAME; false if it could
    /// not be read or had no entries.
    fn db_build(&mut self, db: &mut Db, db_filename: &[u8]) -> bool {
        let top_len = db_filename.len().saturating_sub(4);
        let top_dir = db_filename[..top_len].to_vec();
        let data = match read_text_file(&self.to_os_path(db_filename)) {
            Ok(d) => {
                self.trace_fopen(db_filename, b"r", true);
                d
            }
            Err(_) => {
                self.trace_fopen(db_filename, b"r", false);
                return false;
            }
        };
        // TRANSFORM every byte (kanji-aware), as db_build does per line.
        let mut data = data;
        if self.is_cp932_system != 0 {
            let mut i = 0;
            while i < data.len() {
                if self.is_kanji(&data, i) {
                    i += 2;
                    continue;
                }
                data[i] = transform(data[i]);
                i += 1;
            }
        } else if data.is_ascii() {
            data.make_ascii_lowercase();
        } else {
            for b in data.iter_mut() {
                *b = transform(*b);
            }
        }
        let buf_id = db.bufs.len() as u32;
        let mut file_count = 0u32;
        let mut dir_count = 0u32;
        let mut ignore_dir_count = 0u32;
        let mut cur_dir: Option<u32> = None;
        let mut extra_bufs: Vec<Vec<u8>> = Vec::new();
        let has_nul = data.contains(&0);
        db.entries.reserve(data.len() / 16 + 16);
        {
            let n = data.len();
            let mut pos = 0usize;
            while pos < n {
                let start = pos;
                let mut i = find_eol(&data, start);
                let end = i;
                if i < n {
                    if data[i] == b'\r' && i + 1 < n && data[i + 1] == b'\n' {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                pos = i;
                let raw = &data[start..end];
                let cleaned: Vec<u8>;
                let line: &[u8] = if has_nul && raw.contains(&0) {
                    cleaned = raw.iter().copied().filter(|&b| b != 0).collect();
                    &cleaned
                } else {
                    raw
                };
                let len = line.len();
                if len > 0 && line[len - 1] == b':' && absolute_p(line, true) {
                    if !ignore_dir_p(line) {
                        let off = db.dir_buf.len() as u32;
                        if line[0] == b'.' {
                            db.dir_buf.extend_from_slice(&top_dir);
                            if len > 2 {
                                db.dir_buf.extend_from_slice(&line[2..len - 1]);
                                db.dir_buf.push(DIR_SEP);
                            }
                        } else {
                            db.dir_buf.extend_from_slice(&line[..len - 1]);
                            db.dir_buf.push(DIR_SEP);
                        }
                        db.dirs.push((off, db.dir_buf.len() as u32 - off));
                        cur_dir = Some(db.dirs.len() as u32 - 1);
                        dir_count += 1;
                    } else {
                        cur_dir = None;
                        ignore_dir_count += 1;
                    }
                } else if len != 0 && line != b"." && line != b".." {
                    if let Some(dir) = cur_dir {
                        let h = fx(line);
                        let e = if raw.len() == line.len() {
                            Entry { buf: buf_id, off: start as u32, len: len as u32, dir, hash: h }
                        } else {
                            let id = buf_id + 1 + extra_bufs.len() as u32;
                            extra_bufs.push(line.to_vec());
                            Entry { buf: id, off: 0, len: len as u32, dir, hash: h }
                        };
                        if let Some(cb) = db.c_buckets.as_mut() {
                            cb[c_hash_normalized(DB_HASH_SIZE, line) as usize] += 1;
                        }
                        db.entries.push(e);
                        file_count += 1;
                    }
                }
            }
        }
        db.bufs.push(data);
        db.bufs.extend(extra_bufs);
        self.trace_fclose();

        if file_count == 0 {
            let m = bcat!(b"kpathsea: ", db_filename, b": No usable entries in ls-R");
            self.warning(&m);
            self.warning(b"kpathsea: See the manual for how to generate ls-R");
        } else {
            self.db_dir_list.push(top_dir);
        }
        if self.debug_p(KPSE_DEBUG_HASH) {
            let m = bcat!(
                db_filename,
                b": ",
                file_count,
                b" entries in ",
                dir_count as i32,
                b" directories (",
                ignore_dir_count as i32,
                b" hidden).\n"
            );
            self.debugf(&m);
            let (nonempty, total) = match db.c_buckets.as_ref() {
                Some(cb) => {
                    let ne = cb.iter().filter(|&&c| c > 0).count() as u32;
                    let t: u32 = cb.iter().sum();
                    (ne, t)
                }
                None => (0, 0),
            };
            let mut m = b"ls-R hash table:".to_vec();
            m.extend_from_slice(&hash_summary_line(DB_HASH_SIZE, nonempty, total));
            self.debugf(&m);
        }
        file_count != 0
    }

    /// db.c `alias_build`.
    fn alias_build(&mut self, table: &mut CHash, alias_filename: &[u8]) -> bool {
        let data = match read_text_file(&self.to_os_path(alias_filename)) {
            Ok(d) => {
                self.trace_fopen(alias_filename, b"r", true);
                d
            }
            Err(_) => {
                self.trace_fopen(alias_filename, b"r", false);
                return false;
            }
        };
        let mut count = 0u32;
        let mut lines = Lines::new(&data);
        while let Some(line) = lines.next_line() {
            if line.is_empty() || line[0] == b'%' || line[0] == b'#' {
                continue;
            }
            let n = line.len();
            let mut r = 0;
            while r < n && isspace(line[r]) {
                r += 1;
            }
            let mut a = r;
            while a < n && !isspace(line[a]) {
                a += 1;
            }
            let real = &line[r..a];
            // `*alias++ = 0`: past the terminator if there was no space.
            let mut a2 = a + 1;
            if a >= n {
                a2 = n;
            }
            while a2 < n && isspace(line[a2]) {
                a2 += 1;
            }
            let alias = &line[a2.min(n)..];
            if !real.is_empty() && !alias.is_empty() {
                table.insert_normalized(alias.to_vec(), real.to_vec());
                count += 1;
            }
        }
        self.trace_fclose();
        if self.debug_p(KPSE_DEBUG_HASH) {
            let m = bcat!(alias_filename, b": ", count, b" aliases.\n");
            self.debugf(&m);
            let mut m = b"alias hash table:".to_vec();
            m.extend_from_slice(&table.summary());
            self.debugf(&m);
        }
        true
    }

    /// db.c `kpathsea_init_db`.
    pub(crate) fn init_db(&mut self) {
        let db_path = self.init_format(FileFormat::Db);
        let names: Vec<Vec<u8>> = DB_NAMES.iter().map(|n| n.to_vec()).collect();
        let db_files = self.path_search_list_generic(&db_path, &names, true, true);
        let mut unique: Vec<Vec<u8>> = Vec::new();
        for i in 0..db_files.len() {
            let path1 = &db_files[i];
            let path2 = db_files.get(i + 1);
            if let Some(p2) = path2 {
                if path1.eq_ignore_ascii_case(p2) && sys::same_file(self.win32_codepage, path1, p2) {
                    if self.debug_p(KPSE_DEBUG_HASH) {
                        let m = bcat!(b"db:init(): skipping db same_file_p ", path1, b", will add ", p2, b".\n");
                        self.debugf(&m);
                    }
                    continue;
                }
            }
            if self.debug_p(KPSE_DEBUG_HASH) {
                let m = bcat!(b"db:init(): using db file ", path1, b".\n");
                self.debugf(&m);
            }
            unique.push(path1.clone());
        }
        // the ls-R cache (dbcache.rs): never with hash debugging, which traces the build
        let cache = if self.debug_p(KPSE_DEBUG_HASH) { None } else { crate::dbcache::dir() };
        let os_files: Vec<std::path::PathBuf> = unique.iter().map(|f| self.to_os_path(f)).collect();
        let stamp = cache.as_ref().and_then(|_| crate::dbcache::stamp(&os_files, self.is_cp932_system));
        let cached = match (&cache, stamp) {
            (Some(dir), Some(st)) => Db::load(&crate::dbcache::file_for(dir, st), st),
            _ => None,
        };
        if let Some((db, dir_list)) = cached {
            self.db_dir_list.extend(dir_list);
            self.db = Some(db);
        } else {
            let mut db = Db::new(self.debug_p(KPSE_DEBUG_HASH));
            let mut ok = false;
            let mut all_ok = true;
            let dir_list_start = self.db_dir_list.len();
            for f in &unique {
                if self.db_build(&mut db, f) {
                    ok = true;
                } else {
                    all_ok = false;
                }
            }
            if ok {
                db.c_buckets = None;
                db.build_table();
                if let (Some(dir), Some(st), true) = (&cache, stamp, all_ok) {
                    // only clean builds are cached: a failed one printed warnings
                    let data = db.serialize(st, &self.db_dir_list[dir_list_start..]);
                    crate::dbcache::save(dir, st, &data);
                }
                self.db = Some(db);
            } else {
                self.db = None;
            }
        }

        let alias_files = self.all_path_search(&db_path, ALIAS_NAME);
        let mut table = CHash::new(ALIAS_HASH_SIZE);
        let mut ok = false;
        for f in &alias_files {
            if self.alias_build(&mut table, f) {
                ok = true;
            }
        }
        self.alias_db = if ok { Some(table) } else { None };
    }

    /// `kpathsea_db_insert`: record a file created during the run.
    pub fn db_insert(&mut self, passed_fname: &[u8]) {
        if let Some(db) = self.db.as_mut() {
            let base = xbasename(passed_fname);
            let file_part = base.to_vec();
            let dir_part = passed_fname[..passed_fname.len() - base.len()].to_vec();
            db.extra.entry(file_part.to_ascii_lowercase()).or_default().push(dir_part);
        }
    }

    fn db_lookup(&mut self, key: &[u8]) -> Vec<Vec<u8>> {
        let r: Vec<Vec<u8>> = match self.db.as_ref() {
            Some(db) => db.lookup(key).into_iter().map(|d| d.to_vec()).collect(),
            None => Vec::new(),
        };
        if self.compat_def && self.debug_p(KPSE_DEBUG_HASH) {
            let refs: Vec<&[u8]> = r.iter().map(|v| v.as_slice()).collect();
            self.trace_hash_lookup(key, &refs, false);
        }
        r
    }

    fn alias_lookup(&mut self, name: &[u8]) -> Vec<Vec<u8>> {
        let r: Vec<Vec<u8>> = match self.alias_db.as_ref() {
            Some(a) => a.lookup(name).into_iter().map(|v| v.to_vec()).collect(),
            None => return Vec::new(),
        };
        if self.compat_def && self.debug_p(KPSE_DEBUG_HASH) {
            let refs: Vec<&[u8]> = r.iter().map(|v| v.as_slice()).collect();
            self.trace_hash_lookup(name, &refs, false);
        }
        r
    }

    fn db_relevant(&self, path_elt: &[u8]) -> bool {
        self.db_dir_list.iter().any(|d| elt_in_db(d, path_elt))
    }

    /// Try DB_FILE (and the aliases' names in the same directory).
    fn db_try(&mut self, dir: &[u8], ctry: &[u8], path_elt: &[u8], aliases: &[Vec<u8>]) -> Option<Vec<u8>> {
        let db_file = cat(&[dir, ctry]);
        let matched = db_match(&db_file, path_elt);
        if self.debug_p(KPSE_DEBUG_SEARCH) {
            let m = bcat!(b"db:match(", &db_file, b",", path_elt, b") = ", matched, b"\n");
            self.debugf(&m);
        }
        if !matched {
            return None;
        }
        if let Some(f) = self.readable_file(db_file) {
            return Some(f);
        }
        for a in aliases.iter().skip(1) {
            let atry = cat(&[dir, a]);
            if let Some(f) = self.readable_file(atry) {
                return Some(f);
            }
        }
        None
    }

    /// db.c `kpathsea_db_search`: `None` if no database covers PATH_ELT.
    pub(crate) fn db_search(&mut self, name: &[u8], orig_path_elt: &[u8], all: bool) -> Option<Vec<Vec<u8>>> {
        self.db.as_ref()?;
        let (name, path_elt): (&[u8], Vec<u8>) = match name.iter().rposition(|&c| c == b'/') {
            Some(ls) if ls != 0 => (&name[ls + 1..], cat(&[orig_path_elt, b"/", &name[..ls]])),
            _ => (name, orig_path_elt.to_vec()),
        };
        if !self.db_relevant(&path_elt) {
            return None;
        }
        let mut aliases = vec![name.to_vec()];
        aliases.extend(self.alias_lookup(name));
        let mut ret: Vec<Vec<u8>> = Vec::new();
        let mut done = false;
        for r in 0..aliases.len() {
            if done {
                break;
            }
            let ctry = aliases[r].clone();
            ret = Vec::new();
            let dirs = self.db_lookup(&ctry);
            for dir in dirs {
                if done {
                    break;
                }
                if let Some(found) = self.db_try(&dir, &ctry, &path_elt, &aliases) {
                    ret.push(found);
                    if !all {
                        done = true;
                    }
                }
            }
        }
        Some(ret)
    }

    /// db.c `kpathsea_db_search_list`.
    pub(crate) fn db_search_list(&mut self, names: &[Vec<u8>], path_elt: &[u8], all: bool) -> Option<Vec<Vec<u8>>> {
        self.db.as_ref()?;
        if !self.db_relevant(path_elt) {
            return None;
        }
        let mut ret: Vec<Vec<u8>> = Vec::new();
        let mut done = false;
        for full in names {
            if done {
                break;
            }
            if absolute_p(full, true) {
                continue;
            }
            let (name, path): (&[u8], Vec<u8>) = match full.iter().rposition(|&c| c == b'/') {
                Some(ls) if ls != 0 => (&full[ls + 1..], cat(&[path_elt, b"/", &full[..ls]])),
                _ => (&full[..], path_elt.to_vec()),
            };
            let mut aliases = vec![name.to_vec()];
            aliases.extend(self.alias_lookup(name));
            for r in 0..aliases.len() {
                if done {
                    break;
                }
                let ctry = aliases[r].clone();
                let dirs = self.db_lookup(&ctry);
                for dir in dirs {
                    if done {
                        break;
                    }
                    if let Some(found) = self.db_try(&dir, &ctry, &path, &aliases) {
                        ret.push(found);
                        if !all {
                            done = true;
                        }
                    }
                }
            }
        }
        Some(ret)
    }
}
