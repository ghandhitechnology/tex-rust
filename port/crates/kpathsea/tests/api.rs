//! Integration tests against the TeX Live 2026 installation in
//! `<repo>/tools/texlive/2026` (skipped, with a message, if it is absent).
//! Results are cross-checked with TeX Live's own kpsewhich.exe.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use texrs_kpathsea::{FileFormat, GlyphSource, Kpse, SrcLevel};

fn tl_bin() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tools/texlive/2026/bin/windows");
    let p = p.canonicalize().ok()?;
    if p.join("kpsewhich.exe").is_file() {
        // canonicalize() gives a \\?\ path; strip it like a normal path.
        let s = p.to_string_lossy().trim_start_matches(r"\\?\").to_string();
        Some(PathBuf::from(s))
    } else {
        None
    }
}

macro_rules! need_tl {
    () => {
        match tl_bin() {
            Some(p) => p,
            None => {
                eprintln!("TeX Live 2026 not found under tools/texlive; skipping");
                return;
            }
        }
    };
}

/// An instance that pretends to be `<tl bin>/<name>.exe`.
fn kpse(bin: &Path, name: &str, progname: Option<&str>) -> Kpse {
    let exe = bin.join(format!("{}.exe", name));
    Kpse::with_exe_path(exe.to_string_lossy().as_bytes(), progname)
}

fn c_kpsewhich(bin: &Path, args: &[&str]) -> String {
    let out = Command::new(bin.join("kpsewhich.exe")).args(args).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

fn s(v: Option<Vec<u8>>) -> String {
    v.map(|v| String::from_utf8_lossy(&v).into_owned()).unwrap_or_default()
}

#[test]
fn finds_the_same_files_as_kpsewhich() {
    let bin = need_tl!();
    let mut k = kpse(&bin, "pdftex", Some("pdflatex"));
    k.init_command_line_encoding();
    k.xputenv("engine", "pdftex");
    let got = [
        s(k.find_file("cmr10.tfm", FileFormat::Tfm, true)),
        s(k.find_file("article.cls", FileFormat::Tex, true)),
        s(k.find_file("pdftex.map", FileFormat::Fontmap, false)),
        s(k.find_file("pdflatex.fmt", FileFormat::Fmt, true)),
        s(k.find_file("ARTICLE.CLS", FileFormat::Tex, true)),
        s(k.find_file("cmr10", FileFormat::Tfm, false)),
    ];
    let want = c_kpsewhich(
        &bin,
        &["-progname=pdflatex", "-engine=pdftex", "cmr10.tfm", "article.cls", "pdftex.map", "pdflatex.fmt", "ARTICLE.CLS", "cmr10.tfm"],
    );
    let want: Vec<&str> = want.lines().collect();
    assert_eq!(got.to_vec(), want);
    assert!(got[0].ends_with("/fonts/tfm/public/cm/cmr10.tfm"));
    // On-disk case is restored by the long-name conversion.
    assert!(got[4].ends_with("/tex/latex/base/article.cls"));
    assert!(k.find_file("surely-nonexistent-file.sty", FileFormat::Tex, false).is_none());
}

#[test]
fn variables_match_kpsewhich() {
    let bin = need_tl!();
    let mut k = kpse(&bin, "kpsewhich", None);
    for v in ["TEXMFDIST", "TEXMFHOME", "TEXINPUTS", "SELFAUTOPARENT", "openout_any", "TEXMFCNF"] {
        let mine = s(k.var_value(v));
        let theirs = c_kpsewhich(&bin, &[&format!("-var-value={}", v)]);
        assert_eq!(mine, theirs, "variable {}", v);
    }
    assert_eq!(k.var_value("surely_unset_variable_xyz"), None);
    assert_eq!(k.var_expand("$NONEXISTENT/x"), b"$NONEXISTENT/x".to_vec());
    assert_eq!(k.brace_expand("{a,b}{x,y}"), b"ax;bx;ay;by".to_vec());
}

#[test]
fn recorder_sees_config_files() {
    let bin = need_tl!();
    let seen: Rc<RefCell<Vec<String>>> = Rc::default();
    let s2 = seen.clone();
    let mut k = kpse(&bin, "pdftex", None);
    k.set_record_input(Some(Box::new(move |n: &[u8]| s2.borrow_mut().push(String::from_utf8_lossy(n).into_owned()))));
    k.find_file("cmr10", FileFormat::Tfm, true);
    let seen = seen.borrow();
    assert!(seen.iter().any(|n| n.ends_with("web2c/texmf.cnf")), "{:?}", seen);
    assert!(seen.iter().any(|n| n.ends_with("texfonts.map")), "{:?}", seen);
}

#[test]
fn program_name_and_reset() {
    let bin = need_tl!();
    let mut k = kpse(&bin, "pdftex", None);
    assert_eq!(k.program_name(), b"pdftex");
    assert_eq!(k.invocation_name(), b"pdftex.exe");
    let p1 = k.format_path(FileFormat::Tex);
    k.reset_program_name("pdflatex");
    assert_eq!(k.program_name(), b"pdflatex");
    let p2 = k.format_path(FileFormat::Tex);
    assert_ne!(p1, p2);
    assert!(String::from_utf8_lossy(&p2).contains("tex/latex//"));
    assert_eq!(s(k.var_value("progname")), "pdflatex");
    let loc = s(k.var_value("SELFAUTOLOC"));
    assert_eq!(loc.to_lowercase(), bin.to_string_lossy().replace('\\', "/").to_lowercase());
}

#[test]
fn cnf_lines_and_name_checks() {
    let bin = need_tl!();
    let mut k = kpse(&bin, "kpsewhich", None);
    k.add_cnf_line("FOO_FOR_TEST = some value % comment");
    assert_eq!(s(k.var_value("FOO_FOR_TEST")), "some value");
    assert_eq!(s(k.getenv("FOO_FOR_TEST_kpsewhich")), "some value");
    assert!(k.in_name_ok_silent("../anything"));
    assert!(k.out_name_ok_silent("out.tex"));
    assert!(!k.out_name_ok_silent("../out.tex"));
    assert!(!k.out_name_ok_silent("sub/../out.tex"));
    assert!(!k.out_name_ok_silent("c:/tmp/x.log"));
    assert!(!k.out_name_ok_silent("evil.EXE"));
    assert!(!k.out_name_ok_silent("evil.bat. "));
    k.add_cnf_line("openout_any=a");
    assert!(k.out_name_ok_silent("c:/tmp/x.log"));
    assert!(Kpse::absolute_p("c:/x", false));
    assert!(Kpse::absolute_p("./x", true));
    assert!(!Kpse::absolute_p("./x", false));
}

#[test]
fn glyph_lookup_with_tolerance_and_fallback() {
    let bin = need_tl!();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("glyphs");
    let _ = std::fs::create_dir_all(dir.join("dpi300"));
    std::fs::write(dir.join("fxr.600pk"), b"x").unwrap();
    std::fs::write(dir.join("dpi300").join("fxr.pk"), b"x").unwrap();
    let mut k = kpse(&bin, "kpsewhich", None);
    let d = dir.to_string_lossy().replace('\\', "/");
    k.xputenv("PKFONTS", format!("{};", d));
    k.init_prog("KPSEWHICH", 600, None, Some("fxr"));
    let (p, g) = k.find_glyph("fxr", 600, FileFormat::Pk).unwrap();
    assert!(String::from_utf8_lossy(&p).ends_with("fxr.600pk"));
    assert_eq!((g.dpi, g.format, g.source), (600, FileFormat::Pk, GlyphSource::Normal));
    let (_, g) = k.find_glyph("fxr", 601, FileFormat::Pk).unwrap();
    assert_eq!(g.dpi, 600);
    let (p, g) = k.find_glyph("fxr", 300, FileFormat::Pk).unwrap();
    assert!(String::from_utf8_lossy(&p).ends_with("dpi300/fxr.pk"));
    assert_eq!(g.dpi, 300);
    // Unknown font: the fallback font is used.
    let (p, g) = k.find_glyph("nosuchfont", 600, FileFormat::Pk).unwrap();
    assert!(String::from_utf8_lossy(&p).ends_with("fxr.600pk"));
    assert_eq!(g.source, GlyphSource::Fallback);
}

#[test]
fn mktex_switches() {
    let bin = need_tl!();
    let mut k = kpse(&bin, "kpsewhich", None);
    k.set_program_enabled(FileFormat::Tfm, false, SrcLevel::Cmdline);
    // A lower level cannot override.
    k.set_program_enabled(FileFormat::Tfm, true, SrcLevel::Compile);
    k.maketex_option("pk", false);
    // Nothing is created for a missing font when disabled.
    assert!(k.find_file("nosuchfontxyz", FileFormat::Tfm, true).is_none());
}

#[test]
fn independent_instances() {
    let bin = need_tl!();
    let mut a = kpse(&bin, "pdftex", None);
    let mut b = kpse(&bin, "kpsewhich", None);
    a.xputenv("ONLY_IN_A", "1");
    assert_eq!(s(a.var_value("ONLY_IN_A")), "1");
    assert_eq!(b.var_value("ONLY_IN_A"), None);
}

/// A compact differential run of the rkpsewhich binary against
/// kpsewhich.exe (the full suite is tests/difftest.py).
#[test]
fn rkpsewhich_matches_kpsewhich() {
    let bin = need_tl!();
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("rbin");
    std::fs::create_dir_all(&tmp).unwrap();
    let rk = tmp.join("kpsewhich.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_rkpsewhich"), &rk).unwrap();
    let queries: &[&[&str]] = &[
        &["cmr10.tfm"],
        &["-progname=pdflatex", "-engine=pdftex", "article.cls", "pdflatex.fmt", "nonexistent.sty"],
        &["-all", "texmf.cnf"],
        &["-show-path=tex"],
        &["-var-value=TEXMF"],
        &["-expand-braces={a,b}{x,y}"],
        &["-expand-path=$TEXMFDIST/fonts/map//"],
        &["-format=type1", "cmr10"],
        &["-safe-out-name=../x"],
        &["-debug=32", "cmr10.tfm"],
        &["-debug=8", "-progname=pdflatex", "-show-path=fmt"],
        &["cmr10.600pk"],
    ];
    for q in queries {
        let c = Command::new(bin.join("kpsewhich.exe")).args(*q).output().unwrap();
        let r = Command::new(&rk).args(*q).env("TEXRS_SELFAUTOLOC", &bin).output().unwrap();
        assert_eq!(c.status.code(), r.status.code(), "status for {:?}", q);
        assert_eq!(String::from_utf8_lossy(&c.stdout), String::from_utf8_lossy(&r.stdout), "stdout for {:?}", q);
        assert_eq!(String::from_utf8_lossy(&c.stderr), String::from_utf8_lossy(&r.stderr), "stderr for {:?}", q);
    }
}
