//! variable.c: `$VAR` / `${VAR}` expansion and `kpse_var_value`.

use crate::debug::KPSE_DEBUG_VARS;
use crate::util::*;
use crate::Kpse;

#[inline]
fn is_var_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

impl Kpse {
    /// `kpathsea_var_value`: VAR.progname, VAR_progname, VAR from the
    /// environment, else texmf.cnf; the value is `$`- and `~`-expanded.
    pub fn var_value(&mut self, var: impl AsRef<[u8]>) -> Option<Vec<u8>> {
        let var = var.as_ref();
        let mut value = self.env.get(&cat(&[var, b".", &self.program_name]));
        if value.as_ref().map_or(true, |v| v.is_empty()) {
            value = self.env.get(&cat(&[var, b"_", &self.program_name]));
        }
        if value.as_ref().map_or(true, |v| v.is_empty()) {
            value = self.env.get(var);
        }
        if value.as_ref().map_or(true, |v| v.is_empty()) {
            value = self.cnf_get(var);
        }
        let ret = value.map(|v| self.expand(&v));
        if self.debug_p(KPSE_DEBUG_VARS) {
            let m = bcat!(b"variable: ", var, b" = ", or_nil(ret.as_deref()), b"\n");
            self.debugf(&m);
        }
        ret
    }

    /// `kpse_var_value` returning a lossily-decoded `String`.
    pub fn var_value_str(&mut self, var: &str) -> Option<String> {
        self.var_value(var.as_bytes()).map(|v| crate::lossy(&v))
    }

    /// variable.c `expanding`.
    fn expanding(&mut self, var: &[u8], xp: bool) {
        for e in self.expansions.iter_mut() {
            if e.0 == var {
                e.1 = xp;
                return;
            }
        }
        self.expansions.push((var.to_vec(), xp));
    }

    /// variable.c `expanding_p`.
    fn expanding_p(&self, var: &[u8]) -> bool {
        self.expansions.iter().find(|e| e.0 == var).map_or(false, |e| e.1)
    }

    /// variable.c `expand`: append the value of VAR to EXPANSION; return
    /// whether VAR was defined.
    fn expand_var_into(&mut self, expansion: &mut Vec<u8>, var: &[u8]) -> bool {
        if self.expanding_p(var) {
            let m = bcat!(b"kpathsea: variable `", var, b"' references itself (eventually)");
            self.warning(&m);
            return false;
        }
        let mut value = self.env.get(&cat(&[var, b"_", &self.program_name]));
        if value.as_ref().map_or(true, |v| v.is_empty()) {
            value = self.env.get(var);
        }
        if value.as_ref().map_or(true, |v| v.is_empty()) {
            value = self.cnf_get(var);
        }
        match value {
            Some(v) => {
                self.expanding(var, true);
                let tmp = self.expand(&v);
                self.expanding(var, false);
                expansion.extend_from_slice(&tmp);
                true
            }
            None => false,
        }
    }

    /// `kpathsea_var_expand`: expand `$VAR`, `${VAR}`; leave unknown
    /// variables literally.
    pub fn var_expand(&mut self, src: impl AsRef<[u8]>) -> Vec<u8> {
        let src = c_str(src.as_ref());
        if !src.contains(&b'$') {
            return src.to_vec();
        }
        let mut expansion: Vec<u8> = Vec::with_capacity(src.len() + 16);
        let n = src.len();
        let mut s = 0usize;
        while s < n {
            if src[s] == b'$' {
                s += 1;
                let c = at(src, s);
                if is_var_char(c) {
                    let mut var_end = s;
                    loop {
                        var_end += 1;
                        if !is_var_char(at(src, var_end)) {
                            break;
                        }
                    }
                    var_end -= 1;
                    let var = src[s..=var_end].to_vec();
                    if !self.expand_var_into(&mut expansion, &var) {
                        expansion.extend_from_slice(&src[s - 1..=var_end]);
                    }
                    s = var_end;
                } else if c == b'{' {
                    s += 1;
                    let mut var_end = s;
                    while var_end < n && src[var_end] != b'}' {
                        if self.is_kanji(src, var_end) {
                            var_end += 1;
                        }
                        var_end += 1;
                    }
                    if var_end >= n {
                        let m = bcat!(b"kpathsea: ", src, b": No matching } for ${");
                        self.warning(&m);
                        // s = var_end - 1; the loop increment then hits NUL.
                        break;
                    } else {
                        let var = src[s..var_end].to_vec();
                        self.expand_var_into(&mut expansion, &var);
                        s = var_end;
                    }
                } else {
                    let m = bcat!(
                        b"kpathsea: ",
                        src,
                        b": Unrecognized variable construct `$",
                        [c],
                        b"'"
                    );
                    self.warning(&m);
                    if c == 0 {
                        // "$" at the very end: C appends "$" plus the NUL and
                        // then reads past the string; the visible result is a
                        // trailing "$".
                        expansion.push(b'$');
                        break;
                    }
                    expansion.push(b'$');
                    expansion.push(c);
                }
            } else {
                expansion.push(src[s]);
            }
            s += 1;
        }
        expansion
    }

    /// `kpse_var_expand` returning a lossily-decoded `String`.
    pub fn var_expand_str(&mut self, src: &str) -> String {
        crate::lossy(&self.var_expand(src.as_bytes()))
    }
}
