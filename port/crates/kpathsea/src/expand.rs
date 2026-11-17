//! expand.c (brace / path expansion), kdefault.c (`kpse_expand_default`),
//! tilde.c (`~` expansion) and path-elt.c (path element splitting).

use crate::util::*;
use crate::Kpse;

impl Kpse {
    /// path-elt.c `element (kpse, path, env_p)`: split PATH at separators
    /// that are not inside braces.  An empty PATH yields one empty element.
    pub(crate) fn split_path<'a>(&self, path: &'a [u8], env_p: bool) -> Vec<&'a [u8]> {
        let mut out = Vec::new();
        let n = path.len();
        let mut start = 0usize;
        loop {
            let mut p = start;
            let mut brace_level: i32 = 0;
            while p < n {
                let c = path[p];
                if brace_level == 0 && (if env_p { is_kpse_sep(c) } else { is_dir_sep(c) }) {
                    break;
                }
                if c == b'{' {
                    brace_level += 1;
                } else if c == b'}' {
                    brace_level -= 1;
                } else if self.is_kanji(path, p) {
                    p += 1;
                }
                p += 1;
            }
            let p = p.min(n);
            out.push(&path[start..p]);
            if p >= n {
                break;
            }
            start = p + 1;
        }
        out
    }

    /// `kpathsea_path_element` iteration as a list.
    pub(crate) fn path_elements<'a>(&self, path: &'a [u8]) -> Vec<&'a [u8]> {
        self.split_path(path, true)
    }

    /// expand.c `kpathsea_expand`: variable then tilde expansion.  On
    /// Windows the result is always slashified (tilde.c).
    pub fn expand(&mut self, s: &[u8]) -> Vec<u8> {
        let v = self.var_expand(s);
        self.tilde_expand(v)
    }

    /// tilde.c `kpathsea_tilde_expand` (WIN32: HOMEVAR is USERPROFILE).
    pub(crate) fn tilde_expand(&mut self, name: Vec<u8>) -> Vec<u8> {
        let (prefix, off): (&[u8], usize) =
            if at(&name, 0) == b'!' && at(&name, 1) == b'!' { (b"!!", 2) } else { (b"", 0) };
        let body = &name[off..];
        let mut expansion = if at(body, 0) != b'~' {
            name.clone()
        } else {
            let mut c: usize;
            let home: Vec<u8>;
            if at(body, 1) != 0 && !is_dir_sep(at(body, 1)) {
                c = 2;
                while at(body, c) != 0 && !is_dir_sep(at(body, c)) {
                    if self.is_kanji(body, c) {
                        c += 1;
                    }
                    c += 1;
                }
                let user = &body[1..c.min(body.len())];
                home = self.getpwnam_dir(user).unwrap_or_else(|| b".".to_vec());
            } else {
                c = 1;
                home = self.env.get(b"USERPROFILE").unwrap_or_else(|| b".".to_vec());
            }
            let mut h: &[u8] = &home;
            if is_dir_sep(at(h, 0)) && is_dir_sep(at(h, 1)) {
                h = &h[1..];
            }
            if at(body, c) != 0 {
                let mut q = 0usize;
                while q < h.len() {
                    if is_dir_sep(h[q]) && q + 1 == h.len() {
                        c += 1;
                    } else if self.is_kanji(h, q) {
                        q += 1;
                    }
                    q += 1;
                }
            }
            let rest = if c <= body.len() { &body[c..] } else { &b""[..] };
            cat(&[prefix, h, rest])
        };
        self.slashify(&mut expansion);
        expansion
    }

    /// expand.c `kpathsea_expand_kpse_dot`.
    fn expand_kpse_dot(&mut self, path: Vec<u8>) -> Vec<u8> {
        let kpse_dot = match self.env.get(b"KPSE_DOT") {
            Some(d) => d,
            None => return path,
        };
        let mut ret: Vec<u8> = Vec::new();
        for elt in self.path_elements(&path) {
            if absolute_p(elt, false) || (at(elt, 0) == b'!' && at(elt, 1) == b'!') {
                ret.extend_from_slice(elt);
                ret.push(ENV_SEP);
            } else if elt == b"." {
                ret.extend_from_slice(&kpse_dot);
                ret.push(ENV_SEP);
            } else if at(elt, 0) == b'.' && is_dir_sep(at(elt, 1)) {
                ret.extend_from_slice(&kpse_dot);
                ret.extend_from_slice(&elt[1..]);
                ret.push(ENV_SEP);
            } else if !elt.is_empty() {
                ret.extend_from_slice(&kpse_dot);
                ret.push(DIR_SEP);
                ret.extend_from_slice(elt);
                ret.push(ENV_SEP);
            }
        }
        ret.pop();
        ret
    }

    /// expand.c `brace_expand` (the static one): expand `{a,b}` starting at
    /// `*pos`; on return `*pos` is at the closing brace or end.
    fn brace_expand_inner(&mut self, s: &[u8], pos: &mut usize) -> Vec<Vec<u8>> {
        let n = s.len();
        let mut result: Vec<Vec<u8>> = Vec::new();
        let mut partial: Vec<Vec<u8>> = Vec::new();
        let mut text = *pos;
        let mut p = *pos;
        while p < n && s[p] != b'}' {
            let c = s[p];
            if c == ENV_SEP || c == b',' {
                expand_append(&mut partial, &s[text..p]);
                result.append(&mut partial);
                text = p + 1;
            } else if c == b'{' {
                expand_append(&mut partial, &s[text..p]);
                p += 1;
                let recurse = self.brace_expand_inner(s, &mut p);
                concat_elements(&mut partial, recurse);
                if p >= n || s[p] != b'}' {
                    let m = bcat!(b"kpathsea: ", &s[text..], b": Unmatched {");
                    self.warning(&m);
                    p -= 1;
                }
                text = p + 1;
            } else if c == b'$' {
                if at(s, p + 1) == b'{' {
                    p += 2;
                    while p < n && s[p] != b'}' {
                        p += 1;
                    }
                    if p >= n {
                        // C runs off the end of the string here.
                        break;
                    }
                }
            } else if self.is_kanji(s, p) {
                p += 1;
            }
            p += 1;
        }
        let p = p.min(n);
        let text = text.min(p);
        expand_append(&mut partial, &s[text..p]);
        result.append(&mut partial);
        *pos = p;
        result
    }

    /// expand.c `kpathsea_brace_expand_element`.
    fn brace_expand_element(&mut self, elt: &[u8]) -> Vec<u8> {
        let mut pos = 0usize;
        let expansions = self.brace_expand_inner(elt, &mut pos);
        let mut ret: Vec<u8> = Vec::new();
        for e in expansions {
            let mut x = self.expand(&e);
            if x != e {
                x = self.brace_expand_element(&x);
            }
            ret.extend_from_slice(&x);
            ret.push(ENV_SEP);
        }
        ret.pop();
        ret
    }

    /// expand.c `kpathsea_brace_expand`: variable and brace expansion of
    /// every element of PATH, then `KPSE_DOT` handling.
    pub fn brace_expand(&mut self, path: impl AsRef<[u8]>) -> Vec<u8> {
        let xpath = self.var_expand(path.as_ref());
        let mut ret: Vec<u8> = Vec::new();
        let elts: Vec<Vec<u8>> = self.path_elements(&xpath).iter().map(|e| e.to_vec()).collect();
        for elt in elts {
            let e = self.brace_expand_element(&elt);
            ret.extend_from_slice(&e);
            ret.push(ENV_SEP);
        }
        if !ret.is_empty() {
            ret.pop();
        }
        self.expand_kpse_dot(ret)
    }

    /// expand.c `kpathsea_path_expand`: full expansion of a path, listing
    /// only existing directories (including `//` subdirectories).
    pub fn path_expand(&mut self, path: impl AsRef<[u8]>) -> Vec<u8> {
        let mut zpath = path.as_ref().to_vec();
        self.slashify(&mut zpath);
        let xpath = self.brace_expand(&zpath);
        let mut ret: Vec<u8> = Vec::new();
        let elts: Vec<Vec<u8>> = self.path_elements(&xpath).iter().map(|e| e.to_vec()).collect();
        for elt in elts {
            let e: &[u8] = if at(&elt, 0) == b'!' && at(&elt, 1) == b'!' { &elt[2..] } else { &elt };
            if let Some(idx) = self.element_dirs(e) {
                let dirs: Vec<Vec<u8>> =
                    self.elt_cache[idx].dirs.iter().map(|d| d.name.clone()).collect();
                for thedir in dirs {
                    let dirlen = thedir.len();
                    if dirlen == 1
                        || (dirlen == 3 && name_begins_with_device(&thedir) && is_dir_sep(thedir[2]))
                    {
                        ret.extend_from_slice(&thedir);
                        ret.push(ENV_SEP);
                    } else {
                        ret.extend_from_slice(&thedir);
                        let l = ret.len();
                        ret[l - 1] = ENV_SEP;
                    }
                }
            }
        }
        if !ret.is_empty() {
            ret.pop();
        }
        ret
    }
}

/// expand.c `expand_append`.
fn expand_append(partial: &mut Vec<Vec<u8>>, text: &[u8]) {
    concat_elements(partial, vec![text.to_vec()]);
}

/// str-list.c `str_list_concat_elements`: every TARGET element followed by
/// every MORE element, TARGET index varying fastest.
fn concat_elements(target: &mut Vec<Vec<u8>>, more: Vec<Vec<u8>>) {
    if more.is_empty() {
        return;
    }
    if target.is_empty() {
        *target = more;
        return;
    }
    let mut new_list = Vec::with_capacity(target.len() * more.len());
    for m in &more {
        for t in target.iter() {
            new_list.push(cat(&[t, m]));
        }
    }
    *target = new_list;
}

/// kdefault.c `kpathsea_expand_default`: replace an extra `;` (leading,
/// trailing or doubled) in PATH with FALLBACK.
pub(crate) fn expand_default(path: Option<&[u8]>, fallback: &[u8]) -> Vec<u8> {
    let path = match path {
        None => return fallback.to_vec(),
        Some(p) if p.is_empty() => return fallback.to_vec(),
        Some(p) => p,
    };
    if path[0] == ENV_SEP {
        return if path.len() == 1 { fallback.to_vec() } else { cat(&[fallback, path]) };
    }
    if *path.last().unwrap() == ENV_SEP {
        return cat(&[path, fallback]);
    }
    for i in 0..path.len() - 1 {
        if path[i] == ENV_SEP && path[i + 1] == ENV_SEP {
            return cat(&[&path[..=i], fallback, &path[i + 1..]]);
        }
    }
    path.to_vec()
}
