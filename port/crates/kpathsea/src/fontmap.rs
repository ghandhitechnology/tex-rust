//! fontmap.c: texfonts.map font name aliases.

use crate::hash::CHash;
use crate::util::*;
use crate::{FileFormat, Kpse};

const MAP_NAME: &[u8] = b"texfonts.map";
const MAP_HASH_SIZE: u32 = 4001;

/// fontmap.c `token`: next whitespace-delimited token (possibly empty).
fn token(s: &[u8]) -> (usize, usize) {
    let mut i = 0;
    while i < s.len() && isspace(s[i]) {
        i += 1;
    }
    let start = i;
    while i < s.len() && !isspace(s[i]) {
        i += 1;
    }
    (start, i)
}

impl Kpse {
    /// fontmap.c `map_file_parse`.
    fn map_file_parse(&mut self, map: &mut CHash, map_filename: &[u8]) {
        let data = match read_text_file(&self.to_os_path(map_filename)) {
            Ok(d) => {
                self.trace_fopen(map_filename, b"r", true);
                d
            }
            Err(e) => {
                self.trace_fopen(map_filename, b"r", false);
                let m = bcat!(&self.invocation_name, b": ");
                self.err_write(&m);
                self.perror(map_filename, &e);
                std::process::exit(1);
            }
        };
        if let Some(f) = self.record_input.as_mut() {
            f(map_filename);
        }
        let mut lines = Lines::new(&data);
        let mut lineno = 0u32;
        while let Some(orig) = lines.next_line() {
            let mut l: &[u8] = orig;
            // Ignore anything after the last % (or the first @c).
            if let Some(i) = l.iter().rposition(|&c| c == b'%') {
                l = &l[..i];
            } else if let Some(i) = find_sub(l, b"@c", 0) {
                l = &l[..i];
            }
            lineno += 1;
            let mut s = 0;
            while s < l.len() && isspace(l[s]) {
                s += 1;
            }
            let l = &l[s..];
            let (fs, fe) = token(l);
            let filename = l[fs..fe].to_vec();
            let rest = &l[filename.len()..];
            let (as_, ae) = token(rest);
            let alias = rest[as_..ae].to_vec();
            if filename == b"include" {
                let map_path = self.map_path.clone().unwrap_or_default();
                match self.path_search_first(&map_path, &alias, false) {
                    Some(inc) => self.map_file_parse(map, &inc),
                    None => {
                        let m = bcat!(
                            b"kpathsea: ",
                            map_filename,
                            b":",
                            lineno,
                            b": Can't find fontname include file `",
                            &alias,
                            b"'"
                        );
                        self.warning(&m);
                    }
                }
            } else {
                map.insert_normalized(alias, filename);
            }
        }
        self.trace_fclose();
    }

    /// fontmap.c `read_all_maps`.
    fn read_all_maps(&mut self) {
        let map_path = self.init_format(FileFormat::Fontmap);
        self.map_path = Some(map_path.clone());
        let filenames = self.all_path_search(&map_path, MAP_NAME);
        let mut map = CHash::new(MAP_HASH_SIZE);
        for f in filenames {
            self.map_file_parse(&mut map, &f);
        }
        self.map = Some(map);
    }

    /// fontmap.c `kpathsea_fontmap_lookup`: the names KEY is an alias for,
    /// with KEY's suffix (if any) appended to suffix-less names.
    pub fn fontmap_lookup(&mut self, key: &[u8]) -> Option<Vec<Vec<u8>>> {
        let suffix = find_suffix(key).map(|i| key[i..].to_vec());
        if self.map.is_none() {
            self.read_all_maps();
        }
        let mut ret = self.map_lookup(key);
        if ret.is_empty() {
            if suffix.is_some() {
                let base = remove_suffix(key);
                ret = self.map_lookup(&base);
            }
        }
        if ret.is_empty() {
            return None;
        }
        if let Some(suf) = suffix {
            for e in ret.iter_mut() {
                *e = extend_filename(e, &suf);
            }
        }
        Some(ret)
    }

    fn map_lookup(&mut self, key: &[u8]) -> Vec<Vec<u8>> {
        let r: Vec<Vec<u8>> =
            self.map.as_ref().unwrap().lookup(key).into_iter().map(|v| v.to_vec()).collect();
        if self.compat_def && self.debug_p(crate::debug::KPSE_DEBUG_HASH) {
            let refs: Vec<&[u8]> = r.iter().map(|v| v.as_slice()).collect();
            self.trace_hash_lookup(key, &refs, false);
        }
        r
    }
}
