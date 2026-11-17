//! Shared helpers for the integration tests: corpus discovery, temp
//! directories and running the TeX Live binaries (`pdftosrc.exe`,
//! `pdftex.exe`) that are built on the real xpdf 4.06 and serve as the
//! oracle.

#![allow(dead_code)]

pub mod builder;
pub mod epdf;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use pdfread::{Object, PdfDoc, XRefEntryType};

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

pub fn tl_bin(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{}.exe", name) } else { name.to_string() };
    let p = repo_root().join("tools/texlive/2026/bin/windows").join(&exe);
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

pub fn collect_pdfs(dir: &Path, out: &mut Vec<PathBuf>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for e in rd.flatten() {
        let p = e.path();
        let ft = match e.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_dir() {
            collect_pdfs(&p, out);
        } else if p.extension().map(|x| x.eq_ignore_ascii_case("pdf")).unwrap_or(false) {
            out.push(p);
        }
    }
}

pub fn corpus() -> Vec<PathBuf> {
    let root = repo_root();
    let mut files = Vec::new();
    collect_pdfs(&root.join("tools/texlive/2026/texmf-dist"), &mut files);
    collect_pdfs(&root.join("bench/out"), &mut files);
    files.sort();
    files
}

static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh private temp directory, removed on drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let n = TMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("pdfread-{}-{}-{}", tag, std::process::id(), n));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Run `pdftosrc <dir>/a.pdf <arg>` on a copy of `pdf` and return the
/// produced file (xref dump for arg -1, decoded stream otherwise).
pub fn pdftosrc(pdftosrc: &Path, tmp: &Path, pdf_bytes: &[u8], arg: i32, gen: i32) -> Option<Vec<u8>> {
    let a = tmp.join("a.pdf");
    if !a.exists() {
        std::fs::write(&a, pdf_bytes).unwrap();
    }
    let mut cmd = Command::new(pdftosrc);
    cmd.arg(&a).arg(arg.to_string());
    if gen != 0 {
        cmd.arg(gen.to_string());
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let name = if arg < 0 {
        "a.xref".to_string()
    } else if gen == 0 {
        format!("a.{}", arg)
    } else {
        format!("a.{}+{}", arg, gen)
    };
    let p = tmp.join(name);
    let r = std::fs::read(&p).ok();
    let _ = std::fs::remove_file(&p);
    r
}

/// The xref dump `pdftosrc file.pdf -1` writes, computed from our XRef
/// (mirrors the code in pdftosrc.cc, including the 32-bit `unsigned
/// long` of the Windows build).
pub fn xref_dump(doc: &PdfDoc) -> Vec<u8> {
    let xref = doc.get_xref().unwrap();
    let size = xref.get_num_objects();
    let mut s = String::new();
    s.push_str("xref\n");
    s.push_str(&format!("0 {}\n", size));
    let pad5 = |v: i32| -> String {
        if v < 0 {
            format!("-{:05}", (v as i64).unsigned_abs())
        } else {
            format!("{:05}", v)
        }
    };
    for i in 0..size {
        let e = xref.get_entry(i);
        if e.type_ != XRefEntryType::Compressed {
            s.push_str(&format!(
                "{:010} {} {}\n",
                e.offset as u32,
                pad5(e.gen),
                if e.type_ == XRefEntryType::Free { "f" } else { "n" }
            ));
        } else {
            let obj_str = xref.fetch(e.offset as i32, 0);
            let st = obj_str.get_stream().expect("object stream");
            let n_objects = st.get_dict().lookup("N").get_int();
            let first = st.get_dict().lookup("First").get_int();
            let first_offset = (st.get_start() as u32).wrapping_add(first as u32);
            let data: std::rc::Rc<[u8]> = std::rc::Rc::from(st.get_decoded_data().unwrap());
            let lexer = pdfread::Lexer::new(data, 0, Some(first as u64));
            let mut parser = pdfread::Parser::new(Some(xref), lexer, false);
            let mut local_offset = 0;
            for n in 0..n_objects {
                let _o1 = parser.get_obj();
                let o2 = parser.get_obj();
                if n == e.gen {
                    local_offset = o2.get_int();
                }
            }
            s.push_str(&format!("{:010} 00000 n\n", first_offset.wrapping_add(local_offset as u32)));
        }
    }
    s.into_bytes()
}

/// xpdf leaves `gen` uninitialized for xref entries that were never set
/// (offset -1), so the dump shows garbage there; blank it out.
pub fn normalize_dump(d: &[u8]) -> String {
    String::from_utf8_lossy(d)
        .lines()
        .map(|l| {
            if l.starts_with("4294967295 ") && l.ends_with(" f") {
                "4294967295 * f".to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("
")
}

/// All stream objects (num, gen) of a document, in xref order.
pub fn stream_objects(doc: &PdfDoc) -> Vec<(i32, i32)> {
    let xref = doc.get_xref().unwrap();
    let mut v = Vec::new();
    for num in 0..xref.get_num_objects() {
        let e = xref.get_entry(num);
        if e.type_ == XRefEntryType::Uncompressed {
            if let Object::Stream(_) = xref.fetch(num, e.gen) {
                v.push((num, e.gen));
            }
        }
    }
    v
}
