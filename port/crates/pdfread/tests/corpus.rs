//! Opens every PDF found under the portable TeX Live tree and the
//! benchmark outputs, walks all pages (boxes, resources, contents) and
//! fetches every object in the xref, decoding every stream.  The test
//! fails if anything panics.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use pdfread::{set_err_quiet, Object, PdfDoc, XRefEntryType};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn collect_pdfs(dir: &Path, out: &mut Vec<PathBuf>) {
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

#[derive(Default, Debug)]
struct Stats {
    files: usize,
    opened: usize,
    failed_open: Vec<(PathBuf, i32)>,
    pages: usize,
    objects: usize,
    streams: usize,
    unsupported_streams: usize,
    encrypted: usize,
    repaired: usize,
    panics: Vec<PathBuf>,
}

fn touch_stream(s: &pdfread::Stream, st: &mut Stats) {
    let _ = s.get_undecoded_data();
    match s.get_decoded_data() {
        Ok(_) => {}
        Err(_) => st.unsupported_streams += 1,
    }
    st.streams += 1;
}

fn walk(path: &Path, st: &mut Stats) {
    let doc = PdfDoc::open(path);
    if !doc.is_ok() {
        st.failed_open.push((path.to_path_buf(), doc.get_error_code()));
        return;
    }
    st.opened += 1;
    let xref = doc.get_xref().unwrap().clone();
    if xref.is_encrypted() {
        st.encrypted += 1;
    }
    if xref.is_repaired() {
        st.repaired += 1;
    }
    let _ = doc.get_pdf_version();
    let _ = doc.ok_to_print(false);
    let _ = doc.get_doc_info_nf();
    let cat = doc.get_catalog().unwrap();
    for i in 1..=cat.get_num_pages() {
        let page = cat.get_page(i).expect("page");
        let r = cat.get_page_ref(i).unwrap();
        let _ = xref.fetch(r.num, r.gen);
        let _ = (page.get_media_box(), page.get_crop_box(), page.get_bleed_box());
        let _ = (page.get_trim_box(), page.get_art_box(), page.get_rotate(), page.get_group());
        if let Some(res) = page.get_resource_dict() {
            for k in 0..res.get_length() {
                let v = res.get_val(k);
                if let Object::Dict(d) = &v {
                    for j in 0..d.get_length() {
                        let _ = d.get_val(j);
                    }
                }
            }
        }
        match page.get_contents() {
            Object::Stream(s) => touch_stream(&s, st),
            Object::Array(a) => {
                for j in 0..a.get_length() {
                    if let Object::Stream(s) = a.get(j) {
                        touch_stream(&s, st);
                    }
                }
            }
            _ => {}
        }
        st.pages += 1;
    }
    let n = xref.get_num_objects();
    for num in 0..n {
        let e = xref.get_entry(num);
        let gen = match e.type_ {
            XRefEntryType::Uncompressed => e.gen,
            XRefEntryType::Compressed => 0,
            XRefEntryType::Free => continue,
        };
        let o = xref.fetch(num, gen);
        if let Object::Stream(s) = &o {
            touch_stream(s, st);
        }
        st.objects += 1;
    }
}

#[test]
fn open_walk_and_fetch_every_corpus_pdf() {
    set_err_quiet(true);
    let root = repo_root();
    let mut files = Vec::new();
    collect_pdfs(&root.join("tools/texlive/2026/texmf-dist"), &mut files);
    collect_pdfs(&root.join("bench/out"), &mut files);
    files.sort();
    if files.is_empty() {
        eprintln!("no corpus PDFs found under {}; skipping", root.display());
        return;
    }
    let mut st = Stats::default();
    for f in &files {
        st.files += 1;
        let r = catch_unwind(AssertUnwindSafe(|| {
            let mut local = Stats::default();
            walk(f, &mut local);
            local
        }));
        match r {
            Ok(l) => {
                st.opened += l.opened;
                st.failed_open.extend(l.failed_open);
                st.pages += l.pages;
                st.objects += l.objects;
                st.streams += l.streams;
                st.unsupported_streams += l.unsupported_streams;
                st.encrypted += l.encrypted;
                st.repaired += l.repaired;
            }
            Err(_) => st.panics.push(f.clone()),
        }
    }
    println!(
        "files {} opened {} pages {} objects {} streams {} (unsupported-filter streams {}) encrypted {} repaired {}",
        st.files, st.opened, st.pages, st.objects, st.streams, st.unsupported_streams, st.encrypted, st.repaired
    );
    for (p, code) in &st.failed_open {
        println!("failed to open (err {}): {}", code, p.display());
    }
    for p in &st.panics {
        println!("PANIC: {}", p.display());
    }
    assert!(st.panics.is_empty(), "{} files panicked", st.panics.len());
    assert!(st.opened > 0);
}
