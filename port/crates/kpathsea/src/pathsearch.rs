//! pathsearch.c: searching a path for one or several file names, plus
//! readable.c (`kpathsea_readable_file`) and str-list.c's uniqify.
//!
//! On Windows `MONOCASE_FILENAMES` is defined, so the case-folding
//! fallback search of Unix kpathsea is compiled out (debug output says
//! `casefold=No`).

use std::io::Write;

use crate::debug::KPSE_DEBUG_SEARCH;
use crate::sys;
use crate::util::*;
use crate::Kpse;

/// `NAME_MAX` from c-namemx.h.
const NAME_MAX: usize = 255;

/// `print_space_list`: "[a b c]".
fn space_list(l: &[Vec<u8>]) -> Vec<u8> {
    let mut v = vec![b'['];
    for (i, s) in l.iter().enumerate() {
        if i > 0 {
            v.push(b' ');
        }
        v.extend_from_slice(s);
    }
    v.push(b']');
    v
}

/// str-list.c `str_list_uniqify` (case-insensitive on Windows).
fn uniqify(l: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let mut ret: Vec<Vec<u8>> = Vec::with_capacity(l.len());
    for e in l {
        if !ret.iter().any(|r| file_str_case_eq(r, &e)) {
            ret.push(e);
        }
    }
    ret
}

enum Readable {
    Yes,
    No,
    NameTooLong,
    Access,
}

impl Kpse {
    /// readable.c `READABLE` (WIN32 version).
    fn readable_check(&mut self, f: &[u8]) -> Readable {
        let len = f.len();
        let ends_ci = |suf: &[u8]| len >= suf.len() && f[len - suf.len()..].eq_ignore_ascii_case(suf);
        let mut special = ends_ci(b"nul") || ends_ci(b"nul:") || ends_ci(b"nul.tex") || ends_ci(b"nul:.tex");
        if !special {
            special = find_sub(f, b".\\", 0).is_some() || find_sub(f, b"./", 0).is_some();
        }
        if !special && len > 2 {
            special = find_sub(f, b"//", 2).is_some()
                || find_sub(f, b"\\\\", 2).is_some()
                || find_sub(f, b"\\/", 2).is_some()
                || find_sub(f, b"/\\", 2).is_some();
        }
        let mut fnn: Vec<u8> = if !special
            && len > 2
            && ((f[0] == b'/' && f[1] == b'/') || (f[0] == b'\\' && f[1] == b'\\' && f[2] != b'?'))
        {
            cat(&[b"\\\\?\\UNC\\", &f[2..]])
        } else if !special && len > 2 && f[1] == b':' {
            cat(&[b"\\\\?\\", f])
        } else {
            f.to_vec()
        };
        for c in fnn.iter_mut() {
            if *c == b'/' {
                *c = b'\\';
            }
        }
        match sys::get_file_attributes_w(self.fs_cp(), &fnn) {
            Ok(a) => {
                if a & sys::FILE_ATTRIBUTE_DIRECTORY != 0 {
                    Readable::No
                } else {
                    Readable::Yes
                }
            }
            Err(sys::ERROR_BUFFER_OVERFLOW) => Readable::NameTooLong,
            Err(sys::ERROR_ACCESS_DENIED) => Readable::Access,
            Err(_) => Readable::No,
        }
    }

    /// readable.c `kpathsea_readable_file`: NAME (normalized) if it exists
    /// and is not a directory.
    pub fn readable_file(&mut self, name: impl AsRef<[u8]>) -> Option<Vec<u8>> {
        let mut name = name.as_ref().to_vec();
        self.normalize_path(&mut name);
        match self.readable_check(&name) {
            Readable::Yes => Some(name),
            Readable::No => None,
            Readable::NameTooLong => {
                // Truncate any too-long components in NAME.
                let mut out: Vec<u8> = Vec::with_capacity(name.len());
                let mut comp = 0usize;
                for &c in &name {
                    if is_dir_sep(c) || is_device_sep(c) {
                        comp = 0;
                        out.push(c);
                    } else {
                        comp += 1;
                        if comp <= NAME_MAX {
                            out.push(c);
                        }
                    }
                }
                match self.readable_check(&out) {
                    Readable::Yes => Some(out),
                    _ => None,
                }
            }
            Readable::Access => {
                if !self.tex_hush(b"readable") {
                    let m = bcat!(&name, b": Permission denied\n");
                    self.err_write(&m);
                }
                None
            }
        }
    }

    /// tex-hush.c `kpathsea_tex_hush`.
    pub(crate) fn tex_hush(&mut self, what: &[u8]) -> bool {
        match self.var_value(b"TEX_HUSH") {
            None => false,
            Some(h) => {
                if h == b"all" {
                    return true;
                }
                if h == b"none" {
                    return false;
                }
                self.path_elements(&h).iter().any(|e| *e == what)
            }
        }
    }

    /// pathsearch.c `log_search`.
    fn log_search(&mut self, filenames: &[Vec<u8>]) {
        if !self.log_opened {
            let log_name = self.var_value(b"TEXMFLOG");
            self.log_opened = true;
            if let Some(n) = log_name {
                match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(self.to_os_path(&n))
                {
                    Ok(f) => {
                        self.trace_fopen(&n, b"ab", true);
                        self.log_file = Some(f);
                    }
                    Err(e) => {
                        self.trace_fopen(&n, b"ab", false);
                        self.perror(&n, &e);
                    }
                }
            }
        }
        if self.debug_p(KPSE_DEBUG_SEARCH) || self.log_file.is_some() {
            for f in filenames {
                if self.log_file.is_some() && absolute_p(f, false) {
                    let t = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let line = bcat!(t, b" ", f, b"\n");
                    if let Some(lf) = self.log_file.as_mut() {
                        let _ = lf.write_all(&line);
                    }
                }
                if self.debug_p(KPSE_DEBUG_SEARCH) {
                    let m = bcat!(b" ", f);
                    self.err_write(&m);
                }
            }
        }
    }

    /// pathsearch.c `dir_list_search`.
    fn dir_list_search(&mut self, idx: usize, name: &[u8], search_all: bool) -> Vec<Vec<u8>> {
        if self.debug_p(KPSE_DEBUG_SEARCH) {
            let m = bcat!(b"  dir_list_search(file=", name, b", find_all=", search_all, b", casefold=No)\n");
            self.debugf(&m);
        }
        let mut ret = Vec::new();
        let mut i = 0;
        while i < self.elt_cache[idx].dirs.len() {
            let potential = cat(&[&self.elt_cache[idx].dirs[i].name, name]);
            if let Some(r) = self.readable_file(potential) {
                ret.push(r);
                self.elt_cache[idx].float(i);
                if !search_all {
                    return ret;
                }
            }
            i += 1;
        }
        ret
    }

    /// pathsearch.c `dir_list_search_list`.
    fn dir_list_search_list(&mut self, idx: usize, names: &[Vec<u8>], search_all: bool) -> Vec<Vec<u8>> {
        if self.debug_p(KPSE_DEBUG_SEARCH) {
            let m = bcat!(
                b"  dir_list_search_list(files=",
                space_list(names),
                b", find_all=",
                search_all,
                b", casefold=No)\n"
            );
            self.debugf(&m);
        }
        let mut ret = Vec::new();
        let mut i = 0;
        while i < self.elt_cache[idx].dirs.len() {
            let dir = self.elt_cache[idx].dirs[i].name.clone();
            for name in names {
                if absolute_p(name, true) {
                    continue;
                }
                let potential = cat(&[&dir, name]);
                if let Some(r) = self.readable_file(potential) {
                    ret.push(r);
                    self.elt_cache[idx].float(i);
                    if !search_all {
                        return ret;
                    }
                }
            }
            i += 1;
        }
        ret
    }

    /// pathsearch.c `absolute_search`.
    fn absolute_search(&mut self, name: &[u8]) -> Vec<Vec<u8>> {
        let found = self.readable_file(name);
        if self.debug_p(KPSE_DEBUG_SEARCH) {
            let m = bcat!(b" absolute_search(", name, b") => ", or_nil(found.as_deref()), b"\n");
            self.debugf(&m);
        }
        found.into_iter().collect()
    }

    /// pathsearch.c `path_search`.
    fn path_search(&mut self, path: &[u8], name: &[u8], must_exist: bool, all: bool) -> Vec<Vec<u8>> {
        if self.debug_p(KPSE_DEBUG_SEARCH) {
            // Note: C passes `all` and `must_exist` in swapped order.
            let m = bcat!(
                b" path_search(file=",
                name,
                b", must_exist=",
                all,
                b", find_all=",
                must_exist,
                b", path=",
                path,
                b")\n"
            );
            self.debugf(&m);
        }
        let mut ret_list: Vec<Vec<u8>> = Vec::new();
        let elts: Vec<Vec<u8>> = self.path_elements(path).iter().map(|e| e.to_vec()).collect();
        for e in elts {
            let (mut elt, allow_disk_search) = if at(&e, 0) == b'!' && at(&e, 1) == b'!' {
                (e[2..].to_vec(), false)
            } else {
                (e, true)
            };
            self.normalize_path(&mut elt);
            let mut found = if self.followup_search { self.db_search(name, &elt, all) } else { None };
            if allow_disk_search && (found.is_none() || (must_exist && found.as_ref().unwrap().is_empty())) {
                if let Some(idx) = self.element_dirs(&elt) {
                    if !self.elt_cache[idx].dirs.is_empty() {
                        found = Some(self.dir_list_search(idx, name, all));
                    }
                }
            }
            if let Some(f) = found {
                if !f.is_empty() {
                    if all {
                        ret_list.extend(f);
                    } else {
                        ret_list.push(f.into_iter().next().unwrap());
                        break;
                    }
                }
            }
        }
        ret_list
    }

    /// pathsearch.c `search`.
    fn search(&mut self, path: &[u8], original_name: &[u8], must_exist: bool, all: bool) -> Vec<Vec<u8>> {
        let name = self.expand(original_name);
        if self.debug_p(KPSE_DEBUG_SEARCH) {
            let m = bcat!(
                b"start search(xname=",
                &name,
                b", must_exist=",
                must_exist,
                b", find_all=",
                all,
                b", path=",
                path,
                b").\n"
            );
            self.debugf(&m);
        }
        let ret_list = if absolute_p(&name, true) {
            self.absolute_search(&name)
        } else {
            self.path_search(path, &name, must_exist, all)
        };
        if !self.followup_search {
            self.followup_search = true;
        } else {
            if self.debug_p(KPSE_DEBUG_SEARCH) {
                let m = bcat!(b"returning from search(", original_name, b") =>");
                self.debugf(&m);
            }
            self.log_search(&ret_list);
            if self.debug_p(KPSE_DEBUG_SEARCH) {
                self.err_write(b"\n");
            }
        }
        ret_list
    }

    /// pathsearch.c `kpathsea_path_search_list_generic`.
    pub(crate) fn path_search_list_generic(
        &mut self,
        path: &[u8],
        names: &[Vec<u8>],
        must_exist: bool,
        all: bool,
    ) -> Vec<Vec<u8>> {
        if self.debug_p(KPSE_DEBUG_SEARCH) {
            let m = bcat!(
                b"start generic search(files=",
                space_list(names),
                b", must_exist=",
                must_exist,
                b", find_all=",
                all,
                b", path=",
                path,
                b")\n"
            );
            self.debugf(&m);
        }
        let mut ret_list: Vec<Vec<u8>> = Vec::new();
        let mut all_absolute = true;
        let mut done = false;
        for n in names {
            if absolute_p(n, true) {
                let r = self.absolute_search(n);
                if let Some(first) = r.into_iter().next() {
                    ret_list.push(first);
                    if !all {
                        done = true;
                        break;
                    }
                }
            } else {
                all_absolute = false;
            }
        }
        if !done && all_absolute {
            if self.debug_p(KPSE_DEBUG_SEARCH) {
                let mut m = b" generic search: all absolute, candidates are:".to_vec();
                for r in &ret_list {
                    m.push(b' ');
                    m.extend_from_slice(r);
                }
                m.extend_from_slice(b".\n");
                self.debugf(&m);
            }
            done = true;
        }
        if !done {
            let elts: Vec<Vec<u8>> = self.path_elements(path).iter().map(|e| e.to_vec()).collect();
            for e in elts {
                let (mut elt, allow_disk_search) = if at(&e, 0) == b'!' && at(&e, 1) == b'!' {
                    (e[2..].to_vec(), false)
                } else {
                    (e, true)
                };
                self.normalize_path(&mut elt);
                let mut found =
                    if self.followup_search { self.db_search_list(names, &elt, all) } else { None };
                if allow_disk_search
                    && (found.is_none() || (must_exist && found.as_ref().unwrap().is_empty()))
                {
                    if let Some(idx) = self.element_dirs(&elt) {
                        if !self.elt_cache[idx].dirs.is_empty() {
                            found = Some(self.dir_list_search_list(idx, names, all));
                        }
                    }
                }
                if let Some(f) = found {
                    if !f.is_empty() {
                        if all {
                            ret_list.extend(f);
                        } else {
                            ret_list.push(f.into_iter().next().unwrap());
                            break;
                        }
                    }
                }
            }
        }
        let ret_list = uniqify(ret_list);
        if !self.followup_search {
            self.followup_search = true;
        } else {
            if self.debug_p(KPSE_DEBUG_SEARCH) {
                let m = bcat!(b"returning from generic search(", space_list(names), b") =>");
                self.debugf(&m);
            }
            self.log_search(&ret_list);
            if self.debug_p(KPSE_DEBUG_SEARCH) {
                self.err_write(b"\n");
            }
        }
        ret_list
    }

    /// `kpathsea_path_search`: first match of NAME along PATH.
    pub fn path_search_first(&mut self, path: impl AsRef<[u8]>, name: impl AsRef<[u8]>, must_exist: bool) -> Option<Vec<u8>> {
        self.search(path.as_ref(), name.as_ref(), must_exist, false).into_iter().next()
    }

    /// `kpathsea_all_path_search`: all matches of NAME along PATH.
    pub fn all_path_search(&mut self, path: impl AsRef<[u8]>, name: impl AsRef<[u8]>) -> Vec<Vec<u8>> {
        self.search(path.as_ref(), name.as_ref(), true, true)
    }
}
