//! tex-file.c `kpathsea_name_ok` and friends: `openin_any` /
//! `openout_any` checks, plus the WIN32 `executable_filep` refusal.

use crate::util::*;
use crate::Kpse;

#[derive(Clone, Copy, PartialEq)]
enum OkType {
    Reading,
    Writing,
}

/// tex-file.c `abs_fname_ok`.
fn abs_fname_ok(fname: &[u8], checkdir: Option<&[u8]>) -> bool {
    match checkdir {
        Some(d) if !d.is_empty() => {
            fname.starts_with(d) && (fname.len() == d.len() || is_dir_sep(fname[d.len()]))
        }
        _ => false,
    }
}

impl Kpse {
    fn name_ok(
        &mut self,
        fname: &[u8],
        check_var: &[u8],
        default_choice: &[u8],
        action: OkType,
        silent: bool,
        extended: bool,
    ) -> bool {
        if action == OkType::Reading {
            return true;
        }
        let open_choice = self.var_value(check_var).unwrap_or_else(|| default_choice.to_vec());
        let c0 = at(&open_choice, 0);
        if c0 == b'a' || c0 == b'y' || c0 == b'1' {
            return true;
        }
        let expanded = self.expand(fname);
        // (Windows: no check for dot files.)
        if c0 == b'r' || c0 == b'n' || c0 == b'0' {
            return true;
        }
        let mut ok = true;
        if absolute_p(&expanded, false) {
            let texmfoutdir = self.env.get(b"TEXMF_OUTPUT_DIRECTORY");
            if !abs_fname_ok(&expanded, texmfoutdir.as_deref()) {
                let texmfoutput = self.var_value(b"TEXMFOUTPUT");
                if !abs_fname_ok(&expanded, texmfoutput.as_deref()) {
                    if extended {
                        let texmfvar = self.var_value(b"TEXMFVAR");
                        if !abs_fname_ok(&expanded, texmfvar.as_deref()) {
                            let texmfsysvar = self.var_value(b"TEXMFSYSVAR");
                            if !abs_fname_ok(&expanded, texmfsysvar.as_deref()) {
                                ok = false;
                            }
                        }
                    } else {
                        ok = false;
                    }
                }
            }
        }
        if ok {
            if at(fname, 0) == b'.' && at(fname, 1) == b'.' && is_dir_sep(at(fname, 2)) {
                ok = false;
            } else {
                let mut from = 0;
                while let Some(d) = find_sub(fname, b"..", from) {
                    if is_dir_sep(at(fname, d + 2)) && d > 0 && is_dir_sep(fname[d - 1]) {
                        ok = false;
                        break;
                    }
                    from = d + 2;
                }
            }
        }
        if ok {
            return true;
        }
        if !silent {
            let m = bcat!(
                b"\n",
                &self.invocation_name,
                b": Not ",
                if action == OkType::Reading { "reading from" } else { "writing to" },
                b" ",
                fname,
                b" (",
                check_var,
                b" = ",
                &open_choice,
                b"; ",
                if extended { "" } else { "no " },
                b"extended check).\n"
            );
            self.err_write(&m);
        }
        false
    }

    /// tex-file.c `executable_filep` (WIN32).
    fn executable_filep(&mut self, fname: &[u8], silent: bool) -> bool {
        let p = self.var_value(b"openout_any");
        if let Some(p) = p {
            if at(&p, 0) == b'p' {
                let mut fnp = fname;
                if let Some(i) = fnp.iter().rposition(|&c| c == b'/') {
                    fnp = &fnp[i + 1..];
                }
                if let Some(i) = fnp.iter().rposition(|&c| c == b'\\') {
                    fnp = &fnp[i + 1..];
                }
                let mut base = fnp.to_ascii_lowercase();
                while let Some(&c) = base.last() {
                    if c == b'.' || c == b' ' || c == b'\t' {
                        base.pop();
                    } else {
                        break;
                    }
                }
                if let Some(d) = base.iter().rposition(|&c| c == b'.') {
                    let q = &base[d..];
                    if self.suffixlist.iter().any(|s| s.as_slice() == q) {
                        if !silent {
                            let m = bcat!(b"\n", fname, b": Forbidden to open for writing\n");
                            self.err_write(&m);
                        }
                        return true;
                    }
                }
            }
        }
        false
    }

    fn out_name_ok_1(&mut self, fname: &[u8], silent: bool, extended: bool) -> bool {
        if self.executable_filep(fname, silent) {
            return false;
        }
        self.name_ok(fname, b"openout_any", b"p", OkType::Writing, silent, extended)
    }

    /// `kpathsea_in_name_ok`: as of TeX Live 2026 always true.
    pub fn in_name_ok(&mut self, fname: impl AsRef<[u8]>) -> bool {
        self.name_ok(fname.as_ref(), b"openin_any", b"a", OkType::Reading, false, false)
    }
    /// `kpathsea_in_name_ok_silent`.
    pub fn in_name_ok_silent(&mut self, fname: impl AsRef<[u8]>) -> bool {
        self.name_ok(fname.as_ref(), b"openin_any", b"a", OkType::Reading, true, false)
    }
    /// `kpathsea_in_name_ok_extended`.
    pub fn in_name_ok_extended(&mut self, fname: impl AsRef<[u8]>) -> bool {
        self.name_ok(fname.as_ref(), b"openin_any", b"a", OkType::Reading, false, true)
    }
    /// `kpathsea_in_name_ok_silent_extended`.
    pub fn in_name_ok_silent_extended(&mut self, fname: impl AsRef<[u8]>) -> bool {
        self.name_ok(fname.as_ref(), b"openin_any", b"a", OkType::Reading, true, true)
    }
    /// `kpathsea_out_name_ok`.
    pub fn out_name_ok(&mut self, fname: impl AsRef<[u8]>) -> bool {
        self.out_name_ok_1(fname.as_ref(), false, false)
    }
    /// `kpathsea_out_name_ok_silent`.
    pub fn out_name_ok_silent(&mut self, fname: impl AsRef<[u8]>) -> bool {
        self.out_name_ok_1(fname.as_ref(), true, false)
    }
    /// `kpathsea_out_name_ok_extended`.
    pub fn out_name_ok_extended(&mut self, fname: impl AsRef<[u8]>) -> bool {
        self.out_name_ok_1(fname.as_ref(), false, true)
    }
    /// `kpathsea_out_name_ok_silent_extended`.
    pub fn out_name_ok_silent_extended(&mut self, fname: impl AsRef<[u8]>) -> bool {
        self.out_name_ok_1(fname.as_ref(), true, true)
    }
}
