//! rkpsewhich: a port of kpsewhich.c (kpathsea 6.4.2, Windows build) on
//! top of texrs-kpathsea, used for differential testing against the C
//! kpsewhich.exe.

use std::io::{BufRead, Write};

use texrs_kpathsea::{FileFormat, Kpse, SrcLevel, BUG_ADDRESS, LAST_FORMAT, VERSION_STRING};

/// kpsewhich.c `format_abbrs`.
const FORMAT_ABBRS: &[(&str, FileFormat)] = &[
    ("bitmapfont", FileFormat::AnyGlyph),
    ("mpsupport", FileFormat::Mpsupport),
    ("psheader", FileFormat::TexPsHeader),
    ("doc", FileFormat::Texdoc),
    ("source", FileFormat::Texsource),
    ("src", FileFormat::Texsource),
    ("trofffont", FileFormat::TroffFont),
    ("type1", FileFormat::Type1),
    ("pfb", FileFormat::Type1),
    ("dvipsconfig", FileFormat::DvipsConfig),
    ("truetype", FileFormat::Truetype),
    ("ttf", FileFormat::Truetype),
    ("type42", FileFormat::Type42),
    ("web2c", FileFormat::Web2c),
    ("othertext", FileFormat::ProgramText),
    ("otherbin", FileFormat::ProgramBinary),
    ("miscfont", FileFormat::Miscfonts),
    ("enc", FileFormat::Enc),
    ("cmap", FileFormat::Cmap),
    ("sfd", FileFormat::Sfd),
    ("opentype", FileFormat::Opentype),
    ("otf", FileFormat::Opentype),
    ("pdftexconfig", FileFormat::PdftexConfig),
    ("lig", FileFormat::Lig),
    ("fea", FileFormat::Fea),
    ("cidmap", FileFormat::Cid),
];

/// kpsewhich.c `long_options`: (name, has_arg).
const LONG_OPTIONS: &[(&str, bool)] = &[
    ("D", true),
    ("all", false),
    ("casefold-search", false),
    ("cnf-line", true),
    ("debug", true),
    ("dpi", true),
    ("engine", true),
    ("expand-braces", true),
    ("expand-path", true),
    ("expand-var", true),
    ("format", true),
    ("help", false),
    ("help-formats", false),
    ("interactive", false),
    ("mktex", true),
    ("mode", true),
    ("must-exist", false),
    ("path", true),
    ("no-casefold-search", false),
    ("no-mktex", true),
    ("progname", true),
    ("safe-extended-in-name", true),
    ("safe-extended-out-name", true),
    ("safe-in-name", true),
    ("safe-out-name", true),
    ("subdir", true),
    ("show-path", true),
    ("var-brace-value", true),
    ("var-value", true),
    ("version", false),
];

const USAGE: &str = "\n\
Standalone path lookup and expansion for the Kpathsea library.\n\
The default is to look up each FILENAME in turn and report its\n\
first match (if any) to standard output.\n\
\n\
If multiple FILENAMEs are given and a given file is not found, a blank\n\
line is written to standard output.\n\
\n\
When looking up format (.fmt/.base/.mem) files, it is usually necessary\n\
to also use -engine, or nothing will be returned; in particular,\n\
-engine=/ will return matching format files for any engine.\n\
\n\
The exit status is 0 if all files are found, nonzero otherwise.\n\
\n\
-all                   output all matches, one per line (no effect with pk/gf).\n\
[-no]-casefold-search  fall back to case-insensitive search if no exact match.\n\
-cnf-line=STRING       handle STRING as a configuration file line.\n\
-debug=NUM             set debugging flags.\n\
-D, -dpi=NUM           use a base resolution of NUM; default 600.\n\
-engine=STRING         set engine name to STRING.\n\
-expand-braces=STRING  output variable and brace expansion of STRING.\n\
-expand-path=STRING    output complete path expansion of STRING.\n\
-expand-var=STRING     output variable expansion of STRING.\n\
-format=NAME           use file type NAME (list shown by -help-formats).\n\
-help                  display this message and exit.\n\
-help-formats          display information about all supported file formats.\n\
-interactive           ask for additional filenames to look up.\n\
[-no]-mktex=FMT        disable/enable mktexFMT generation (FMT=pk/mf/tex/tfm).\n\
-mode=STRING           set device name for $MAKETEX_MODE to STRING; no default.\n\
-must-exist            search the disk as well as ls-R if necessary.\n\
-path=STRING           search in the path STRING.\n\
-progname=STRING       set program name to STRING.\n\
-safe-in-name=STRING   check if STRING is ok to open for input.\n\
-safe-out-name=STRING  check if STRING is ok to open for output.\n\
-safe-extended-in-name=STRING   also check TEXMF[SYS]VAR.\n\
-safe-extended-out-name=STRING  also check TEXMF[SYS]VAR.\n\
-show-path=TYPE        output search path for file type TYPE\n\
                         (list shown by -help-formats).\n\
-subdir=STRING         only output matches whose directory ends with STRING.\n\
-var-brace-value=STRING output brace-expanded value of variable STRING.\n\
-var-value=STRING       output variable-expanded value of variable STRING.\n\
-version               display version information number and exit.\n \
";

/// Text-mode stdout (LF -> CR LF), buffered.
struct Out {
    buf: Vec<u8>,
}

impl Out {
    fn write(&mut self, b: &[u8]) {
        for &c in b {
            if c == b'\n' {
                self.buf.push(b'\r');
            }
            self.buf.push(c);
        }
        if self.buf.len() > 1 << 16 {
            self.flush();
        }
    }
    fn puts(&mut self, b: &[u8]) {
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        self.write(&b[..end]);
        self.write(b"\n");
    }
    fn flush(&mut self) {
        let so = std::io::stdout();
        let mut l = so.lock();
        let _ = l.write_all(&self.buf);
        let _ = l.flush();
        self.buf.clear();
    }
}

/// `fprintf (stderr, ...)` through kpathsea's emulated C stderr stream, so
/// that it is ordered with kpathsea's own buffered output exactly as in C.
fn kerr(k: &mut Kpse, b: &[u8]) {
    k.stderr_write(b);
    k.flush_stderr();
}

/// `exit (code)`: the C runtime flushes stdout and stderr.
fn die(k: &mut Kpse, out: &mut Out, code: i32) -> ! {
    out.flush();
    k.flush_stderr();
    std::process::exit(code);
}

/// kpsewhich.c `ENSURE_NONEMPTY_STRING`.
fn ensure_nonempty(k: &mut Kpse, out: &mut Out, name: &str, s: &[u8]) {
    if s.is_empty() {
        kerr(k, format!("kpsewhich: empty argument for {}\n", name).as_bytes());
        kerr(k, b"Try `kpsewhich --help' for more information.\n");
        die(k, out, 1);
    }
}

#[derive(Default)]
struct State {
    var_to_expand: Option<Vec<u8>>,
    braces_to_expand: Option<Vec<u8>>,
    path_to_expand: Option<Vec<u8>>,
    path_to_show: Option<Vec<u8>>,
    var_to_value: Option<Vec<u8>>,
    var_to_brace_value: Option<Vec<u8>>,
    user_cnf_lines: Vec<Vec<u8>>,
    dpi: u32,
    engine: Option<Vec<u8>>,
    interactive: bool,
    mode: Option<Vec<u8>>,
    must_exist: bool,
    progname: Option<Vec<u8>>,
    safe_in_name: Option<Vec<u8>>,
    safe_out_name: Option<Vec<u8>>,
    safe_extended_in_name: Option<Vec<u8>>,
    safe_extended_out_name: Option<Vec<u8>>,
    show_all: bool,
    subdir_paths: Vec<Vec<u8>>,
    user_format: Option<FileFormat>,
    user_format_string: Option<Vec<u8>>,
    user_path: Option<Vec<u8>>,
    nonopts: Vec<Vec<u8>>,
}

/// C `atoi`.
fn atoi(s: &[u8]) -> i32 {
    let mut i = 0;
    while i < s.len() && (s[i] == b' ' || (9..=13).contains(&s[i])) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut n: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        n = (n * 10 + (s[i] - b'0') as i64).min(1 << 40);
        i += 1;
    }
    let n = if neg { -n } else { n };
    n.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// `sscanf (extension, "%u", &dpi)`.
fn scan_u(s: &[u8]) -> u32 {
    let mut i = 0;
    while i < s.len() && (s[i] == b' ' || (9..=13).contains(&s[i])) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let start = i;
    let mut n: u64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        n = n.wrapping_mul(10).wrapping_add((s[i] - b'0') as u64);
        i += 1;
    }
    if i == start {
        return 0;
    }
    let n = n as u32;
    if neg {
        n.wrapping_neg()
    } else {
        n
    }
}

/// kpsewhich.c `format_abbr`.
fn format_abbr(s: &[u8]) -> Option<FileFormat> {
    FORMAT_ABBRS.iter().find(|(a, _)| a.as_bytes() == s).map(|(_, f)| *f)
}

/// kpsewhich.c `try_suffix`.
fn try_suffix(is_filename: bool, name: &[u8], ftry: &[u8]) -> bool {
    if ftry.is_empty() || ftry.len() > name.len() {
        return false;
    }
    if !is_filename && ftry.len() < name.len() {
        return false;
    }
    name[name.len() - ftry.len()..].eq_ignore_ascii_case(ftry)
}

/// kpsewhich.c `find_format`; `None` is `kpse_last_format`.
fn find_format(k: &mut Kpse, st: &State, name: &[u8], is_filename: bool) -> Option<FileFormat> {
    if is_filename && st.user_format.is_some() {
        return st.user_format;
    }
    let ci = |s: &str| name.eq_ignore_ascii_case(s.as_bytes());
    if ci("config.ps") {
        return Some(FileFormat::DvipsConfig);
    } else if ci("fmtutil.cnf") {
        return Some(FileFormat::Web2c);
    } else if ci("glyphlist.txt") {
        return Some(FileFormat::Fontmap);
    } else if ci("mktex.cnf") {
        return Some(FileFormat::Web2c);
    } else if ci("pdfglyphlist.txt") {
        return Some(FileFormat::Fontmap);
    } else if ci("pdftex.cfg") {
        return Some(FileFormat::PdftexConfig);
    } else if ci("texglyphlist.txt") {
        return Some(FileFormat::Fontmap);
    } else if ci("texmf.cnf") {
        return Some(FileFormat::Cnf);
    } else if ci("updmap.cfg") {
        return Some(FileFormat::Web2c);
    } else if ci("XDvi") {
        return Some(FileFormat::ProgramText);
    }
    let mut ret = if !is_filename { format_abbr(name) } else { None };
    if ret.is_none() {
        for &f in FileFormat::all() {
            if k.format_type(f).is_none() {
                k.init_format(f);
            }
            let mut found = false;
            if !is_filename {
                if let Some(t) = k.format_type(f) {
                    found = try_suffix(is_filename, name, t.as_bytes());
                }
            }
            let (suf, alt) = k.format_suffixes(f);
            if !found {
                if let Some(l) = &suf {
                    found = l.iter().any(|e| try_suffix(is_filename, name, e));
                }
            }
            if !found {
                if let Some(l) = &alt {
                    found = l.iter().any(|e| try_suffix(is_filename, name, e));
                }
            }
            if found {
                ret = Some(f);
                break;
            }
        }
    }
    ret
}

/// kpsewhich.c `kpathsea_subdir_match` (WIN32).
fn subdir_match(subdirs: &mut [Vec<u8>], matches: &[Vec<u8>]) -> Vec<Vec<u8>> {
    for sd in subdirs.iter_mut() {
        for c in sd.iter_mut() {
            if *c == b'\\' {
                *c = b'/';
            }
        }
    }
    let mut ret = Vec::new();
    for m in matches {
        let s = m.clone();
        let mut loc = s.len();
        while loc > 0 && s[loc - 1] != b'/' {
            loc -= 1;
        }
        while loc > 0 && s[loc - 1] == b'/' {
            loc -= 1;
        }
        for sd in subdirs.iter_mut() {
            while sd.last() == Some(&b'/') {
                sd.pop();
            }
            let n = sd.len();
            // FILESTRCASEEQ (subdir, s + loc - subdir_len): pointer may
            // precede the string when subdir is longer.
            if n <= loc && s[loc - n..loc].eq_ignore_ascii_case(sd) {
                ret.push(m.clone());
            }
        }
    }
    ret
}

fn help_formats(k: &mut Kpse, st: &State, out: &mut Out, argv0: &[u8]) -> ! {
    k.set_program_name(argv0, st.progname.as_deref());
    out.puts(VERSION_STRING.as_bytes());
    out.puts(b"\nRecognized Kpathsea format names and their (abbreviations) and suffixes:");
    for i in 0..LAST_FORMAT {
        let f = FileFormat::from_index(i).unwrap();
        let envvar_list = k.init_format_return_varlist(f);
        out.write(k.format_type(f).unwrap_or("").as_bytes());
        if let Some((a, _)) = FORMAT_ABBRS.iter().find(|(_, ff)| *ff == f) {
            out.write(format!(" ({})", a).as_bytes());
        }
        out.write(b":");
        let (suf, alt) = k.format_suffixes(f);
        if let Some(l) = &suf {
            for e in l {
                out.write(b" ");
                out.write(e);
            }
        }
        if let Some(l) = &alt {
            out.write(b" ");
            for e in l {
                out.write(b" ");
                out.write(e);
            }
        }
        out.write(b"  [variables: ");
        out.write(&envvar_list);
        out.write(b"]\n");
        let (src, raw) = k.format_origin(f);
        out.write(b"  [original path (from ");
        out.write(&src);
        out.write(b") = ");
        out.write(&raw);
        out.write(b"]\n");
    }
    out.write(b"\nTo see paths after expansion, use --show-path=FMT.\n\n");
    out.write(BUG_ADDRESS.as_bytes());
    out.flush();
    die(k, out, 0);
}

/// kpsewhich.c `read_command_line`, with GNU getopt_long_only semantics
/// (abbreviations, `-opt value`, permutation of non-options).
fn read_command_line(k: &mut Kpse, st: &mut State, args: &[Vec<u8>], out: &mut Out) {
    let argv0 = args[0].clone();
    st.nonopts.clear();
    let mut i = 1;
    let mut only_nonopts = false;
    while i < args.len() {
        let a = &args[i];
        if only_nonopts || a.first() != Some(&b'-') || a.len() == 1 {
            st.nonopts.push(a.clone());
            i += 1;
            continue;
        }
        if a.as_slice() == b"--" {
            only_nonopts = true;
            i += 1;
            continue;
        }
        let dd = a.len() > 1 && a[1] == b'-';
        let body = &a[if dd { 2 } else { 1 }..];
        let eq = body.iter().position(|&c| c == b'=');
        let nm = &body[..eq.unwrap_or(body.len())];
        let mut exact: Option<usize> = None;
        let mut pfound: Option<usize> = None;
        let mut ambig = false;
        for (idx, (name, _)) in LONG_OPTIONS.iter().enumerate() {
            if name.as_bytes().starts_with(nm) {
                if name.len() == nm.len() {
                    exact = Some(idx);
                    break;
                } else if pfound.is_none() {
                    pfound = Some(idx);
                } else {
                    ambig = true;
                }
            }
        }
        let a0 = String::from_utf8_lossy(&argv0).into_owned();
        if ambig && exact.is_none() {
            kerr(k, format!("{}: option `{}' is ambiguous\n", a0, String::from_utf8_lossy(a)).as_bytes());
            die(k, out, 1);
        }
        let idx = match exact.or(pfound) {
            Some(x) => x,
            None => {
                let m = if dd {
                    format!("{}: unrecognized option `--{}'\n", a0, String::from_utf8_lossy(body))
                } else {
                    format!("{}: unrecognized option `-{}'\n", a0, String::from_utf8_lossy(body))
                };
                kerr(k, m.as_bytes());
                die(k, out, 1);
            }
        };
        let (name, has_arg) = LONG_OPTIONS[idx];
        i += 1;
        let optarg: Option<Vec<u8>> = if let Some(e) = eq {
            if has_arg {
                Some(body[e + 1..].to_vec())
            } else {
                let m = if dd {
                    format!("{}: option `--{}' doesn't allow an argument\n", a0, name)
                } else {
                    format!("{}: option `-{}' doesn't allow an argument\n", a0, name)
                };
                kerr(k, m.as_bytes());
                die(k, out, 1);
            }
        } else if has_arg {
            if i < args.len() {
                i += 1;
                Some(args[i - 1].clone())
            } else {
                kerr(k, format!("{}: option `{}' requires an argument\n", a0, String::from_utf8_lossy(a)).as_bytes());
                die(k, out, 1);
            }
        } else {
            None
        };
        let arg = optarg.unwrap_or_default();
        match name {
            "all" => st.show_all = true,
            "interactive" => st.interactive = true,
            "must-exist" => st.must_exist = true,
            "casefold-search" => k.xputenv("texmf_casefold_search", "1"),
            "cnf-line" => st.user_cnf_lines.push(arg),
            "debug" => k.set_debug(atoi(&arg) as u32),
            "dpi" | "D" => st.dpi = atoi(&arg) as u32,
            "engine" => st.engine = Some(arg),
            "expand-braces" => st.braces_to_expand = Some(arg),
            "expand-path" => st.path_to_expand = Some(arg),
            "expand-var" => st.var_to_expand = Some(arg),
            "format" => {
                ensure_nonempty(k, out, name, &arg);
                st.user_format_string = Some(arg);
            }
            "help" => {
                out.write(format!("Usage: {} [OPTION]... [FILENAME]...\n", a0).as_bytes());
                out.write(USAGE.as_bytes());
                out.write(b"\n");
                out.write(BUG_ADDRESS.as_bytes());
                out.write(b"Kpathsea home page: https://tug.org/kpathsea/\n");
                out.flush();
                die(k, out, 0);
            }
            "help-formats" => help_formats(k, st, out, &argv0),
            "mktex" => {
                k.maketex_option(&arg, true);
                st.must_exist = true;
            }
            "mode" => st.mode = Some(arg),
            "no-casefold-search" => k.xputenv("texmf_casefold_search", "0"),
            "no-mktex" => {
                k.maketex_option(&arg, false);
                st.must_exist = false;
            }
            "path" => st.user_path = Some(arg),
            "progname" => st.progname = Some(arg),
            "safe-extended-in-name" => {
                ensure_nonempty(k, out, name, &arg);
                st.safe_extended_in_name = Some(arg);
            }
            "safe-extended-out-name" => {
                ensure_nonempty(k, out, name, &arg);
                st.safe_extended_out_name = Some(arg);
            }
            "safe-in-name" => {
                ensure_nonempty(k, out, name, &arg);
                st.safe_in_name = Some(arg);
            }
            "safe-out-name" => {
                ensure_nonempty(k, out, name, &arg);
                st.safe_out_name = Some(arg);
            }
            "show-path" => {
                st.path_to_show = Some(arg.clone());
                st.user_format_string = Some(arg);
            }
            "subdir" => st.subdir_paths.push(arg),
            "var-brace-value" => {
                ensure_nonempty(k, out, name, &arg);
                st.var_to_brace_value = Some(arg);
            }
            "var-value" => {
                ensure_nonempty(k, out, name, &arg);
                st.var_to_value = Some(arg);
            }
            "version" => {
                out.puts(VERSION_STRING.as_bytes());
                out.puts(
                    b"Copyright 2026 Karl Berry & Olaf Weber.\n\
License LGPLv2.1+: GNU Lesser GPL version 2.1 or later <https://gnu.org/licenses/lgpl.html>\n\
This is free software: you are free to change and redistribute it.\n\
There is NO WARRANTY, to the extent permitted by law.\n",
                );
                out.flush();
                die(k, out, 0);
            }
            _ => {}
        }
    }
    if let (Some(p), Some(f)) = (&st.user_path, &st.user_format_string) {
        kerr(k, 
            format!(
                "kpsewhich: -path ({}) and -format ({}) are mutually exclusive.\n",
                String::from_utf8_lossy(p),
                String::from_utf8_lossy(f)
            )
            .as_bytes(),
        );
        kerr(k, b"Try `kpsewhich --help' for more information.\n");
        die(k, out, 1);
    }
    if st.nonopts.is_empty()
        && st.var_to_expand.is_none()
        && st.braces_to_expand.is_none()
        && st.path_to_expand.is_none()
        && st.path_to_show.is_none()
        && st.var_to_value.is_none()
        && st.var_to_brace_value.is_none()
        && st.safe_extended_in_name.is_none()
        && st.safe_extended_out_name.is_none()
        && st.safe_in_name.is_none()
        && st.safe_out_name.is_none()
    {
        kerr(k, b"Missing argument. Try `kpsewhich --help' for more information.\n");
        die(k, out, 1);
    }
}

/// kpsewhich.c `init_more`.
fn init_more(k: &mut Kpse, st: &mut State) {
    if let Some(e) = &st.engine {
        k.xputenv("engine", e);
    }
    for l in st.user_cnf_lines.clone() {
        k.add_cnf_line(&l);
    }
    for f in [
        FileFormat::Pk,
        FileFormat::Mf,
        FileFormat::Tex,
        FileFormat::Tfm,
        FileFormat::Fmt,
        FileFormat::Ofm,
        FileFormat::Ocp,
    ] {
        k.set_program_enabled(f, false, SrcLevel::X);
    }
    let prefix = String::from_utf8_lossy(&k.program_name().to_ascii_uppercase()).into_owned();
    let mode = st.mode.as_ref().map(|m| String::from_utf8_lossy(m).into_owned());
    k.init_prog(&prefix, st.dpi, mode.as_deref(), None);
    if let Some(ufs) = st.user_format_string.clone() {
        st.user_format = find_format(k, st, &ufs, false);
        if st.user_format.is_none() {
            let m = format!(
                "warning: kpsewhich: Ignoring unknown file type `{}'.\n",
                String::from_utf8_lossy(&ufs)
            );
            kerr(k, m.as_bytes());
        }
    }
}

/// kpsewhich.c `lookup`; returns true if not found.
fn lookup(k: &mut Kpse, st: &mut State, name: &[u8], out: &mut Out) -> bool {
    let mut ret_list: Vec<Vec<u8>> = Vec::new();
    if let Some(up) = st.user_path.clone() {
        let p = k.path_expand(&up);
        st.user_path = Some(p.clone());
        if st.show_all {
            ret_list = k.all_path_search(&p, name);
        } else if let Some(r) = k.path_search_first(&p, name, st.must_exist) {
            ret_list.push(r);
        }
    } else {
        let fmt = find_format(k, st, name, true);
        match fmt {
            Some(FileFormat::Pk) | Some(FileFormat::Gf) | Some(FileFormat::AnyGlyph) => {
                let f = fmt.unwrap();
                let temp = Kpse::remove_suffix(name);
                let mut local_dpi = Kpse::find_suffix(name).map(scan_u).unwrap_or(0);
                if local_dpi == 0 {
                    local_dpi = st.dpi;
                }
                if st.show_all {
                    let m = format!(
                        "warning: kpsewhich: Ignoring --all for bitmap font, sorry: {}.\n",
                        String::from_utf8_lossy(name)
                    );
                    kerr(k, m.as_bytes());
                }
                if let Some((r, _)) = k.find_glyph(&temp, local_dpi, f) {
                    ret_list.push(r);
                }
            }
            _ => {
                let f = fmt.unwrap_or(FileFormat::Tex);
                if st.show_all {
                    ret_list = k.find_file_generic(name, f, st.must_exist, true);
                } else if let Some(r) = k.find_file(name, f, st.must_exist) {
                    ret_list.push(r);
                }
            }
        }
    }
    if !st.subdir_paths.is_empty() {
        ret_list = subdir_match(&mut st.subdir_paths, &ret_list);
    }
    for r in &ret_list {
        out.puts(r);
    }
    ret_list.is_empty()
}

fn main() {
    let mut k = Kpse::blank();
    k.set_compat_def(false);
    let mut out = Out { buf: Vec::new() };
    let mut st = State { dpi: 600, ..Default::default() };

    // First pass: arguments in the ANSI code page, like the CRT's argv.
    let acp_args: Vec<Vec<u8>> = std::env::args_os().map(|a| k.os_to_bytes(&a)).collect();
    read_command_line(&mut k, &mut st, &acp_args, &mut out);
    k.set_program_name(&acp_args[0], st.progname.clone().as_deref());

    // WIN32: re-read the command line as UTF-8 for these programs.
    let pn = k.program_name().to_vec();
    let has = |s: &str| pn.windows(s.len()).any(|w| w == s.as_bytes());
    let mut args = acp_args.clone();
    if cfg!(windows)
        && (has("xetex")
            || has("xelatex")
            || has("uptex")
            || has("uplatex")
            || has("pdftex")
            || has("pdflatex")
            || has("dvipdfm")
            || has("extractbb")
            || has("xbb")
            || has("ebb")
            || has("dvips")
            || has("upmendex")
            || has("bibtex8")
            || has("bibtexu"))
    {
        let utf8 = if has("upmendex") {
            k.set_file_system_codepage(65001);
            true
        } else {
            k.init_command_line_encoding()
        };
        if utf8 {
            args = std::env::args_os().map(|a| a.to_string_lossy().into_owned().into_bytes()).collect();
            read_command_line(&mut k, &mut st, &args, &mut out);
        }
    }
    let _ = args;
    init_more(&mut k, &mut st);

    let mut unfound: u32 = 0;
    if let Some(v) = st.var_to_expand.clone() {
        let r = k.var_expand(&v);
        out.puts(&r);
    }
    if let Some(v) = st.braces_to_expand.clone() {
        let r = k.brace_expand(&v);
        out.puts(&r);
    }
    if let Some(v) = st.path_to_expand.clone() {
        let r = k.path_expand(&v);
        out.puts(&r);
    }
    if let Some(p) = st.path_to_show.clone() {
        match st.user_format {
            Some(f) => {
                let path = k.format_path(f);
                out.puts(&path);
            }
            None => {
                let m = format!(
                    "warning: kpsewhich: Unknown file type, cannot show path: {}.\n",
                    String::from_utf8_lossy(&p)
                );
                out.flush();
                kerr(&mut k, m.as_bytes());
            }
        }
    }
    if let Some(v) = st.var_to_value.clone() {
        match k.var_value(&v) {
            Some(val) => out.puts(&val),
            None => {
                unfound += 1;
                out.puts(b"");
            }
        }
    }
    if let Some(v) = st.var_to_brace_value.clone() {
        match k.var_value(&v) {
            Some(val) => {
                let b = k.brace_expand(&val);
                out.puts(&b);
            }
            None => {
                unfound += 1;
                out.puts(b"");
            }
        }
    }
    if let Some(n) = st.safe_in_name.clone() {
        if !k.in_name_ok_silent(&n) {
            unfound += 1;
        }
    }
    if let Some(n) = st.safe_out_name.clone() {
        if !k.out_name_ok_silent(&n) {
            unfound += 1;
        }
    }
    if let Some(n) = st.safe_extended_in_name.clone() {
        if !k.in_name_ok_silent_extended(&n) {
            unfound += 1;
        }
    }
    if let Some(n) = st.safe_extended_out_name.clone() {
        if !k.out_name_ok_silent_extended(&n) {
            unfound += 1;
        }
    }
    if !st.subdir_paths.is_empty() {
        st.show_all = true;
    }
    let names = st.nonopts.clone();
    for (i, n) in names.iter().enumerate() {
        let not_found = lookup(&mut k, &mut st, n, &mut out);
        if not_found {
            unfound += 1;
            if i + 1 < names.len() {
                out.puts(b"");
            }
        }
    }
    if st.interactive {
        out.flush();
        let stdin = std::io::stdin();
        let mut lock = stdin.lock();
        loop {
            let mut line = Vec::new();
            match lock.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            while matches!(line.last(), Some(b'\n') | Some(b'\r')) {
                line.pop();
            }
            if line == b"q" || line == b"quit" {
                break;
            }
            if lookup(&mut k, &mut st, &line, &mut out) {
                unfound += 1;
            }
            out.flush();
        }
    }
    out.flush();
    die(&mut k, &mut out, if unfound > 255 { 1 } else { unfound as i32 });
}
