//! Port of web2c's lib/texmfmp.c (+ setupvar.c, uexit.c, printversion.c, usage.c) for the
//! pdfTeX, WIN32 configuration: startup, option parsing, terminal input, TCX files,
//! dates, shell escape, source specials and the pdfTeX string helpers.

use super::*;
use crate::gen::globals::Globals;

pub const BANNER: &str = "This is pdfTeX, Version 3.141592653-2.6-1.40.29";
pub const VERSION_STRING: &str = " (TeX Live 2026)";
pub const KPSE_VERSION: &str = "kpathsea version 6.4.2";
const INI_PROGRAM: &str = "pdfinitex";
const VIR_PROGRAM: &str = "pdfvirtex";

const HELP: &[&str] = &[
    "Usage: pdftex [OPTION]... [TEXNAME[.tex]] [COMMANDS]",
    "   or: pdftex [OPTION]... \\FIRST-LINE",
    "   or: pdftex [OPTION]... &FMT ARGS",
    "  Run pdfTeX on TEXNAME, usually creating TEXNAME.pdf.",
    "  Any remaining COMMANDS are processed as pdfTeX input, after TEXNAME is read.",
    "  If the first line of TEXNAME is %&FMT, and FMT is an existing .fmt file,",
    "  use it.  Else use `NAME.fmt', where NAME is the program invocation name,",
    "  most commonly `pdftex'.",
    "",
    "  Alternatively, if the first non-option argument begins with a backslash,",
    "  interpret all non-option arguments as a line of pdfTeX input.",
    "",
    "  Alternatively, if the first non-option argument begins with a &, the",
    "  next word is taken as the FMT to read, overriding all else.  Any",
    "  remaining arguments are processed as above.",
    "",
    "  If no arguments or options are specified, prompt for input.",
    "",
    "-cnf-line=STRING        parse STRING as a configuration file line",
    "-draftmode              switch on draft mode (generates no output PDF)",
    "-enc                    enable encTeX extensions such as \\mubyte",
    "-etex                   enable e-TeX extensions",
    "[-no]-file-line-error   disable/enable file:line:error style messages",
    "-fmt=FMTNAME            use FMTNAME instead of program name or a %& line",
    "-halt-on-error          stop processing at the first error",
    "-ini                    be pdfinitex, for dumping formats; this is implicitly",
    "                          true if the program name is `pdfinitex'",
    "-interaction=STRING     set interaction mode (STRING=batchmode/nonstopmode/",
    "                          scrollmode/errorstopmode)",
    "-jobname=STRING         set the job name to STRING",
    "-kpathsea-debug=NUMBER  set path searching debugging flags according to",
    "                          the bits of NUMBER",
    "[-no]-mktex=FMT         disable/enable mktexFMT generation (FMT=tex/tfm/pk)",
    "-mltex                  enable MLTeX extensions such as \\charsubdef",
    "-output-comment=STRING  use STRING for DVI file comment instead of date",
    "                          (no effect for PDF)",
    "-output-directory=DIR   use existing DIR as the directory to write files in",
    "-output-format=FORMAT   use FORMAT for job output; FORMAT is `dvi' or `pdf'",
    "[-no]-parse-first-line  disable/enable parsing of first line of input file",
    "-progname=STRING        set program (and fmt) name to STRING",
    "-recorder               enable filename recorder",
    "[-no]-shell-escape      disable/enable \\write18{SHELL COMMAND}",
    "-shell-restricted       enable restricted \\write18",
    "-src-specials           insert source specials into the DVI file",
    "-src-specials=WHERE     insert source specials in certain places of",
    "                          the DVI file. WHERE is a comma-separated value",
    "                          list: cr display hbox math par parend vbox",
    "-synctex=NUMBER         generate SyncTeX data for previewers according to",
    "                          bits of NUMBER (`man synctex' for details)",
    "-translate-file=TCXNAME use the TCX file TCXNAME",
    "-8bit                   make all characters printable by default",
    "-help                   display this help and exit",
    "-version                output version information and exit",
    "",
    "pdfTeX home page: <https://pdftex.org>",
];

// ------------------------------------------------------------------ fatal errors / exit
/// kpathsea FATAL: "<prog>: fatal: <msg>.\n" and exit(1).
pub fn fatal(msg: &str) -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let prog = PROGNAME.with(|p| p.borrow().clone());
    let _ = write!(std::io::stderr(), "{}: fatal: {}.\r\n", prog, msg);
    std::process::exit(1);
}

pub fn fatal_perror(what: &str) -> ! {
    let err = std::io::Error::last_os_error();
    let prog = PROGNAME.with(|p| p.borrow().clone());
    eprint!("{}: {}: {}\r\n", prog, what, err);
    std::process::exit(1);
}

pub fn warning(msg: &str) {
    let prog = PROGNAME.with(|p| p.borrow().clone());
    eprint!("{}: warning: {}.\r\n", prog, msg);
}

thread_local! {
    pub static PROGNAME: std::cell::RefCell<String> = std::cell::RefCell::new("pdftex".to_string());
}

// ------------------------------------------------------------------ cpascal.h helpers
/// lib/zround.c
#[inline]
pub fn zround(r: f64) -> i32 {
    if r > 2147483647.0 {
        2147483647
    } else if r < -2147483647.0 {
        -2147483647
    } else if r >= 0.0 {
        (r + 0.5) as i32
    } else {
        (r - 0.5) as i32
    }
}

/// cpascal.h abs() applied to a real: ((integer)(x) >= 0 ? (integer)(x) : (integer)-(x))
#[inline]
pub fn w2c_abs_real(x: f64) -> i32 {
    if (x as i32) >= 0 {
        x as i32
    } else {
        (-x) as i32
    }
}

#[inline]
pub fn w2c_fabs(x: f64) -> f64 {
    if x >= 0.0 {
        x
    } else {
        -x
    }
}

pub fn w2c_strcpy(dst: Ptr<u8>, src: Ptr<u8>) {
    let b = src.cstr_bytes().to_vec();
    for (i, c) in b.iter().enumerate() {
        *ix(dst, i as i32) = *c;
    }
    *ix(dst, b.len() as i32) = 0;
}

// ------------------------------------------------------------------ shell escape
fn isspace(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

/// texmfmp.c shell_cmd_is_allowed (WIN32 quoting). Returns (allow, safecmd).
pub fn shell_cmd_is_allowed(cmd: &str, cmdlist: Option<&[String]>) -> (i32, Option<String>) {
    let b = cmd.as_bytes();
    let mut i = 0;
    while i < b.len() && isspace(b[i]) {
        i += 1;
    }
    let mut j = i;
    while j < b.len() && !isspace(b[j]) {
        j += 1;
    }
    let cmdname = &cmd[i..j];
    let allow = cmdlist.map_or(false, |l| l.iter().any(|c| c == cmdname));
    if !allow {
        return (0, None);
    }
    const QUOTE: u8 = b'"';
    let mut d: Vec<u8> = Vec::with_capacity(2 * b.len() + 3);
    let mut s = 0;
    while s < b.len() && isspace(b[s]) {
        s += 1;
    }
    while s < b.len() && !isspace(b[s]) {
        d.push(b[s]);
        s += 1;
    }
    let mut pre = true;
    while s < b.len() {
        if b[s] == b'\'' {
            return (-1, None);
        }
        if b[s] == b'"' {
            if !pre {
                if s > 0 && b[s - 1] == b'=' {
                    let l = d.len();
                    d[l - 1] = QUOTE;
                    d.push(b'=');
                } else {
                    d.push(QUOTE);
                }
            }
            pre = false;
            d.push(QUOTE);
            s += 1;
            while s >= b.len() || b[s] != b'"' {
                if s >= b.len() || b[s] == b'\'' {
                    return (-1, None);
                }
                d.push(b[s]);
                s += 1;
            }
            s += 1;
            if s < b.len() && !isspace(b[s]) {
                return (-1, None);
            }
        } else if pre && !isspace(b[s]) {
            pre = false;
            d.push(QUOTE);
            d.push(b[s]);
            s += 1;
        } else if !pre && isspace(b[s]) {
            pre = true;
            d.push(QUOTE);
            d.push(b[s]);
            s += 1;
        } else {
            d.push(b[s]);
            s += 1;
        }
    }
    if !pre {
        d.push(QUOTE);
    }
    (2, Some(bytes_to_string(&d)))
}

// ------------------------------------------------------------------ getopt_long_only
struct Opt {
    name: &'static str,
    has_arg: u8, // 0 none, 1 required, 2 optional
}

const LONG_OPTIONS: &[Opt] = &[
    Opt { name: "fmt", has_arg: 1 },
    Opt { name: "efmt", has_arg: 1 },
    Opt { name: "cnf-line", has_arg: 1 },
    Opt { name: "help", has_arg: 0 },
    Opt { name: "ini", has_arg: 0 },
    Opt { name: "interaction", has_arg: 1 },
    Opt { name: "halt-on-error", has_arg: 0 },
    Opt { name: "kpathsea-debug", has_arg: 1 },
    Opt { name: "progname", has_arg: 1 },
    Opt { name: "recorder", has_arg: 0 },
    Opt { name: "version", has_arg: 0 },
    Opt { name: "mltex", has_arg: 0 },
    Opt { name: "enc", has_arg: 0 },
    Opt { name: "etex", has_arg: 0 },
    Opt { name: "output-comment", has_arg: 1 },
    Opt { name: "draftmode", has_arg: 0 },
    Opt { name: "output-format", has_arg: 1 },
    Opt { name: "shell-escape", has_arg: 0 },
    Opt { name: "no-shell-escape", has_arg: 0 },
    Opt { name: "enable-write18", has_arg: 0 },
    Opt { name: "disable-write18", has_arg: 0 },
    Opt { name: "shell-restricted", has_arg: 0 },
    Opt { name: "debug-format", has_arg: 0 },
    Opt { name: "src-specials", has_arg: 2 },
    Opt { name: "synctex", has_arg: 1 },
    Opt { name: "file-line-error-style", has_arg: 0 },
    Opt { name: "no-file-line-error-style", has_arg: 0 },
    Opt { name: "file-line-error", has_arg: 0 },
    Opt { name: "no-file-line-error", has_arg: 0 },
    Opt { name: "jobname", has_arg: 1 },
    Opt { name: "output-directory", has_arg: 1 },
    Opt { name: "parse-first-line", has_arg: 0 },
    Opt { name: "no-parse-first-line", has_arg: 0 },
    Opt { name: "translate-file", has_arg: 1 },
    Opt { name: "default-translate-file", has_arg: 1 },
    Opt { name: "8bit", has_arg: 0 },
    Opt { name: "mktex", has_arg: 1 },
    Opt { name: "no-mktex", has_arg: 1 },
];

/// Normalize quoting of a filename (non-XeTeX variant): quote only if there is a space.
fn normalize_quotes(name: &str, mesg: &str) -> String {
    let must_quote = name.contains(' ');
    let mut quoted = false;
    let mut ret = String::new();
    if must_quote {
        ret.push('"');
    }
    for c in name.chars() {
        if c == '"' {
            quoted = !quoted;
        } else {
            ret.push(c);
        }
    }
    if must_quote {
        ret.push('"');
    }
    if quoted {
        eprint!("! Unbalanced quotes in {} {}\r\n", mesg, name);
        std::process::exit(1);
    }
    ret
}

impl Globals {
    // ------------------------------------------------------------------ main
    /// texmfmp.c main(): set up, run the Pascal main body.
    pub fn run_main(&mut self, mut av: Vec<String>) -> ! {
        if av.is_empty() {
            av.push("pdftex".into());
        }
        // WIN32: av[0] = kpse_program_basename(av[0])
        let base = av[0].rsplit(|c| c == '/' || c == '\\').next().unwrap_or(&av[0]).to_string();
        let base = if base.to_ascii_lowercase().ends_with(".exe") { base[..base.len() - 4].to_string() } else { base };
        av[0] = base;
        let n = av.len();
        if n > 1 {
            let last = &av[n - 1];
            let lb = last.as_bytes();
            if lb.len() > 2 && lb[0].is_ascii_alphabetic() && lb[1] == b':' && lb[2] == b'\\' {
                av[n - 1] = last.replace('\\', "/");
            }
        }
        self.maininit(av);
        crate::rt::phase::mark("maininit");
        self.mainbody();
        self.x_uexit(0);
    }

    pub fn maininit(&mut self, av: Vec<String>) {
        self.rt.argv = av;
        self.interactionoption = 4;
        self.synctexoption = i32::MAX;
        self.rt.files.kpse = Some(kpse::Kpse::new(&self.rt.argv[0]));
        crate::rt::phase::mark("kpathsea init");
        PROGNAME.with(|p| *p.borrow_mut() = self.rt.kpse().invocation_name().to_string());
        // kpse_record_input = recorder_record_input; kpse_record_output = recorder_record_output
        {
            let pn = self.rt.kpse().program_name();
            self.rt.recorder.borrow_mut().progname = pn.into_bytes();
            let ri = self.rt.recorder.clone();
            let ro = self.rt.recorder.clone();
            let k = &mut self.rt.kpse().k;
            k.set_record_input(Some(Box::new(move |f: &[u8]| ri.borrow_mut().record(b"INPUT", f))));
            k.set_record_output(Some(Box::new(move |f: &[u8]| ro.borrow_mut().record(b"OUTPUT", f))));
        }
        // command_line_encoding: Rust gives us UTF-8 arguments already.
        self.parse_options();
        if self.rt.recorder.borrow().enabled {
            // WIN32: texmf.cnf is recorded here because kpathsea started before the options
            for p in self.rt.kpse().find_file_all("texmf.cnf", kpse::CNF_FORMAT, false) {
                self.recorder_record_input(&p);
            }
        }
        if self.rt.user_progname.is_none() {
            if !self.dumpname.is_null() {
                self.rt.user_progname = Some(bytes_to_string(self.dumpname.cstr_bytes()));
            }
        }
        if let Some(p) = self.rt.user_progname.clone() {
            self.rt.kpse().reset_program_name(&p);
            self.rt.recorder.borrow_mut().progname = p.into_bytes();
        }
        self.rt.kpse().xputenv("engine", "pdftex");
        for line in std::mem::take(&mut self.rt.user_cnf_lines) {
            self.rt.kpse().add_cnf_line(&line);
        }
        let mut main_input_file = self.get_input_file_name();
        if main_input_file.is_none() {
            let argc = self.rt.argv.len();
            let name = self.rt.argv[argc - 1].clone();
            if argc > 1 && !name.is_empty() && !name.starts_with('-') && !name.starts_with('&') && !name.starts_with('\\') {
                let nb = name.as_bytes();
                let mut name2 = name.clone();
                if nb.len() > 2 && nb[0].is_ascii_alphabetic() && nb[1] == b':' && nb[2] == b'\\' {
                    name2 = name2.replace('\\', "/");
                }
                let norm = normalize_quotes(&name2, "argument");
                let quoted = norm.starts_with('"');
                let lookup = if quoted { norm[1..norm.len() - 1].to_string() } else { norm.clone() };
                main_input_file = self.rt.kpse().find_file(lookup.as_bytes(), kpse::TEX_FORMAT, false);
                self.rt.argv[argc - 1] = norm;
            }
        }
        if self.filelineerrorstylep < 0 {
            self.filelineerrorstylep = 0;
        } else if self.filelineerrorstylep == 0 {
            self.filelineerrorstylep = self.texmf_yesno_str("file_line_error_style") as i32;
        }
        if self.parsefirstlinep < 0 {
            self.parsefirstlinep = 0;
        } else if self.parsefirstlinep == 0 {
            self.parsefirstlinep = self.texmf_yesno_str("parse_first_line") as i32;
        }
        if self.parsefirstlinep != 0 && (self.dumpname.is_null() || self.translatefilename.is_null()) {
            let mif = main_input_file.clone();
            self.parse_first_line(mif.as_deref());
        }
        if self.translatefilename.is_null() {
            if let Some(d) = &self.rt.default_translate_filename {
                self.translatefilename = Ptr::<u8>::from_bytes(d.as_bytes());
            }
        }
        if self.readyalready != 314159 {
            let mut virversion = false;
            let prog = self.rt.kpse().program_name().to_ascii_lowercase();
            if prog == INI_PROGRAM {
                self.iniversion = true;
            } else if prog == VIR_PROGRAM {
                virversion = true;
            } else if prog == "initex" {
                self.iniversion = true;
            } else if prog == "virtex" {
                virversion = true;
            } else if prog == "mltex" {
                self.mltexp = true;
            }
            if main_input_file.is_none() {
                if let Some(a1) = self.rt.argv.get(1) {
                    if let Some(rest) = a1.strip_prefix('&') {
                        self.dumpname = Ptr::<u8>::from_bytes(rest.as_bytes());
                    }
                }
            }
            if self.dumpname.is_null() {
                let d = if virversion { "plain".to_string() } else { self.rt.kpse().program_name().to_string() };
                self.dumpname = Ptr::<u8>::from_bytes(d.as_bytes());
            }
        }
        if !self.iniversion {
            if self.mltexp {
                self.w_cstr_err("-mltex only works with -ini\n");
            }
            if self.enctexp {
                self.w_cstr_err("-enc only works with -ini\n");
            }
            if self.etexp {
                self.w_cstr_err("-etex only works with -ini\n");
            }
        }
        let dump_name = bytes_to_string(self.dumpname.cstr_bytes());
        let with_ext = if dump_name.len() > 4 && dump_name[dump_name.len() - 4..].eq_ignore_ascii_case(".fmt") {
            dump_name.clone()
        } else {
            format!("{}.fmt", dump_name)
        };
        let dv = format!(" {}", with_ext);
        self.TEXformatdefault = Ptr::<u8>::from_bytes(dv.as_bytes());
        self.formatdefaultlength = with_ext.len() as i32;
        self.rt.kpse().set_program_enabled(kpse::TFM_FORMAT, true, kpse::SRC_COMPILE);
        self.rt.kpse().set_program_enabled(kpse::TEX_FORMAT, false, kpse::SRC_COMPILE);
        self.rt.kpse().set_program_enabled(kpse::FMT_FORMAT, true, kpse::SRC_COMPILE);
        self.init_shell_escape();
        if self.outputcomment.is_null() {
            if let Some(v) = self.rt.kpse().var_value("output_comment") {
                self.outputcomment = Ptr::<u8>::from_bytes(v.as_bytes());
            }
        }
        self.versionstring = Ptr::<u8>::cstr_lit(b" (TeX Live 2026)\0");
    }

    fn get_input_file_name(&mut self) -> Option<Vec<u8>> {
        let optind = self.rt.optind;
        let arg = self.rt.argv.get(optind)?.clone();
        if arg.starts_with('&') || arg.starts_with('\\') {
            return None;
        }
        let mut a = arg.clone();
        let ab = a.as_bytes();
        if ab.len() > 2 && ab[0].is_ascii_alphabetic() && ab[1] == b':' && ab[2] == b'\\' {
            a = a.replace('\\', "/");
        }
        let name = normalize_quotes(&a, "argument");
        let quoted = name.starts_with('"');
        let lookup = if quoted { name[1..name.len() - 1].to_string() } else { name.clone() };
        let found = self.rt.kpse().find_file(lookup.as_bytes(), kpse::TEX_FORMAT, false);
        self.rt.argv[optind] = name;
        found
    }

    fn parse_first_line(&mut self, filename: Option<&[u8]>) {
        let Some(filename) = filename else { return };
        let path = self.os_path(filename);
        let Ok(data) = std::fs::read(path) else { return };
        let end = data.iter().position(|&c| c == b'\n').unwrap_or(data.len());
        let mut line = data[..end].to_vec();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if line.len() < 2 || line[0] != b'%' || line[1] != b'&' {
            return;
        }
        let mut s = 2;
        while s < line.len() && (line[s] == b' ' || line[s] == b'\t') {
            s += 1;
        }
        let mut parts: Vec<String> = Vec::new();
        while s < line.len() && parts.len() != 3 {
            let st = s;
            while s < line.len() && line[s] != b' ' {
                s += 1;
            }
            parts.push(bytes_to_string(&line[st..s]));
            while s < line.len() && line[s] == b' ' {
                s += 1;
            }
        }
        let mut pi = 0;
        if pi < parts.len() && !parts[pi].starts_with('-') {
            if self.dumpname.is_null() {
                let f_name = format!("{}.fmt", parts[0]);
                if let Some(d) = self.rt.kpse().find_file(f_name.as_bytes(), kpse::FMT_FORMAT, false) {
                    let dp = self.os_path(&d);
                    if std::fs::File::open(&dp).is_ok() {
                        self.dumpname = Ptr::<u8>::from_bytes(parts[0].as_bytes());
                        let p = parts[0].clone();
                        self.rt.kpse().reset_program_name(&p);
                        self.rt.recorder.borrow_mut().progname = p.into_bytes();
                        self.dumpline = true;
                    }
                }
            }
            pi += 1;
        }
        if pi < parts.len() {
            let mut val: Option<String> = None;
            if !self.translatefilename.is_null() {
            } else if parts[pi] == "--translate-file" || parts[pi] == "-translate-file" {
                val = parts.get(pi + 1).cloned();
            } else if let Some(v) = parts[pi].strip_prefix("--translate-file=") {
                val = Some(v.to_string());
            } else if let Some(v) = parts[pi].strip_prefix("-translate-file=") {
                val = Some(v.to_string());
            }
            if let Some(v) = val {
                if !v.is_empty() {
                    self.translatefilename = Ptr::<u8>::from_bytes(v.as_bytes());
                }
            }
        }
    }

    /// getopt_long_only with the "+" (stop at first non-option) convention.
    fn parse_options(&mut self) {
        let argv = self.rt.argv.clone();
        let prog = argv[0].clone();
        let mut i = 1;
        while i < argv.len() {
            let a = &argv[i];
            if a == "--" {
                i += 1;
                break;
            }
            if !a.starts_with('-') || a == "-" {
                break;
            }
            let body = a.trim_start_matches('-');
            let (name, inline_val) = match body.find('=') {
                Some(e) => (&body[..e], Some(body[e + 1..].to_string())),
                None => (body, None),
            };
            // exact match first, then unique prefix
            let mut m: Vec<&Opt> = LONG_OPTIONS.iter().filter(|o| o.name == name).collect();
            if m.is_empty() {
                m = LONG_OPTIONS.iter().filter(|o| o.name.starts_with(name)).collect();
            }
            i += 1;
            if m.is_empty() {
                eprint!("{}: unrecognized option '{}'\r\n", prog, a);
                continue;
            }
            if m.len() > 1 {
                eprint!("{}: option '{}' is ambiguous\r\n", prog, a);
                continue;
            }
            let opt = m[0];
            let optarg: Option<String> = match opt.has_arg {
                0 => {
                    if inline_val.is_some() {
                        eprint!("{}: option '{}' doesn't allow an argument\r\n", prog, a.split('=').next().unwrap());
                        continue;
                    }
                    None
                }
                1 => {
                    if inline_val.is_some() {
                        inline_val
                    } else if i < argv.len() {
                        i += 1;
                        Some(argv[i - 1].clone())
                    } else {
                        eprint!("{}: option '{}' requires an argument\r\n", prog, a);
                        continue;
                    }
                }
                _ => inline_val,
            };
            self.apply_option(opt.name, optarg);
        }
        self.rt.optind = i;
        if let Some(od) = self.rt.output_directory.clone() {
            self.rt.kpse().xputenv("TEXMF_OUTPUT_DIRECTORY", &od);
        } else if let Some(v) = self.rt.kpse().getenv("TEXMF_OUTPUT_DIRECTORY") {
            self.rt.output_directory = Some(v);
        }
        self.rt.recorder.borrow_mut().output_directory = self.rt.output_directory.clone().map(|s| s.into_bytes());
    }

    fn apply_option(&mut self, name: &str, optarg: Option<String>) {
        let arg = optarg.clone().unwrap_or_default();
        match name {
            "ini" => self.iniversion = true,
            "halt-on-error" => self.haltonerrorp = 1,
            "recorder" => self.rt.recorder.borrow_mut().enabled = true,
            "mltex" => self.mltexp = true,
            "enc" => self.enctexp = true,
            "etex" => self.etexp = true,
            "shell-escape" | "enable-write18" => self.shellenabledp = 1,
            "no-shell-escape" | "disable-write18" => self.shellenabledp = -1,
            "debug-format" => self.debugformatfile = true,
            "file-line-error-style" | "file-line-error" => self.filelineerrorstylep = 1,
            "no-file-line-error-style" | "no-file-line-error" => self.filelineerrorstylep = -1,
            "parse-first-line" => self.parsefirstlinep = 1,
            "no-parse-first-line" => self.parsefirstlinep = -1,
            "8bit" => self.eightbitp = 1,
            "kpathsea-debug" => {
                let v = atoi(&arg);
                self.rt.kpathsea_debug |= v as u32;
                self.rt.kpse().set_debug(v as u32);
            }
            "progname" => self.rt.user_progname = Some(arg),
            "cnf-line" => self.rt.user_cnf_lines.push(arg),
            "jobname" => self.rt.c_job_name = Some(normalize_quotes(&arg, "jobname")),
            "fmt" | "efmt" => {
                self.dumpname = Ptr::<u8>::from_bytes(arg.as_bytes());
                self.dumpoption = true;
            }
            "output-directory" => self.rt.output_directory = Some(arg),
            "output-comment" => {
                let b = arg.as_bytes();
                if b.len() < 256 {
                    self.outputcomment = Ptr::<u8>::from_bytes(b);
                } else {
                    warning(&format!("Comment truncated to 255 characters from {}. ({})", b.len(), arg));
                    self.outputcomment = Ptr::<u8>::from_bytes(&b[..255]);
                }
            }
            "shell-restricted" => {
                self.shellenabledp = 1;
                self.restrictedshell = 1;
            }
            "src-specials" => {
                self.rt.last_source_name = Some(String::new());
                match optarg {
                    None => {
                        self.insertsrcspecialeverypar = true;
                        self.insertsrcspecialauto = true;
                        self.rt.srcspecialsoption = true;
                        self.srcspecialsp = true;
                    }
                    Some(o) => self.parse_src_specials_option(&o),
                }
            }
            "output-format" => {
                self.pdfoutputoption = 1;
                if arg == "dvi" {
                    self.pdfoutputvalue = 0;
                } else if arg == "pdf" {
                    self.pdfoutputvalue = 2;
                } else {
                    warning(&format!("Ignoring unknown value `{}' for --output-format", arg));
                    self.pdfoutputoption = 0;
                }
            }
            "draftmode" => {
                self.pdfdraftmodeoption = 1;
                self.pdfdraftmodevalue = 1;
            }
            "translate-file" => self.translatefilename = Ptr::<u8>::from_bytes(arg.as_bytes()),
            "default-translate-file" => self.rt.default_translate_filename = Some(arg),
            "mktex" => self.rt.kpse().maketex_option(&arg, true),
            "no-mktex" => self.rt.kpse().maketex_option(&arg, false),
            "interaction" => match arg.as_str() {
                "batchmode" => self.interactionoption = 0,
                "nonstopmode" => self.interactionoption = 1,
                "scrollmode" => self.interactionoption = 2,
                "errorstopmode" => self.interactionoption = 3,
                _ => warning(&format!("Ignoring unknown argument `{}' to --interaction", arg)),
            },
            "help" => {
                let mut out = String::new();
                for l in HELP {
                    out.push_str(l);
                    out.push('\n');
                }
                out.push_str("\nEmail bug reports to pdftex@tug.org (https://lists.tug.org/pdftex).\n");
                self.rt.files.write(FileRef::STDOUT, out.as_bytes());
                self.x_uexit(0);
            }
            "synctex" => self.synctexoption = strtol0(&arg),
            "version" => {
                let out = format!(
                    "pdfTeX 3.141592653-2.6-1.40.29{}\n{}\nCopyright 2026 Han The Thanh (pdfTeX) et al.\n\
                     There is NO warranty.  Redistribution of this software is\n\
                     covered by the terms of both the pdfTeX copyright and\n\
                     the Lesser GNU General Public License.\n\
                     For more information about these matters, see the file\n\
                     named COPYING and the pdfTeX source.\n\
                     Primary author of pdfTeX: Han The Thanh (pdfTeX) et al.\n\
                     Compiled with Rust ports of libpng 1.6.55, zlib 1.3.2 and xpdf 4.06 (no C libraries)\n",
                    VERSION_STRING, KPSE_VERSION
                );
                self.rt.files.write(FileRef::STDOUT, out.as_bytes());
                self.x_uexit(0);
            }
            _ => {}
        }
    }

    fn parse_src_specials_option(&mut self, opt_list: &str) {
        self.insertsrcspecialauto = false;
        for tok in opt_list.split(|c| c == ',' || c == ' ').filter(|t| !t.is_empty()) {
            match tok {
                "everypar" | "par" | "auto" => {
                    self.insertsrcspecialauto = true;
                    self.insertsrcspecialeverypar = true;
                }
                "everyparend" | "parend" => self.insertsrcspecialeveryparend = true,
                "everycr" | "cr" => self.insertsrcspecialeverycr = true,
                "everymath" | "math" => self.insertsrcspecialeverymath = true,
                "everyhbox" | "hbox" => self.insertsrcspecialeveryhbox = true,
                "everyvbox" | "vbox" => self.insertsrcspecialeveryvbox = true,
                "everydisplay" | "display" => self.insertsrcspecialeverydisplay = true,
                "none" => {
                    self.insertsrcspecialauto = false;
                    self.insertsrcspecialeverypar = false;
                    self.insertsrcspecialeveryparend = false;
                    self.insertsrcspecialeverycr = false;
                    self.insertsrcspecialeverymath = false;
                    self.insertsrcspecialeveryhbox = false;
                    self.insertsrcspecialeveryvbox = false;
                    self.insertsrcspecialeverydisplay = false;
                }
                _ => warning(&format!("Ignoring unknown argument `{}' to --src-specials", tok)),
            }
        }
        self.srcspecialsp = self.insertsrcspecialauto
            | self.insertsrcspecialeverypar
            | self.insertsrcspecialeveryparend
            | self.insertsrcspecialeverycr
            | self.insertsrcspecialeverymath
            | self.insertsrcspecialeveryhbox
            | self.insertsrcspecialeveryvbox
            | self.insertsrcspecialeverydisplay;
        self.rt.srcspecialsoption = true;
    }

    fn init_shell_escape(&mut self) {
        if self.shellenabledp < 0 {
            self.shellenabledp = 0;
        } else {
            if self.shellenabledp == 0 {
                if let Some(v1) = self.rt.kpse().var_value("shell_escape") {
                    match v1.as_bytes().first() {
                        Some(b't') | Some(b'y') | Some(b'1') => self.shellenabledp = 1,
                        Some(b'p') => {
                            self.shellenabledp = 1;
                            self.restrictedshell = 1;
                        }
                        _ => {}
                    }
                }
            }
            if self.shellenabledp != 0 && self.restrictedshell == 1 {
                if let Some(v2) = self.rt.kpse().var_value("shell_escape_commands") {
                    let list: Vec<String> = v2.split(',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
                    self.rt.cmdlist = Some(list);
                }
            }
        }
    }

    // ------------------------------------------------------------------ topenin
    pub fn topenin(&mut self) {
        let first = self.first;
        let buffer = self.buffer;
        *ix(buffer, first) = 0;
        if self.rt.optind < self.rt.argv.len() && !self.rt.argv_consumed {
            let mut k = first;
            for a in self.rt.argv[self.rt.optind..].to_vec() {
                for &c in a.as_bytes() {
                    *ix(buffer, k) = c;
                    k += 1;
                }
                *ix(buffer, k) = b' ';
                k += 1;
            }
            self.rt.argv_consumed = true;
            *ix(buffer, k) = 0;
        }
        let mut last = first;
        while *ix(buffer, last) != 0 {
            last += 1;
        }
        last -= 1;
        while last >= first && matches!(*ix(buffer, last), b' ' | b'\r' | b'\n') {
            last -= 1;
        }
        last += 1;
        self.last = last;
        let xord = self.xord;
        for i in first..last {
            let c = *ix(buffer, i);
            *ix(buffer, i) = *ix(xord, c as i32);
        }
    }

    // ------------------------------------------------------------------ TCX
    pub fn readtcxfile(&mut self) {
        let mut name = bytes_to_string(self.translatefilename.cstr_bytes());
        if !has_suffix(&name) {
            name.push_str(".tcx");
        }
        let orig = name.clone();
        let found = self.rt.kpse().find_file(name.as_bytes(), kpse::WEB2C_FORMAT, true);
        match found {
            Some(pathb) => {
                self.translatefilename = Ptr::<u8>::from_bytes(&pathb);
                let path = bytes_to_string(&pathb);
                let osp = self.os_path(&pathb);
                let data = std::fs::read(&osp).unwrap_or_else(|_| fatal_perror(&path));
                let mut line_count = 0u32;
                for raw in split_lines(&data) {
                    let mut line = raw.to_vec();
                    if let Some(p) = line.iter().position(|&c| c == b'%') {
                        line.truncate(p);
                    }
                    line_count += 1;
                    let (first, rest) = tcx_get_num(255, line_count, &line, &path);
                    if first >= 0 {
                        let (mut second, rest2) = tcx_get_num(255, line_count, rest, &path);
                        let printable;
                        if second >= 0 {
                            *ix(self.xord, first) = second as u8;
                            *ix(self.xchr, second) = first as u8;
                            let (mut p, _) = tcx_get_num(1, line_count, rest2, &path);
                            if p == -1 {
                                p = 1;
                            }
                            if (32..=126).contains(&second) {
                                p = 1;
                            }
                            printable = p;
                        } else {
                            second = first;
                            printable = 1;
                        }
                        *ix(self.xprn, second) = printable as u8;
                    }
                }
            }
            None => {
                self.translatefilename = Ptr::null();
                warning(&format!("Could not open char translation file `{}'", orig));
            }
        }
    }

    // ------------------------------------------------------------------ cnf values
    fn texmf_yesno_str(&mut self, var: &str) -> bool {
        match self.rt.kpse().var_value(var) {
            Some(v) => matches!(v.as_bytes().first(), Some(b't') | Some(b'y') | Some(b'1')),
            None => false,
        }
    }

    pub fn texmfyesno(&mut self, var: &[u8]) -> bool {
        self.texmf_yesno_str(&bytes_to_string(var))
    }

    /// setupvar.c setupboundvariable
    pub fn setup_bound_var(&mut self, name: Ptr<u8>, dflt: i32) -> i32 {
        let var = bytes_to_string(name.cstr_bytes());
        let mut v = dflt;
        if let Some(expansion) = self.rt.kpse().var_value(&var) {
            let conf_val = atoi(&expansion);
            if conf_val < 0 || (conf_val == 0 && dflt > 0) {
                eprint!(
                    "{}: Bad value ({}) in environment or texmf.cnf for {}, keeping {}.\r\n",
                    self.rt.invocation_name(),
                    conf_val,
                    var,
                    dflt
                );
            } else {
                v = conf_val;
            }
        }
        v
    }

    // ------------------------------------------------------------------ time
    pub fn init_start_time(&mut self) {
        if !self.rt.start_time_set {
            self.rt.start_time_set = true;
            if let Ok(sde) = std::env::var("SOURCE_DATE_EPOCH") {
                match sde.parse::<u64>() {
                    Ok(epoch) => {
                        let epoch = epoch.min(32535291599);
                        self.rt.start_time = epoch as i64;
                        self.rt.sde_set = true;
                    }
                    Err(_) => fatal(&format!(
                        "invalid epoch-seconds-timezone value for environment variable $SOURCE_DATE_EPOCH: {}",
                        sde
                    )),
                }
            } else {
                self.rt.start_time = time::now_unix();
            }
        }
    }

    pub fn date_and_time(&mut self) -> (i32, i32, i32, i32) {
        // texmfmp.c get_date_and_time also installs the Ctrl-C handler (WIN32:
        // SetConsoleCtrlHandler) that sets TeX's `interrupt`.
        install_interrupt_handler(&raw mut self.interrupt);
        let sde = std::env::var("FORCE_SOURCE_DATE").ok();
        let tm = if sde.as_deref() == Some("1") {
            self.init_start_time();
            self.rt.fsd_set = true;
            time::gmtime(self.rt.start_time)
        } else {
            let tm = time::localtime(time::now_unix());
            if let Some(v) = &sde {
                if !v.is_empty() && v != "0" {
                    warning(&format!("invalid value (expected 0 or 1) for environment variable $FORCE_SOURCE_DATE: {}", v));
                }
            }
            tm
        };
        (tm.hour * 60 + tm.min, tm.mday, tm.mon + 1, tm.year + 1900)
    }

    pub fn seconds_and_micros(&mut self) -> (i32, i32) {
        let (s, m) = time::now_secs_micros();
        (s as i32, m)
    }

    pub fn makepdftime(t: i64, utc: bool) -> String {
        let lt = if utc { time::gmtime(t) } else { time::localtime(t) };
        let mut s = format!(
            "D:{:04}{:02}{:02}{:02}{:02}{:02}",
            lt.year + 1900,
            lt.mon + 1,
            lt.mday,
            lt.hour,
            lt.min,
            lt.sec
        );
        if s.as_bytes()[14] == b'6' {
            s.truncate(14);
            s.push_str("59");
        }
        let gmt = time::gmtime(t);
        let mut off = 60 * (lt.hour - gmt.hour) + lt.min - gmt.min;
        if lt.year != gmt.year {
            off += if lt.year > gmt.year { 1440 } else { -1440 };
        } else if lt.yday != gmt.yday {
            off += if lt.yday > gmt.yday { 1440 } else { -1440 };
        }
        if off == 0 {
            s.push('Z');
        } else {
            let off_hours = off / 60;
            let off_mins = (off - off_hours * 60).abs();
            s.push_str(&format!("{:+03}'{:02}'", off_hours, off_mins));
        }
        s
    }

    pub fn initstarttime(&mut self) {
        if !self.rt.start_time_set {
            self.init_start_time();
            let utc = std::env::var("SOURCE_DATE_EPOCH").is_ok();
            self.rt.start_time_str = Self::makepdftime(self.rt.start_time, utc);
        }
    }

    // ------------------------------------------------------------------ strings (utils.c / texmfmp.c)
    /// The bytes of TeX string s.
    pub fn tex_string(&self, s: i32) -> Vec<u8> {
        let a = *ix(self.strstart, s);
        let b = *ix(self.strstart, s + 1);
        slice(self.strpool, a, (b - a).max(0) as usize).to_vec()
    }

    /// pdftexdir/utils.c maketexstring
    pub fn maketexstring(&mut self, s: &[u8]) -> i32 {
        if s.is_empty() {
            return self.getnullstr();
        }
        let l = s.len() as i32;
        self.strroom_c(l);
        for &c in s {
            *ix(self.strpool, self.poolptr) = c;
            self.poolptr += 1;
        }
        self.makestring()
    }

    /// utils.c: `str_room(l)` macro: if pool_ptr + l > pool_size then overflow
    fn strroom_c(&mut self, l: i32) {
        if self.poolptr + l > self.poolsize {
            let s = self.maketexstring_static(b"pool size");
            self.overflow(s, self.poolsize - self.initpoolptr);
        }
    }

    fn maketexstring_static(&mut self, _s: &[u8]) -> i32 {
        // "pool size" is pool string 260 ("09pool size") in pdftex.pool
        260
    }

    pub fn makefullnamestring(&mut self) -> i32 {
        let s = self.rt.fullnameoffile.clone().unwrap_or_default();
        self.maketexstring(&s)
    }

    pub fn getjobname(&mut self, name: i32) -> i32 {
        match self.rt.c_job_name.clone() {
            Some(j) => self.maketexstring(j.as_bytes()),
            None => name,
        }
    }

    pub fn makecstring(&mut self, s: i32) -> Ptr<u8> {
        let b = self.tex_string(s);
        if b.len() + 1 > 1024 * 1024 {
            self.pdftex_fail(&format!("buffer overflow at file {}, line {}", "texmfmp.c", 3494));
        }
        self.rt.cstrbuf.free();
        self.rt.cstrbuf = Ptr::<u8>::from_bytes(&b);
        self.rt.cstrbuf
    }

    pub fn makecfilename(&mut self, s: i32) -> Vec<u8> {
        self.tex_string(s).into_iter().take_while(|&c| c != 0).filter(|&c| c != b'"').collect()
    }

    pub fn find_input_file(&mut self, s: i32) -> Option<Vec<u8>> {
        let filename = self.makecfilename(s);
        if let Some(od) = self.rt.output_directory.clone() {
            if !kpse::absolute_p(&filename, false) {
                let mut p = od.into_bytes();
                p.push(b'/');
                p.extend_from_slice(&filename);
                let osp = self.os_path(&p);
                if crt_stat(&osp).map(|m| !m.is_dir).unwrap_or(false) {
                    return Some(p);
                }
            }
        }
        if !self.rt.kpse().in_name_ok(&filename) {
            return None;
        }
        self.rt.kpse().find_file(&filename, kpse::TEX_FORMAT, true)
    }

    fn pool_append(&mut self, bytes: &[u8]) {
        if (self.poolptr as i64 + bytes.len() as i64) >= self.poolsize as i64 {
            self.poolptr = self.poolsize;
            return;
        }
        for &c in bytes {
            *ix(self.strpool, self.poolptr) = c;
            self.poolptr += 1;
        }
    }

    pub fn getcreationdate(&mut self) {
        self.initstarttime();
        let s = self.rt.start_time_str.clone();
        self.pool_append(s.as_bytes());
    }

    pub fn getfilemoddate(&mut self, s: i32) {
        let Some(file_name) = self.find_input_file(s) else { return };
        self.recorder_record_input(&file_name);
        let osp = self.os_path(&file_name);
        if let Some(st) = crt_stat(&osp) {
            let use_utc = self.rt.fsd_set && self.rt.sde_set;
            let ts = Self::makepdftime(st.mtime, use_utc);
            self.pool_append(ts.as_bytes());
        }
    }

    pub fn getfilesize(&mut self, s: i32) {
        let Some(file_name) = self.find_input_file(s) else { return };
        self.recorder_record_input(&file_name);
        let osp = self.os_path(&file_name);
        if let Some(st) = crt_stat(&osp) {
            let buf = format!("{}", st.size as u32);
            self.pool_append(buf.as_bytes());
        }
    }

    pub fn getfiledump(&mut self, s: i32, offset: i32, length: i32) {
        if length == 0 {
            return;
        }
        if self.poolptr + 2 * length + 1 >= self.poolsize {
            self.poolptr = self.poolsize;
            return;
        }
        let Some(file_name) = self.find_input_file(s) else { return };
        let osp = self.os_path(&file_name);
        let Ok(data) = std::fs::read(&osp) else { return };
        self.recorder_record_input(&file_name);
        if offset < 0 {
            return;
        }
        let start = (offset as usize).min(data.len());
        let end = (start + length as usize).min(data.len());
        let hex: String = data[start..end].iter().map(|b| format!("{:02X}", b)).collect();
        for &c in hex.as_bytes() {
            *ix(self.strpool, self.poolptr) = c;
            self.poolptr += 1;
        }
    }

    pub fn getmd5sum(&mut self, s: i32, file: bool) {
        let digest = if file {
            let Some(file_name) = self.find_input_file(s) else { return };
            let osp = self.os_path(&file_name);
            let Ok(data) = std::fs::read(&osp) else { return };
            self.recorder_record_input(&file_name);
            md5::md5(&data)
        } else {
            md5::md5(&self.tex_string(s))
        };
        if self.poolptr + 32 >= self.poolsize {
            return;
        }
        let hex: String = digest.iter().map(|b| format!("{:02X}", b)).collect();
        self.pool_append(hex.as_bytes());
    }

    // ------------------------------------------------------------------ source specials
    fn gettexstring(&self, s: i32) -> String {
        bytes_to_string(&self.tex_string(s))
    }

    pub fn isnewsource(&mut self, srcfilename: i32, lineno: i32) -> bool {
        let name = self.gettexstring(srcfilename);
        let last = self.rt.last_source_name.clone().unwrap_or_default();
        compare_paths(&name, &last) != 0 || lineno != self.rt.last_lineno
    }

    pub fn remembersourceinfo(&mut self, srcfilename: i32, lineno: i32) {
        self.rt.last_source_name = Some(self.gettexstring(srcfilename));
        self.rt.last_lineno = lineno;
    }

    pub fn makesrcspecial(&mut self, srcfilename: i32, lineno: i32) -> i32 {
        let oldpoolptr = self.poolptr;
        let filename = self.tex_string(srcfilename);
        let buf = format!("src:{} ", lineno);
        if (self.poolptr as usize + buf.len() + filename.len()) >= self.poolsize as usize {
            eprint!("\r\nstring pool overflow\r\n");
            std::process::exit(1);
        }
        for &c in buf.as_bytes().iter().chain(filename.iter()) {
            *ix(self.strpool, self.poolptr) = c;
            self.poolptr += 1;
        }
        oldpoolptr
    }

    // ------------------------------------------------------------------ editor
    pub fn calledit(&mut self, filename: Ptr<u8>, fnstart: i32, fnlength: i32, linenumber: i32) {
        for is_ptr in 0..self.inputptr {
            let rec = *ix(self.inputstack, is_ptr);
            if rec.statefield == 0 || rec.namefield <= 255 {
                continue;
            }
            let if_ptr = rec.indexfield as i32;
            if if_ptr < 1 || if_ptr > self.inopen {
                eprint!("{}:calledit: unexpected if_ptr={} not in range 1..{},", self.rt.argv[0], if_ptr, self.inopen);
                eprint!("from input_stack[{}].namefield={}\r\n", is_ptr, rec.namefield);
                std::process::exit(1);
            }
            let f = *ix(self.inputfile, if_ptr);
            if !f.is_null() {
                self.rt.files.close(f);
            }
        }
        let edit_value = self.rt.kpse().var_value("TEXEDIT").unwrap_or_else(|| "texworks --position=+%d \"%s\"".to_string());
        let ev = edit_value.as_bytes();
        let fname: Vec<u8> = (0..fnlength).map(|i| *ix(self.xchr, *ix(filename, fnstart + i) as i32)).collect();
        let dontchange = (ev.len() > 2 && ev[0].is_ascii_alphabetic() && ev[1] == b':' && (ev[2] == b'/' || ev[2] == b'\\'))
            || (ev.len() > 3 && ev[0] == b'"' && ev[1].is_ascii_alphabetic() && ev[2] == b':' && (ev[3] == b'/' || ev[3] == b'\\'));
        let mut command: Vec<u8> = Vec::new();
        let mut editorname: Vec<u8> = Vec::new();
        let (mut sdone, mut ddone, mut cnt) = (false, false, 0);
        let mut i = 0;
        while i < ev.len() {
            let c = ev[i];
            i += 1;
            if c == b'%' {
                let c2 = if i < ev.len() { ev[i] } else { 0 };
                i += 1;
                match c2 {
                    b'd' => {
                        if ddone {
                            fatal("call_edit: `%d' appears twice in editor command");
                        }
                        command.extend_from_slice(linenumber.to_string().as_bytes());
                        ddone = true;
                    }
                    b's' => {
                        if sdone {
                            fatal("call_edit: `%s' appears twice in editor command");
                        }
                        command.extend_from_slice(&fname);
                        sdone = true;
                    }
                    0 => {
                        command.push(b'%');
                        i -= 1;
                    }
                    _ => {
                        command.push(b'%');
                        command.push(c2);
                    }
                }
            } else if dontchange {
                command.push(c);
            } else if isspace(c) && cnt == 0 {
                cnt += 1;
                command.clear();
                command.push(c);
            } else if !isspace(c) && cnt == 0 {
                editorname.push(c);
            } else {
                command.push(c);
            }
        }
        let command_s = bytes_to_string(&command);
        let fullcmd = if !dontchange {
            let en = bytes_to_string(&editorname);
            if en.starts_with('.') || en.starts_with('/') || en.starts_with('\\') {
                eprint!("{} is not allowed to execute.\r\n", en);
                self.x_uexit(1);
            }
            match search_path(&en) {
                Some(full) => format!("\"{}\"{}", full, command_s),
                None => {
                    eprint!("I cannot find {} in the PATH.\r\n", en);
                    self.x_uexit(1);
                }
            }
        } else {
            command_s.clone()
        };
        self.rt.files.flush_stdout();
        let ok = std::process::Command::new("cmd")
            .args(["/c"])
            .arg(&fullcmd)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            eprint!("! Trouble executing `{}'.\r\n", command_s);
        }
        self.x_uexit(1);
    }

    // ------------------------------------------------------------------ exit
    /// lib/uexit.c (also flushes our buffered streams).
    pub fn x_uexit(&mut self, code: i32) -> ! {
        crate::rt::phase::mark("x_uexit");
        self.parzip_flush_at_exit();
        #[cfg(feature = "prof")]
        crate::rt::prof::report(crate::gen::prof_names::NAMES);
        self.rt.files.flush_stdout();
        let open: Vec<FileRef> = (4..self.rt.files.slots.len()).map(|i| FileRef(i as u32)).collect();
        for f in open {
            self.rt.files.close(f);
        }
        self.rt.recorder.borrow_mut().flush();
        if let Some(k) = self.rt.files.kpse.as_mut() {
            k.flush_stderr();
        }
        crate::rt::phase::mark("files closed");
        crate::rt::phase::report();
        crate::rt::sampler::finish();
        #[cfg(feature = "crt-exit")]
        {
            extern "C" {
                fn exit(code: i32) -> !;
            }
            unsafe { exit(code) }
        }
        #[cfg(not(feature = "crt-exit"))]
        std::process::exit(code);
    }

    pub fn uexit(&mut self, code: i32) -> ! {
        self.x_uexit(code)
    }

    /// web2c `loadpoolstrings` generated by makecpool from pdftex.pool.
    pub fn loadpoolstrings(&mut self, spare_size: i32) -> i32 {
        let pool: &[u8] = include_bytes!("../../../web/pdftex.pool");
        let mut g = 0;
        let mut i = 0i32;
        for line in pool.split(|&c| c == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.first() == Some(&b'*') {
                break;
            }
            if line.is_empty() && i == 0 && g == 0 {
                // an empty line would be an empty string; pdftex.pool has none
            }
            let o = if line.len() >= 2 && line[0].is_ascii_digit() && line[1].is_ascii_digit() { 2 } else { 0 };
            let s = &line[o..];
            i += s.len() as i32;
            if i >= spare_size {
                return 0;
            }
            for &c in s {
                *ix(self.strpool, self.poolptr) = c;
                self.poolptr += 1;
            }
            g = self.makestring();
        }
        g
    }
}

fn compare_paths(p1: &str, p2: &str) -> i32 {
    let a = p1.as_bytes();
    let b = p2.as_bytes();
    let mut i = 0;
    loop {
        let ca = *a.get(i).unwrap_or(&0);
        let cb = *b.get(i).unwrap_or(&0);
        let ret = ca as i32 - cb as i32;
        if (ret == 0 && cb != 0) || (crate::rt::io::is_dir_sep(ca) && crate::rt::io::is_dir_sep(cb)) {
            i += 1;
            continue;
        }
        return ret.signum();
    }
}

pub fn atoi(s: &str) -> i32 {
    let b = s.trim_start().as_bytes();
    let mut i = 0;
    let mut neg = false;
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        neg = b[i] == b'-';
        i += 1;
    }
    let mut v: i64 = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        v = v * 10 + (b[i] - b'0') as i64;
        if v > i32::MAX as i64 + 1 {
            break;
        }
        i += 1;
    }
    (if neg { -v } else { v }) as i32
}

/// strtol(s, NULL, 0)
pub fn strtol0(s: &str) -> i32 {
    let t = s.trim_start();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let (radix, digits) = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        (16, h)
    } else if t.starts_with('0') && t.len() > 1 {
        (8, &t[1..])
    } else {
        (10, t)
    };
    let end = digits.find(|c: char| !c.is_digit(radix)).unwrap_or(digits.len());
    let v = i64::from_str_radix(&digits[..end], radix).unwrap_or(0);
    let v = if neg { -v } else { v };
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// tcx_get_num: strtol(start, &post, 0) with TeX Live's checks.
fn tcx_get_num<'a>(upb: i32, line_count: u32, start: &'a [u8], fname: &str) -> (i32, &'a [u8]) {
    let mut i = 0;
    while i < start.len() && (start[i] as char).is_ascii_whitespace() {
        i += 1;
    }
    let mut j = i;
    if j < start.len() && (start[j] == b'-' || start[j] == b'+') {
        j += 1;
    }
    let (radix, ds) = if j + 1 < start.len() && start[j] == b'0' && (start[j + 1] == b'x' || start[j + 1] == b'X')
        && start.get(j + 2).map_or(false, |c| c.is_ascii_hexdigit())
    {
        (16, j + 2)
    } else if j < start.len() && start[j] == b'0' {
        (8, j)
    } else {
        (10, j)
    };
    let mut k = ds;
    while k < start.len() && (start[k] as char).is_digit(radix) {
        k += 1;
    }
    if k == ds {
        let rest = &start[i..];
        if !rest.is_empty() {
            eprint!("{}:{}: Expected numeric constant, not `{}'.\r\n", fname, line_count, bytes_to_string(start));
        }
        return (-1, start);
    }
    let txt = std::str::from_utf8(&start[ds..k]).unwrap_or("0");
    let mut num = i64::from_str_radix(txt, radix).unwrap_or(0);
    if start.get(i) == Some(&b'-') {
        num = -num;
    }
    if num < 0 || num > upb as i64 {
        eprint!("{}:{}: Destination charcode {} <0 or >{}.\r\n", fname, line_count, num, upb);
        return (-1, &start[k..]);
    }
    (num as i32, &start[k..])
}

fn has_suffix(name: &str) -> bool {
    let base = name.rsplit(|c| c == '/' || c == '\\').next().unwrap_or(name);
    base.contains('.')
}

fn split_lines(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut s = 0;
    for (i, &c) in data.iter().enumerate() {
        if c == b'\n' {
            let mut e = i;
            if e > s && data[e - 1] == b'\r' {
                e -= 1;
            }
            out.push(&data[s..e]);
            s = i + 1;
        }
    }
    if s < data.len() {
        out.push(&data[s..]);
    }
    out
}

fn search_path(name: &str) -> Option<String> {
    let path = std::env::var("PATH").ok()?;
    for dir in path.split(';') {
        for ext in [".exe", ".bat"] {
            let p = std::path::Path::new(dir).join(format!("{}{}", name, ext));
            if p.is_file() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
    }
    None
}

// ------------------------------------------------------------------ Ctrl-C (texmfmp.c catch_interrupt)
static INTERRUPT_FLAG: std::sync::atomic::AtomicPtr<i32> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

#[link(name = "kernel32")]
extern "system" {
    fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
}

unsafe extern "system" fn catch_interrupt(ctrl_type: u32) -> i32 {
    const CTRL_C_EVENT: u32 = 0;
    const CTRL_BREAK_EVENT: u32 = 1;
    match ctrl_type {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => {
            let p = INTERRUPT_FLAG.load(std::sync::atomic::Ordering::SeqCst);
            if !p.is_null() {
                std::ptr::write_volatile(p, 1);
            }
            1
        }
        _ => 0,
    }
}

fn install_interrupt_handler(flag: *mut i32) {
    INTERRUPT_FLAG.store(flag, std::sync::atomic::Ordering::SeqCst);
    unsafe {
        SetConsoleCtrlHandler(Some(catch_interrupt), 1);
    }
}

// ------------------------------------------------------------------ C runtime stat()
pub struct CStat {
    pub size: u64,
    pub mtime: i64,
    pub is_dir: bool,
}

/// The Windows C runtime's stat(): like fs::metadata, but DOS device names (nul, con,
/// aux, prn, com1-9, lpt1-9, with or without a trailing colon) succeed with size 0 and
/// time 0. expl3 relies on this to detect Windows (`\pdffilesize{nul:}`).
pub fn crt_stat(path: &std::path::Path) -> Option<CStat> {
    if let Ok(md) = std::fs::metadata(path) {
        let mtime = match md.modified() {
            Ok(m) => match m.duration_since(std::time::UNIX_EPOCH) {
                Ok(d) => d.as_secs() as i64,
                Err(e) => -(e.duration().as_secs() as i64),
            },
            Err(_) => 0,
        };
        return Some(CStat { size: md.len(), mtime, is_dir: md.is_dir() });
    }
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let name = name.trim_end_matches(':');
    let dev = matches!(name, "nul" | "con" | "aux" | "prn")
        || ((name.starts_with("com") || name.starts_with("lpt"))
            && name.len() == 4
            && name.as_bytes()[3].is_ascii_digit()
            && name.as_bytes()[3] != b'0');
    if dev {
        Some(CStat { size: 0, mtime: 0, is_dir: false })
    } else {
        None
    }
}
