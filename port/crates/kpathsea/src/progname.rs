//! progname.c (WIN32 branch of `kpathsea_set_program_name`), the user-info
//! and long-path helpers of win32lib.c, `kpathsea_reset_program_name`
//! (tex-file.c) and the command-line-encoding logic of knj.c.

use crate::debug::ErrSink;
use crate::sys;
use crate::util::*;
use crate::{Kpse, LAST_FORMAT};

/// `EXE_SUFFIXES` from progname.c (used if `PATHEXT` is unset).
const EXE_SUFFIXES: &[u8] = b".com;.exe;.bat;.cmd;.vbs;.vbe;.js;.jse;.wsf;.wsh;.ws;.tcl;.py;.pyw";

/// `PATH_MAX` used for the program's own long path.
const PATH_MAX: usize = 260;

impl Kpse {
    /// `kpathsea_set_program_name (kpse, argv0, progname)`.
    pub fn set_program_name(&mut self, argv0: &[u8], progname: Option<&[u8]>) {
        self.set_program_name_common(argv0, None, progname);
    }

    /// Variant taking the executable path explicitly.
    pub fn set_program_name_from_path(&mut self, exe_path: &[u8], progname: Option<&[u8]>) {
        let base = xbasename(exe_path).to_vec();
        self.set_program_name_common(&base, Some(exe_path), progname);
    }

    fn set_program_name_common(
        &mut self,
        argv0: &[u8],
        exe_path: Option<&[u8]>,
        progname: Option<&[u8]>,
    ) {
        if let Some(s) = self.env.get(b"KPATHSEA_DEBUG") {
            self.debug |= atoi(&s) as u32;
        }
        let debug_output = self.env.get(b"KPATHSEA_DEBUG_OUTPUT");
        let append_debug_output = self.env.get(b"KPATHSEA_DEBUG_APPEND");

        let cp = sys::file_apis_codepage();
        if self.file_system_codepage == 0 {
            self.file_system_codepage = cp;
        }
        self.win32_codepage = cp;
        self.is_cp932_system = if cp == 932 || cp == 936 || cp == 950 { cp } else { 0 };

        self.init_user_info();

        // Redirect "stderr" to KPATHSEA_DEBUG_OUTPUT.
        if let Some(out) = debug_output {
            if out.eq_ignore_ascii_case(b"con") || out.eq_ignore_ascii_case(b"con:") {
                // C dups stdout onto stderr; we keep writing to stderr.
            } else {
                let mut opts = std::fs::OpenOptions::new();
                if append_debug_output.is_some() {
                    opts.create(true).append(true);
                } else {
                    opts.create(true).write(true).truncate(true);
                    self.env.put(b"KPATHSEA_DEBUG_APPEND", b"yes");
                }
                match opts.open(sys::bytes_to_os(self.win32_codepage, &out)) {
                    Ok(f) => self.err = ErrSink::File(f),
                    Err(e) => {
                        let m = bcat!(b"kpathsea: Can't open ", &out, b" for stderr redirection!\n");
                        self.warning(&m);
                        self.perror(&out, &e);
                    }
                }
            }
        }

        // SearchPath + getlongpath: the directory of the executable.
        let path = match exe_path {
            Some(p) => {
                let mut p = p.to_vec();
                self.slashify(&mut p);
                self.long_self_path(&p)
            }
            None => self.locate_self(argv0),
        };
        let sdir = xdirname(&path);
        self.invocation_name = xbasename(&path).to_vec();

        self.xputenv(b"SELFAUTOLOC", fix_selfdir(&sdir));
        let sdir_parent = xdirname(&sdir);
        self.xputenv(b"SELFAUTODIR", fix_selfdir(&sdir_parent));
        let sdir_grandparent = xdirname(&sdir_parent);
        self.xputenv(b"SELFAUTOPARENT", fix_selfdir(&sdir_grandparent));
        let sdir_ggp = xdirname(&sdir_grandparent);
        self.xputenv(b"SELFAUTOGRANDPARENT", fix_selfdir(&sdir_ggp));

        self.mk_suffixlist();

        self.invocation_short_name = xbasename(&self.invocation_name).to_vec();

        self.program_name = match progname {
            Some(p) => p.to_vec(),
            None => {
                let s = &self.invocation_short_name;
                match find_suffix(s) {
                    Some(i) if s[i..].eq_ignore_ascii_case(b"exe") => remove_suffix(s),
                    _ => s.clone(),
                }
            }
        };
        let pn = self.program_name.clone();
        self.xputenv(b"progname", &pn);
    }

    /// Find the executable as `SearchPath (NULL, argv0, ".exe", ...)` would
    /// and convert it to its long name.  `TEXRS_SELFAUTOLOC` overrides the
    /// directory.
    fn locate_self(&mut self, argv0: &[u8]) -> Vec<u8> {
        let acp = self.win32_codepage;
        let with_ext = |n: &[u8]| -> Vec<u8> {
            if find_suffix(xbasename(n)).is_some() {
                n.to_vec()
            } else {
                cat(&[n, b".exe"])
            }
        };
        if let Some(dir) = self.env.get(b"TEXRS_SELFAUTOLOC").filter(|d| !d.is_empty()) {
            let mut d = dir;
            self.slashify(&mut d);
            while d.len() > 1 && d.ends_with(b"/") && !(d.len() == 3 && d[1] == b':') {
                d.pop();
            }
            let name = with_ext(xbasename(argv0));
            let long_dir = self.long_self_path(&d);
            return cat(&[&long_dir, b"/", &name]);
        }
        let exists = |p: &[u8]| sys::get_file_attributes_a(p).is_ok();
        let mut found: Option<Vec<u8>> = None;
        let has_dir = argv0.iter().any(|&c| is_dir_sep(c) || c == b':');
        if has_dir {
            let cand = with_ext(argv0);
            let full = if absolute_p(&cand, false) {
                cand
            } else {
                let cwd = std::env::current_dir()
                    .map(|p| sys::os_to_bytes(acp, p.as_os_str()))
                    .unwrap_or_default();
                cat(&[&cwd, b"\\", &cand])
            };
            if exists(&full) {
                found = Some(full);
            }
        } else {
            let name = with_ext(argv0);
            let mut dirs: Vec<Vec<u8>> = Vec::new();
            if let Ok(exe) = std::env::current_exe() {
                if let Some(d) = exe.parent() {
                    dirs.push(sys::os_to_bytes(acp, d.as_os_str()));
                }
            }
            if let Ok(cwd) = std::env::current_dir() {
                dirs.push(sys::os_to_bytes(acp, cwd.as_os_str()));
            }
            if let Some(p) = self.env.get(b"PATH") {
                for e in p.split(|&c| c == b';') {
                    if !e.is_empty() {
                        dirs.push(e.to_vec());
                    }
                }
            }
            for d in dirs {
                let cand = cat(&[&d, b"\\", &name]);
                if exists(&cand) {
                    found = Some(cand);
                    break;
                }
            }
        }
        let full = match found {
            Some(f) => normalize_dots(&f),
            None => match std::env::current_exe() {
                Ok(p) => sys::os_to_bytes(acp, p.as_os_str()),
                Err(_) => argv0.to_vec(),
            },
        };
        self.long_self_path(&full)
    }

    /// `kpathsea_getlongpath` on the program path; falls back to the
    /// slashified input if it fails.
    fn long_self_path(&mut self, p: &[u8]) -> Vec<u8> {
        match self.getlongpath(p, PATH_MAX) {
            Some(l) => l,
            None => {
                let mut v = p.to_vec();
                self.slashify(&mut v);
                v
            }
        }
    }

    /// win32lib.c `kpathsea_getlongpath`: rebuild INPUT component by
    /// component with the names `FindFirstFile` reports (i.e. with the
    /// case stored on disk).  Returns `None` on failure or if the result
    /// would exceed `len` bytes.
    ///
    /// The long names of directory prefixes are cached for the life of the
    /// instance: a directory's on-disk name cannot change during a run
    /// unless it is renamed, which TeX runs do not do.
    pub(crate) fn getlongpath(&mut self, input: &[u8], len: usize) -> Option<Vec<u8>> {
        let q: Vec<u8> = input.iter().map(|&c| if c == b'/' { b'\\' } else { c }).collect();
        let n = q.len();
        let qa = |i: isize| -> u8 {
            if i < 0 {
                0
            } else {
                at(&q, i as usize)
            }
        };
        let mut buff: Vec<u8> = Vec::new();
        let mut cnt = 0usize;
        let mut p = 0usize;
        if n >= 2 && q[0] == b'\\' && q[1] == b'\\' {
            cnt += 2;
            if cnt > len {
                return None;
            }
            buff.extend_from_slice(b"//");
            p = 2;
            for _ in 0..2 {
                while p < n && q[p] != b'\\' {
                    if self.is_kanji(&q, p) {
                        cnt += 1;
                        if cnt > len {
                            return None;
                        }
                        buff.push(q[p]);
                        p += 1;
                    }
                    cnt += 1;
                    if cnt > len {
                        return None;
                    }
                    buff.push(q[p]);
                    p += 1;
                }
                cnt += 1;
                if cnt > len {
                    return None;
                }
                buff.push(b'/');
                if p < n {
                    p += 1;
                }
            }
        } else if n >= 3 && q[0].is_ascii_alphabetic() && q[1] == b':' && q[2] == b'\\' {
            buff.push(q[0]);
            buff.push(b':');
            buff.push(b'/');
            p = 3;
            cnt += 3;
            if cnt > len {
                return None;
            }
        }

        while p < n {
            if self.is_kanji(&q, p) {
                p += 2;
                continue;
            }
            if q[p] == b'\\' {
                let pi = p as isize;
                if (qa(pi - 2) == b'\\' || p == 1) && qa(pi - 1) == b'.' {
                    cnt += 2;
                    if cnt > len {
                        return None;
                    }
                    buff.extend_from_slice(b"./");
                } else if (qa(pi - 3) == b'\\' || p == 2) && qa(pi - 2) == b'.' && qa(pi - 1) == b'.'
                {
                    cnt += 3;
                    if cnt > len {
                        return None;
                    }
                    buff.extend_from_slice(b"../");
                } else {
                    let prefix = &q[..p];
                    let name = match self.longpath_dir_cache.get(prefix) {
                        Some(nm) => nm.clone(),
                        None => {
                            let nm = sys::find_first_a(prefix)?;
                            self.longpath_dir_cache.insert(prefix.to_vec(), nm.clone());
                            nm
                        }
                    };
                    cnt += name.len() + 1;
                    if cnt > len {
                        return None;
                    }
                    buff.extend_from_slice(&name);
                    buff.push(b'/');
                }
            }
            p += 1;
        }
        let name = sys::find_first_a(&q)?;
        cnt += name.len();
        if cnt > len {
            return None;
        }
        buff.extend_from_slice(&name);
        Some(buff)
    }

    /// win32lib.c `kpathsea_get_home_directory`.
    fn get_home_directory(&mut self) -> Option<Vec<u8>> {
        let home = self.env.get(b"HOME").or_else(|| self.env.get(b"USERPROFILE"));
        home.map(|mut h| {
            self.slashify(&mut h);
            h
        })
    }

    /// win32lib.c `kpathsea_init_user_info`.
    fn init_user_info(&mut self) {
        // pw_name (GetUserName) is only needed for `~user`; it is fetched
        // lazily in `getpwnam_dir`.
        self.passwd_name.clear();
        match self.get_home_directory() {
            Some(h) => self.env.put(b"HOME", &h),
            None => self.env.put(b"HOME", b"c:/"),
        }
        if !self.env.is_set(b"SHELL") {
            self.env.put(b"SHELL", b"cmd");
        }
        if let Some(t) = self.env.get(b"TEXMFTEMP") {
            self.env.put(b"TEMP", &t);
            self.env.put(b"TMP", &t);
        }
        self.passwd_dir = self.get_home_directory().unwrap_or_default();
    }

    /// win32lib.c `kpathsea_getpwnam`: only the current user is known.
    pub(crate) fn getpwnam_dir(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        if self.passwd_name.is_empty() {
            self.passwd_name =
                sys::user_name(self.win32_codepage).unwrap_or_else(|| b"unknown".to_vec());
        }
        if name.eq_ignore_ascii_case(&self.passwd_name) {
            Some(self.passwd_dir.clone())
        } else {
            None
        }
    }

    /// progname.c `mk_suffixlist`.
    fn mk_suffixlist(&mut self) {
        let v = match self.env.get(b"PATHEXT") {
            Some(v) => v.to_ascii_lowercase(),
            None => EXE_SUFFIXES.to_vec(),
        };
        let mut list = vec![b".dll".to_vec()];
        let mut parts: Vec<&[u8]> = v.split(|&c| c == b';').collect();
        // A trailing empty element is not added (`if (*q)`).
        if let Some(last) = parts.last() {
            if last.is_empty() {
                parts.pop();
            }
        }
        for p in parts {
            list.push(p.to_vec());
        }
        self.suffixlist = list;
    }

    /// tex-file.c `kpathsea_reset_program_name`.
    pub fn reset_program_name(&mut self, progname: &str) {
        self.reset_program_name_bytes(progname.as_bytes());
    }

    /// Byte-string version of [`Kpse::reset_program_name`].
    pub fn reset_program_name_bytes(&mut self, progname: &[u8]) {
        if self.program_name == progname {
            return;
        }
        self.program_name = progname.to_vec();
        self.xputenv(b"progname", progname);
        for i in 0..LAST_FORMAT {
            if i == crate::FileFormat::Cnf as usize || i == crate::FileFormat::Db as usize {
                continue;
            }
            let f = &mut self.format_info[i];
            f.path = None;
            f.cnf_path = None;
        }
    }

    /// The `command_line_encoding` handling done at start-up by pdfTeX
    /// (texmfmp.c) and kpsewhich on Windows: if the variable is `utf8` or
    /// `utf-8`, file names are UTF-8 from then on
    /// (`File_system_codepage = CP_UTF8`).  Returns whether UTF-8 is on.
    pub fn init_command_line_encoding(&mut self) -> bool {
        let enc = self.var_value(b"command_line_encoding");
        match enc {
            Some(e) if !e.is_empty() => {
                if e == b"utf8" || e == b"utf-8" {
                    self.file_system_codepage = sys::CP_UTF8;
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

/// progname.c `fix_selfdir`: never just `/`.
fn fix_selfdir(dir: &[u8]) -> &[u8] {
    if dir.len() == 1 && is_dir_sep(dir[0]) {
        &dir[1..]
    } else {
        dir
    }
}

/// Resolve `.` and `..` components (what `SearchPath` returns is a full
/// path name).
fn normalize_dots(p: &[u8]) -> Vec<u8> {
    let s: Vec<u8> = p.iter().map(|&c| if c == b'/' { b'\\' } else { c }).collect();
    let (prefix, rest) = if s.len() >= 2 && s[1] == b':' {
        (s[..2].to_vec(), &s[2..])
    } else {
        (Vec::new(), &s[..])
    };
    let mut comps: Vec<&[u8]> = Vec::new();
    for c in rest.split(|&c| c == b'\\') {
        if c.is_empty() || c == b"." {
            continue;
        }
        if c == b".." {
            comps.pop();
            continue;
        }
        comps.push(c);
    }
    let mut out = prefix;
    for c in comps {
        out.push(b'\\');
        out.extend_from_slice(c);
    }
    out
}
