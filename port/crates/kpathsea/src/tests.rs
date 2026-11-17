//! Unit tests for the self-contained helpers (no TeX installation needed).

use crate::db::{db_match, elt_in_db, ignore_dir_p};
use crate::eltdirs::{DirElt, DirList};
use crate::expand::expand_default;
use crate::hash::{c_hash, c_hash_normalized, CHash};
use crate::util::*;

#[test]
fn absolute_names() {
    assert!(absolute_p(b"/foo", false));
    assert!(absolute_p(b"\\foo", false));
    assert!(absolute_p(b"c:foo", false));
    assert!(absolute_p(b"C:/foo", false));
    assert!(absolute_p(b"//server/share", false));
    assert!(!absolute_p(b"./foo", false));
    assert!(absolute_p(b"./foo", true));
    assert!(absolute_p(b"../foo", true));
    assert!(absolute_p(b".\\foo", true));
    assert!(!absolute_p(b"..foo", true));
    assert!(!absolute_p(b"foo/bar", true));
    assert!(!absolute_p(b"", true));
}

#[test]
fn suffixes() {
    assert_eq!(find_suffix(b"foo.tex"), Some(4));
    assert_eq!(find_suffix(b"/a.b/foo"), None);
    assert_eq!(find_suffix(b"a.b\\foo"), None);
    assert_eq!(remove_suffix(b"dir/foo.tex"), b"dir/foo".to_vec());
    assert_eq!(remove_suffix(b"foo"), b"foo".to_vec());
    assert_eq!(extend_filename(b"cmr10", b"tfm"), b"cmr10.tfm".to_vec());
    assert_eq!(extend_filename(b"cmr10.vf", b"tfm"), b"cmr10.vf".to_vec());
    assert_eq!(make_suffix(b"a\\b.c", b"d"), b"a/b.d".to_vec());
}

#[test]
fn dir_and_base_names() {
    assert_eq!(xdirname(b"c:/foo/bar"), b"c:/foo".to_vec());
    assert_eq!(xdirname(b"c:/foo"), b"c:/".to_vec());
    assert_eq!(xdirname(b"c:foo"), b"c:.".to_vec());
    assert_eq!(xdirname(b"foo"), b".".to_vec());
    assert_eq!(xdirname(b"/foo"), b"/".to_vec());
    assert_eq!(xdirname(b"///a"), b"/".to_vec());
    assert_eq!(xdirname(b"a\\b\\c"), b"a/b".to_vec());
    assert_eq!(xdirname(b"//server/share/x"), b"//server/share".to_vec());
    assert_eq!(xdirname(b"//server/share"), b"//server/share".to_vec());
    assert_eq!(xbasename(b"c:foo"), b"foo");
    assert_eq!(xbasename(b"a/b\\c.tex"), b"c.tex");
    assert_eq!(xbasename(b"//server/share/x"), b"x");
}

#[test]
fn c_line_reading() {
    let data = text_mode(b"a\r\nb\rc\n\nd\0e\r\r\nf\x1ag".to_vec());
    let mut l = Lines::new(&data);
    let mut got = Vec::new();
    while let Some(x) = l.next_line() {
        got.push(x.to_vec());
    }
    let want: Vec<&[u8]> = vec![b"a", b"b", b"c", b"", b"de", b"f"];
    assert_eq!(got, want.iter().map(|x| x.to_vec()).collect::<Vec<_>>());
}

#[test]
fn numbers() {
    assert_eq!(atoi(b"  42xyz"), 42);
    assert_eq!(atoi(b"-7"), -7);
    assert_eq!(atoi(b"x"), 0);
    assert!(cnf_p(Some(b"t")));
    assert!(cnf_p(Some(b"2")));
    assert!(!cnf_p(Some(b"false")));
    assert!(!cnf_p(Some(b"0")));
    assert!(!cnf_p(Some(b"")));
    assert!(!cnf_p(None));
}

#[test]
fn expand_default_cases() {
    // From the TEST main of kdefault.c.
    assert_eq!(expand_default(None, b"default"), b"default".to_vec());
    assert_eq!(expand_default(Some(b""), b"default"), b"default".to_vec());
    assert_eq!(expand_default(Some(b"none"), b"default"), b"none".to_vec());
    assert_eq!(expand_default(Some(b";"), b"default"), b"default".to_vec());
    assert_eq!(expand_default(Some(b";first"), b"default"), b"default;first".to_vec());
    assert_eq!(expand_default(Some(b"last;"), b"default"), b"last;default".to_vec());
    assert_eq!(
        expand_default(Some(b"middle;;elddim"), b"default"),
        b"middle;default;elddim".to_vec()
    );
}

#[test]
fn db_matching() {
    assert!(db_match(b"c:/t/fonts/vf/ptmr.vf", b"c:/t/fonts/vf"));
    assert!(db_match(b"c:/t/fonts/vf/ptmr.vf", b"c:/t/fonts/vf/"));
    assert!(db_match(b"c:/t/fonts/vf/a/b/ptmr.vf", b"c:/t/fonts//"));
    assert!(db_match(b"c:/t/fonts/vf/a/b/ptmr.vf", b"c:/t/fonts//b"));
    assert!(!db_match(b"c:/t/fonts/vf/a/b/ptmr.vf", b"c:/t/fonts//a"));
    assert!(!db_match(b"c:/t/fonts/vfoo.ext", b"c:/t/fonts/vf"));
    assert!(db_match(b"C:/T/Fonts/vf/x", b"c:/t/fonts/VF"));
    assert!(db_match(b"ptmr.vf", b""));
    assert!(!db_match(b"foo/ptmr.vf", b""));
    assert!(elt_in_db(b"c:/tl/texmf-dist/", b"c:/tl/texmf-dist/tex//"));
    assert!(elt_in_db(b"c:/tl/texmf-dist/", b"C:/TL/texmf-dist/"));
    assert!(!elt_in_db(b"c:/tl/texmf-dist/", b"c:/tl/texmf-dist"));
    assert!(!elt_in_db(b"", b"x"));
    assert!(ignore_dir_p(b"./tex/.git:"));
    assert!(!ignore_dir_p(b"./tex/latex:"));
    assert!(!ignore_dir_p(b"./tex/a.b:"));
}

#[test]
fn c_hash_layout() {
    // n = (2n + c) % size, unsigned for hash(), signed char for
    // hash_normalized().
    assert_eq!(c_hash(751, b"a"), 97);
    assert_eq!(c_hash(751, b"ab"), (2 * 97 + 98) % 751);
    assert_eq!(c_hash(751, b"AB"), c_hash(751, b"ab"));
    assert_eq!(c_hash_normalized(751, b"ab"), c_hash(751, b"ab"));
    assert_ne!(c_hash_normalized(64007, b"\xc3\xa9"), c_hash(64007, b"\xc3\xa9"));
    let mut h = CHash::new(751);
    h.insert(b"VAR".to_vec(), b"first".to_vec());
    h.insert(b"var".to_vec(), b"second".to_vec());
    assert_eq!(h.lookup(b"Var"), vec![&b"first"[..], &b"second"[..]]);
    assert_eq!(h.lookup_first(b"VAR"), Some(&b"first"[..]));
    // Upper-case keys inserted "normalized" land in a bucket the
    // case-folding lookup does not search.
    let mut m = CHash::new(4001);
    m.insert_normalized(b"Times".to_vec(), b"ptmr".to_vec());
    m.insert_normalized(b"times".to_vec(), b"ptmr2".to_vec());
    assert_eq!(m.lookup(b"times"), vec![&b"ptmr2"[..]]);
}

#[test]
fn printf_rounding() {
    assert_eq!(crate::debug::fmt_f1(1.0), "1.0");
    assert_eq!(crate::debug::fmt_f1(1.94), "1.9");
    assert_eq!(crate::debug::fmt_f1(60958.0 / 31492.0), "1.9");
    assert_eq!(crate::debug::fmt_f1(319.0 / 151.0), "2.1");
    assert_eq!(crate::debug::fmt_f1(0.0), "0.0");
}

#[test]
fn magsteps() {
    use crate::glyph::{bitmap_tolerance, magstep_fix};
    // The C expression compares the dpi with a magstep number, so the
    // magstep returned is (almost) always 0; the true dpi is right.
    assert_eq!(magstep_fix(600, 600), (600, 0));
    assert_eq!(magstep_fix(720, 600), (720, 0));
    assert_eq!(magstep_fix(719, 600), (720, 0));
    assert_eq!(magstep_fix(657, 600), (657, 0));
    assert!(bitmap_tolerance(601.0, 600.0));
    assert!(bitmap_tolerance(598.0, 600.0));
    assert!(!bitmap_tolerance(603.0, 600.0));
}

#[test]
fn llist_float() {
    let mk = |names: &[&str]| DirList {
        key: Vec::new(),
        dirs: names.iter().map(|n| DirElt { name: n.as_bytes().to_vec(), moved: false }).collect(),
    };
    let names = |l: &DirList| l.dirs.iter().map(|d| String::from_utf8(d.name.clone()).unwrap()).collect::<Vec<_>>();
    let mut l = mk(&["a", "b", "c", "d"]);
    l.float(2);
    assert_eq!(names(&l), ["c", "a", "b", "d"]);
    l.float(3);
    assert_eq!(names(&l), ["c", "d", "a", "b"]);
    l.float(0); // already moved
    assert_eq!(names(&l), ["c", "d", "a", "b"]);
}

#[test]
fn format_numbering() {
    use crate::FileFormat;
    assert_eq!(FileFormat::Gf as usize, 0);
    assert_eq!(FileFormat::Tfm as usize, 3);
    assert_eq!(FileFormat::Cnf as usize, 8);
    assert_eq!(FileFormat::Tex as usize, 26);
    assert_eq!(FileFormat::ProgramText as usize, 39);
    assert_eq!(FileFormat::Bltxml as usize, 58);
    assert_eq!(crate::LAST_FORMAT, 59);
    for (i, f) in FileFormat::all().iter().enumerate() {
        assert_eq!(f.index(), i);
        assert_eq!(FileFormat::from_index(i), Some(*f));
    }
}
