//! elt-dirs.c (path element -> list of directories, `//` subdirectory
//! expansion, the element cache) and dir.c (`dir_p`, `dir_links`), plus
//! str-llist.c (the "float found directory to the front" list).

use crate::debug::{KPSE_DEBUG_EXPAND, KPSE_DEBUG_STAT};
use crate::sys;
use crate::util::*;
use crate::Kpse;

/// One `str_llist` element.
#[derive(Clone)]
pub(crate) struct DirElt {
    pub(crate) name: Vec<u8>,
    pub(crate) moved: bool,
}

/// The directory list cached for one normalized path element.
pub(crate) struct DirList {
    #[allow(dead_code)]
    pub(crate) key: Vec<u8>,
    pub(crate) dirs: Vec<DirElt>,
}

impl DirList {
    /// str-llist.c `str_llist_float`: move element I just after the
    /// already-moved prefix and mark it moved.  Elements after I keep
    /// their index.
    pub(crate) fn float(&mut self, i: usize) {
        if self.dirs[i].moved {
            return;
        }
        let unmoved = self.dirs.iter().position(|d| !d.moved).unwrap_or(i);
        if unmoved != i {
            let e = self.dirs.remove(i);
            self.dirs.insert(unmoved, e);
            self.dirs[unmoved].moved = true;
        } else {
            self.dirs[i].moved = true;
        }
    }
}

/// elt-dirs.c `dir_list_add`: ensure a trailing `/`.
fn dir_list_add(l: &mut Vec<DirElt>, dir: &[u8]) {
    let last = *dir.last().unwrap_or(&0);
    let saved = if is_dir_sep_ch(last) || is_device_sep(last) {
        dir.to_vec()
    } else {
        cat(&[dir, b"/"])
    };
    l.push(DirElt { name: saved, moved: false });
}

impl Kpse {
    /// elt-dirs.c `kpathsea_normalize_path` (WIN32): slashify, lower-case a
    /// drive letter, collapse leading multiple slashes.  Returns the
    /// length of the device / UNC prefix.
    pub(crate) fn normalize_path(&mut self, elt: &mut Vec<u8>) -> usize {
        self.slashify(elt);
        let ret = if name_begins_with_device(elt) {
            if elt[0].is_ascii_uppercase() {
                elt[0] += b'a' - b'A';
            }
            2
        } else if is_unc_name(elt) {
            let mut r = 2;
            while r < elt.len() && !is_dir_sep_ch(elt[r]) {
                r += 1;
            }
            r
        } else {
            0
        };
        let mut i = ret;
        while i < elt.len() && is_dir_sep_ch(elt[i]) {
            i += 1;
        }
        if i > ret + 1 {
            if self.debug_p(KPSE_DEBUG_STAT) {
                let m = bcat!(b"kpse_normalize_path (", &elt[..], b") => ", ret as u32, b"\n");
                self.debugf(&m);
            }
            elt.drain(ret + 1..i);
        }
        ret
    }

    /// dir.c `kpathsea_dir_p` (WIN32: `GetFileAttributes`, ANSI).
    pub(crate) fn dir_p(&mut self, f: &mut Vec<u8>) -> bool {
        self.normalize_path(f);
        let r = sys::get_file_attributes_a(f);
        if self.debug_p(KPSE_DEBUG_STAT) {
            let m = match r {
                Err(e) => bcat!(b"failed to get file attributes for ", &f[..], b" (", e as i32, b")\n"),
                Ok(a) => bcat!(
                    b"path ",
                    &f[..],
                    if a & sys::FILE_ATTRIBUTE_DIRECTORY != 0 { " is" } else { " is not" },
                    b" a directory\n"
                ),
            };
            self.err_write(&m);
        }
        matches!(r, Ok(a) if a & sys::FILE_ATTRIBUTE_DIRECTORY != 0)
    }

    /// dir.c `kpathsea_dir_links` (WIN32 flavour: the first call with a
    /// nonzero NLINKS memoizes the value, later calls retrieve it).
    pub(crate) fn dir_links(&mut self, f: &[u8], nlinks: i64) -> i64 {
        let key = f.to_ascii_lowercase();
        if self.compat_def && self.debug_p(crate::debug::KPSE_DEBUG_HASH) {
            let v = self.link_table.get(&key).map(|n| n.to_string().into_bytes());
            let vals: Vec<&[u8]> = v.iter().map(|x| x.as_slice()).collect();
            self.trace_hash_lookup(f, &vals, true);
        }
        if let Some(&n) = self.link_table.get(&key) {
            return n;
        }
        if nlinks != 0 {
            self.link_table.insert(key, nlinks);
        }
        if self.debug_p(KPSE_DEBUG_STAT) {
            let m = bcat!(b"dir_links(", f, b") => ", nlinks, b"\n");
            self.debugf(&m);
        }
        nlinks
    }

    /// elt-dirs.c `checked_dir_list_add`.
    fn checked_dir_list_add(&mut self, l: &mut Vec<DirElt>, dir: &mut Vec<u8>) {
        if self.dir_p(dir) {
            dir_list_add(l, dir);
        }
    }

    /// elt-dirs.c `do_subdir` (WIN32 branch): ELT[..ELT_LENGTH] ends with
    /// `/`; POST is what followed the `//`.
    fn do_subdir(&mut self, l: &mut Vec<DirElt>, elt: &[u8], elt_length: usize, post: &[u8]) {
        let mut name = elt[..elt_length].to_vec();
        let pattern = cat(&[&name, b"/*.*"]);
        let cp = self.fs_cp();
        let mut entries: Vec<(Vec<u8>, u32)> = Vec::new();
        if !sys::list_dir_w(cp, &pattern, |n, a| entries.push((n, a))) {
            return;
        }
        if post.is_empty() {
            dir_list_add(l, &name);
        } else {
            name.extend_from_slice(post);
            let nm = name.clone();
            self.expand_elt(l, nm, elt_length);
            name.truncate(elt_length);
        }
        let mut nlinks: i64 = 2;
        for (fname, attrs) in entries {
            if fname.first() == Some(&b'.') {
                continue;
            }
            name.extend_from_slice(&fname);
            let links = self.dir_links(&name, 0);
            if attrs & sys::FILE_ATTRIBUTE_DIRECTORY != 0 {
                let potential_len = name.len() + 1;
                nlinks += 1;
                name.push(b'/');
                if !post.is_empty() {
                    name.extend_from_slice(post);
                    let nm = name.clone();
                    self.expand_elt(l, nm, potential_len);
                    name.truncate(potential_len);
                }
                if links == 0 || links > 2 {
                    let nm = name.clone();
                    self.do_subdir(l, &nm, potential_len, post);
                } else if post.is_empty() {
                    dir_list_add(l, &name);
                }
            }
            name.truncate(elt_length);
        }
        self.dir_links(&name, nlinks);
    }

    /// elt-dirs.c `expand_elt`.
    fn expand_elt(&mut self, l: &mut Vec<DirElt>, mut elt: Vec<u8>, start: usize) {
        let n = elt.len();
        let mut d = start;
        while d < n {
            if is_dir_sep_ch(elt[d]) && d + 1 < n && is_dir_sep_ch(elt[d + 1]) {
                let mut post = d + 1;
                while post < n && is_dir_sep_ch(elt[post]) {
                    post += 1;
                }
                let p = elt[post..].to_vec();
                self.do_subdir(l, &elt, d + 1, &p);
                return;
            }
            d += 1;
        }
        self.checked_dir_list_add(l, &mut elt);
    }

    /// elt-dirs.c `kpathsea_element_dirs`: the (cached) directory list for
    /// path element ELT, as an index into the cache; `None` for an empty
    /// element.
    pub(crate) fn element_dirs(&mut self, elt: &[u8]) -> Option<usize> {
        if elt.is_empty() {
            return None;
        }
        let mut e = elt.to_vec();
        if self.file_system_codepage != self.win32_codepage && !e.is_ascii() {
            // Re-encode from the ANSI code page to the file-system one.
            let w = sys::mb_to_wide(self.win32_codepage, &e);
            e = sys::wide_to_mb(self.file_system_codepage, &w);
        }
        let i = self.normalize_path(&mut e);
        let key = e.to_ascii_lowercase();
        if let Some(&idx) = self.elt_cache_index.get(&key) {
            return Some(idx);
        }
        let mut list: Vec<DirElt> = Vec::new();
        self.expand_elt(&mut list, e.clone(), i);
        let idx = self.elt_cache.len();
        self.elt_cache.push(DirList { key: e.clone(), dirs: list });
        self.elt_cache_index.insert(key, idx);
        if self.debug_p(KPSE_DEBUG_EXPAND) {
            let mut m = bcat!(b"kpathsea_element_dirs: path element ", &e, b" =>");
            for d in &self.elt_cache[idx].dirs {
                m.push(b' ');
                m.extend_from_slice(&d.name);
            }
            m.push(b'\n');
            self.debugf(&m);
        }
        Some(idx)
    }
}
