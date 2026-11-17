//! tex-make.c: running mktexpk / mktextfm / mktextex / mktexfmt etc.
//! (WIN32 branch: `$SELFAUTOLOC\<program>` is spawned with its standard
//! output captured), `missfont.log`.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use crate::util::*;
use crate::{FileFormat, Kpse};

impl Kpse {
    /// tex-make.c `set_maketex_mag`.
    fn set_maketex_mag(&mut self) {
        let dpi = self.env.get(b"KPATHSEA_DPI").map(|s| atoi(&s) as u32).unwrap_or(0);
        let bdpi = self.env.get(b"MAKETEX_BASE_DPI").map(|s| atoi(&s) as u32).unwrap_or(0);
        if dpi == 0 || bdpi == 0 {
            // assert() in C.
            return;
        }
        let (_, m) = crate::glyph::magstep_fix(dpi, bdpi);
        let q: String = if m == 0 {
            if bdpi <= 4000 {
                format!("{}+{}/{}", dpi / bdpi, dpi % bdpi, bdpi)
            } else {
                let f = bdpi / 4000;
                let r = bdpi % 4000;
                if f > 1 {
                    if r > 0 {
                        format!("{}+{}/({}*{}+{})", dpi / bdpi, dpi % bdpi, f, (bdpi - r) / f, r)
                    } else {
                        format!("{}+{}/({}*{})", dpi / bdpi, dpi % bdpi, f, bdpi / f)
                    }
                } else {
                    format!("{}+{}/(4000+{})", dpi / bdpi, dpi % bdpi, r)
                }
            }
        } else {
            let (sign, m) = if m < 0 { ("-", -m) } else { ("", m) };
            format!("magstep\\({}{}.{}\\)", sign, m / 2, (m & 1) * 5)
        };
        self.xputenv(b"MAKETEX_MAG", q.as_bytes());
    }

    /// tex-make.c `misstex`: log the failed font-creation command.
    fn misstex(&mut self, format: FileFormat, args: &[Vec<u8>]) {
        if !matches!(
            format,
            FileFormat::Gf | FileFormat::Pk | FileFormat::AnyGlyph | FileFormat::Tfm | FileFormat::Vf
        ) {
            return;
        }
        if self.missfont.is_none() && !self.make_tex_discard_errors {
            let v = self.var_value(b"MISSFONT_LOG");
            let missfont_name: Option<Vec<u8>> = match v {
                None => Some(b"missfont.log".to_vec()),
                Some(s) if s.first() == Some(&b'1') => Some(b"missfont.log".to_vec()),
                Some(s) if s.is_empty() || s[0] == b'0' => None,
                Some(s) => Some(s),
            };
            if missfont_name.is_none() {
                // C goes on with `kpathsea_absolute_p (kpse, NULL, false)`,
                // a NULL dereference: the process dies with an access
                // violation and unflushed stdio buffers are lost.
                self.discard_stderr();
                std::process::exit(0xC000_0005u32 as i32);
            }
            if let Some(name) = missfont_name {
                let outdir = self.env.get(b"TEXMF_OUTPUT_DIRECTORY");
                let first_name = match outdir {
                    Some(d) if !absolute_p(&name, false) => cat(&[&d, b"/", &name]),
                    _ => name.clone(),
                };
                let open = |k: &Kpse, n: &[u8]| {
                    std::fs::OpenOptions::new().create(true).append(true).open(k.to_os_path(n)).ok()
                };
                let mut used = first_name.clone();
                self.missfont = open(self, &first_name);
                if self.missfont.is_none() {
                    if let Some(out) = self.var_value(b"TEXMFOUTPUT") {
                        used = cat(&[&out, b"/", &name]);
                        self.missfont = open(self, &used);
                    }
                }
                if self.missfont.is_some() {
                    let m = bcat!(b"kpathsea: Appending font creation commands to ", &used, b".\n");
                    self.err_write(&m);
                }
            }
        }
        if let Some(f) = self.missfont.as_mut() {
            let mut line = args[0].clone();
            for a in &args[1..] {
                line.push(b' ');
                line.extend_from_slice(a);
            }
            line.push(b'\n');
            let _ = f.write_all(&line);
        }
    }

    /// tex-make.c `maketex` (WIN32 flavour).
    fn maketex(&mut self, format: FileFormat, args: Vec<Vec<u8>>) -> Option<Vec<u8>> {
        let selfloc = match self.var_value(b"SELFAUTOLOC") {
            Some(s) => s,
            None => {
                self.err_write(b"I cannot get SELFAUTOLOC\n");
                self.flush_stderr();
                std::process::exit(100);
            }
        };
        let mut fullbin: Vec<u8> = selfloc.iter().map(|&c| if c == b'/' { b'\\' } else { c }).collect();
        fullbin.push(b'\\');
        fullbin.extend_from_slice(&args[0]);
        if !self.make_tex_discard_errors {
            let mut m = b"\nkpathsea: Running".to_vec();
            for a in &args {
                m.push(b' ');
                m.extend_from_slice(a);
            }
            m.push(b'\n');
            self.err_write(&m);
            let m = bcat!(b"\nThe command name is ", &fullbin, b"\n");
            self.err_write(&m);
        }
        // _spawnvp tries .com, .exe, .bat, .cmd for a name without suffix.
        let mut program = fullbin.clone();
        if find_suffix(xbasename(&fullbin)).is_none() {
            for ext in [&b".com"[..], b".exe", b".bat", b".cmd"] {
                let cand = cat(&[&fullbin, ext]);
                if crate::sys::get_file_attributes_a(&cand).is_ok() {
                    program = cand;
                    break;
                }
            }
        }
        let cp = self.win32_codepage;
        let mut cmd = Command::new(crate::sys::bytes_to_os(cp, &program));
        cmd.args(args[1..].iter().map(|a| crate::sys::bytes_to_os(cp, a)));
        for (k, v) in self.env.changes() {
            let k = crate::sys::bytes_to_os(cp, k);
            match v {
                Some(v) => {
                    cmd.env(k, crate::sys::bytes_to_os(cp, v));
                }
                None => {
                    cmd.env_remove(k);
                }
            }
        }
        cmd.stdin(Stdio::inherit()).stdout(Stdio::piped());
        if self.make_tex_discard_errors {
            cmd.stderr(Stdio::null());
        } else {
            cmd.stderr(Stdio::inherit());
        }
        let mut fnout: Option<Vec<u8>> = None;
        if let Ok(mut child) = cmd.spawn() {
            let mut buf = Vec::new();
            if let Some(mut so) = child.stdout.take() {
                let _ = so.read_to_end(&mut buf);
            }
            let _ = child.wait();
            fnout = Some(text_mode(buf));
        }
        let mut ret = None;
        if let Some(mut f) = fnout {
            while matches!(f.last(), Some(b'\n') | Some(b'\r')) {
                f.pop();
            }
            let len = f.len();
            ret = if len == 0 { None } else { self.readable_file(f.clone()) };
            if ret.is_none() && len > 1 {
                let m = bcat!(b"kpathsea: ", &args[0], b" output `", &f, b"' instead of a filename");
                self.warning(&m);
            }
        }
        match &ret {
            None => self.misstex(format, &args),
            Some(r) => {
                let r = r.clone();
                self.db_insert(&r);
            }
        }
        ret
    }

    /// tex-make.c `kpathsea_make_tex`: try to create BASE in FORMAT with
    /// the format's mktex program, if enabled.
    pub fn make_tex(&mut self, format: FileFormat, base: &[u8]) -> Option<Vec<u8>> {
        if self.format_info[format as usize].type_.is_none() {
            self.init_format(format);
        }
        let (program, enabled, argv) = {
            let f = &self.format_info[format as usize];
            (f.program.clone(), f.program_enabled_p, f.argv.clone().unwrap_or_default())
        };
        if program.is_none() || !enabled {
            return None;
        }
        if base.first() == Some(&b'-') {
            return None;
        }
        for &c in base {
            if !isalnum(c) && c != b'-' && c != b'+' && c != b'_' && c != b'.' && !is_dir_sep(c) {
                return None;
            }
        }
        if matches!(format, FileFormat::Gf | FileFormat::Pk | FileFormat::AnyGlyph) {
            self.set_maketex_mag();
        }
        let mut args: Vec<Vec<u8>> = Vec::with_capacity(argv.len() + 1);
        for a in &argv {
            args.push(self.var_expand(a));
        }
        args.push(base.to_vec());
        self.maketex(format, args)
    }
}
