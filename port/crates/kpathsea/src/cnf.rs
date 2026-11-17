//! cnf.c: reading texmf.cnf files and looking up configuration values.

use crate::hash::CHash;
use crate::util::*;
use crate::{FileFormat, Kpse};

const CNF_HASH_SIZE: u32 = 751;
const CNF_NAME: &[u8] = b"texmf.cnf";

impl Kpse {
    /// cnf.c `do_line`.  LINE is modified in place exactly like the C code
    /// modifies its buffer (NULs written over trailing comments), so that
    /// error messages print the same text.  Returns an error message.
    fn do_line(&mut self, buf: &mut Vec<u8>, env_progname: bool) -> Option<Vec<u8>> {
        let len_total = buf.len();
        let mut line = 0usize;
        while line < len_total && isspace(buf[line]) {
            line += 1;
        }
        if line >= len_total || buf[line] == b'%' || buf[line] == b'#' {
            return None;
        }
        // Remove trailing comment: a % or # preceded by whitespace.
        {
            let mut v = len_total as isize - 1;
            while v > line as isize {
                let c = buf[v as usize];
                if c == b'%' || c == b'#' {
                    v -= 1;
                    while v >= 0 && isspace(buf[v as usize]) {
                        buf[v as usize] = 0;
                        v -= 1;
                    }
                    continue;
                }
                v -= 1;
            }
        }
        // From here on the line is the C string starting at LINE.
        let end = match buf[line..].iter().position(|&c| c == 0) {
            Some(i) => line + i,
            None => len_total,
        };
        let s = &buf[line..end];
        let n = s.len();
        let mut i = 0;
        while i < n && !isspace(s[i]) && s[i] != b'=' && s[i] != b'.' {
            i += 1;
        }
        if i == 0 {
            return Some(b"No cnf variable name".to_vec());
        }
        let var = s[..i].to_vec();
        while i < n && isspace(s[i]) {
            i += 1;
        }
        let mut prog: Option<Vec<u8>> = None;
        if i < n && s[i] == b'.' {
            i += 1;
            while i < n && isspace(s[i]) {
                i += 1;
            }
            let start = i;
            while i < n && !isspace(s[i]) && s[i] != b'=' {
                i += 1;
            }
            let p = s[start..i].to_vec();
            if p.is_empty() {
                return Some(b"Empty program name qualifier".to_vec());
            }
            for &c in &p {
                if c == b'$' || c == b'{' || c == b'}' || is_kpse_sep(c) {
                    let mut m = b"Unlikely character ".to_vec();
                    m.push(c);
                    m.extend_from_slice(b" in program name");
                    return Some(m);
                }
            }
            prog = Some(p);
        }
        while i < n && isspace(s[i]) {
            i += 1;
        }
        if i < n && s[i] == b'=' {
            i += 1;
            while i < n && isspace(s[i]) {
                i += 1;
            }
        }
        let mut vlen = n - i;
        while vlen > 0 && isspace(s[i + vlen - 1]) {
            vlen -= 1;
        }
        if vlen == 0 {
            return Some(b"No cnf value".to_vec());
        }
        let value = s[i..i + vlen].to_vec();
        // IS_ENV_SEP(':') is false on Windows: no ; -> : translation.

        if env_progname {
            self.xputenv(&var, &value);
            let this_prog = prog.unwrap_or_else(|| self.program_name.clone());
            let var_prog = cat(&[&var, b"_", &this_prog]);
            self.xputenv(&var_prog, &value);
        } else {
            let lhs = match prog {
                Some(p) => cat(&[&var, b".", &p]),
                None => var,
            };
            self.cnf_hash.as_mut().expect("cnf hash").insert(lhs, value);
        }
        None
    }

    /// `kpathsea_cnf_line_env_progname`: handle a `-cnf-line` argument.
    pub fn add_cnf_line(&mut self, line: impl AsRef<[u8]>) {
        let orig = line.as_ref().to_vec();
        let mut buf = orig.clone();
        if let Some(msg) = self.do_line(&mut buf, true) {
            let m = bcat!(b"command line (kpathsea): ", msg, b" in argument: ", c_str(&buf));
            self.warning(&m);
        }
    }

    /// cnf.c `read_all_cnf`.
    fn read_all_cnf(&mut self) {
        let cnf_path = self.init_format(FileFormat::Cnf);
        let cnf_files = self.all_path_search(&cnf_path, CNF_NAME);
        if !cnf_files.is_empty() {
            for cnf in cnf_files {
                let ospath = self.to_os_path(&cnf);
                let data = match read_text_file(&ospath) {
                    Ok(d) => {
                        self.trace_fopen(&cnf, b"r", true);
                        d
                    }
                    Err(e) => {
                        // xfopen: FATAL_PERROR (filename).
                        self.trace_fopen(&cnf, b"r", false);
                        let m = bcat!(&self.invocation_name, b": ");
                        self.err_write(&m);
                        self.perror(&cnf, &e);
                        std::process::exit(1);
                    }
                };
                if let Some(f) = self.record_input.as_mut() {
                    f(&cnf);
                }
                let mut lines = Lines::new(&data);
                let mut lineno = 0u32;
                while let Some(l) = lines.next_line() {
                    let mut line = l.to_vec();
                    lineno += 1;
                    while !line.is_empty() && isspace(*line.last().unwrap()) {
                        line.pop();
                    }
                    while !line.is_empty() && *line.last().unwrap() == b'\\' {
                        let next = lines.next_line().map(|n| n.to_vec());
                        lineno += 1;
                        line.pop();
                        match next {
                            None => {
                                let m = bcat!(
                                    &cnf,
                                    b":",
                                    lineno,
                                    b": (kpathsea) Last line of file ends with \\"
                                );
                                self.warning(&m);
                                // C leaves `len` stale here, so the loop ends.
                                break;
                            }
                            Some(n) => {
                                line.extend_from_slice(&n);
                            }
                        }
                    }
                    if let Some(msg) = self.do_line(&mut line, false) {
                        let m = bcat!(
                            &cnf,
                            b":",
                            lineno,
                            b": (kpathsea) ",
                            msg,
                            b" on line: ",
                            c_str(&line)
                        );
                        self.warning(&m);
                    }
                }
                self.trace_fclose();
            }
        } else {
            let warn = self.env.get(b"KPATHSEA_WARNING");
            if warn.as_deref() != Some(b"0") {
                let m = bcat!(
                    b"kpathsea: configuration file texmf.cnf not found in these directories: ",
                    &cnf_path
                );
                self.warning(&m);
            }
        }
    }

    /// cnf.c `kpathsea_cnf_get`: the value of NAME.progname or NAME from
    /// the configuration files (the first definition read wins).  The first
    /// call reads all texmf.cnf files and the ls-R databases.
    pub(crate) fn cnf_get(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        if self.doing_cnf_init {
            return None;
        }
        if self.cnf_hash.is_none() {
            self.cnf_hash = Some(CHash::new(CNF_HASH_SIZE));
            self.doing_cnf_init = true;
            self.read_all_cnf();
            self.doing_cnf_init = false;
            self.init_db();
        }
        let ctry = cat(&[name, b".", &self.program_name]);
        let r = self.cnf_lookup(&ctry);
        if r.is_some() {
            return r;
        }
        self.cnf_lookup(name)
    }

    fn cnf_lookup(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        let h = self.cnf_hash.as_ref().unwrap();
        if self.compat_def && self.debug_p(crate::debug::KPSE_DEBUG_HASH) {
            let vals: Vec<Vec<u8>> = h.lookup(key).into_iter().map(|v| v.to_vec()).collect();
            let refs: Vec<&[u8]> = vals.iter().map(|v| v.as_slice()).collect();
            self.trace_hash_lookup(key, &refs, false);
            return vals.into_iter().next();
        }
        h.lookup_first(key).map(|v| v.to_vec())
    }
}
