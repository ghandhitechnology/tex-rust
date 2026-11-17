//! Differential tests against TeX Live's `pdftosrc.exe`, which is built
//! on the real xpdf 4.06: the xref table it dumps (`pdftosrc f.pdf -1`)
//! and the decoded bytes of stream objects (`pdftosrc f.pdf N`) must be
//! identical to ours.  Skipped when the binary is not present.
//!
//! By default a deterministic sample of the corpus is checked; set
//! `PDFREAD_ORACLE_FULL=1` to check every file and every stream.

mod common;

use common::*;
use pdfread::{set_err_quiet, PdfDoc};

fn check_file(tool: &std::path::Path, path: &std::path::Path, max_streams: usize) -> Vec<String> {
    let mut problems = Vec::new();
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return problems,
    };
    let doc = PdfDoc::from_bytes(bytes.clone());
    let tmp = TempDir::new("oracle");
    let theirs = pdftosrc(tool, tmp.path(), &bytes, -1, 0);
    if !doc.is_ok() {
        if theirs.is_some() {
            problems.push(format!("{}: xpdf opens it, we don't", path.display()));
        }
        return problems;
    }
    let theirs = match theirs {
        Some(t) => t,
        None => {
            // pdftosrc also requires a /Type /Catalog root
            let cat = doc.get_xref().unwrap().get_catalog();
            if cat.is_dict_of("Catalog") {
                problems.push(format!("{}: we open it, xpdf doesn't", path.display()));
            }
            return problems;
        }
    };
    let ours = xref_dump(&doc);
    if normalize_dump(&ours) != normalize_dump(&theirs) {
        problems.push(format!(
            "{}: xref dump differs\n--- ours\n{}\n--- xpdf\n{}",
            path.display(),
            String::from_utf8_lossy(&ours[..ours.len().min(600)]),
            String::from_utf8_lossy(&theirs[..theirs.len().min(600)])
        ));
    }
    let streams = stream_objects(&doc);
    let step = (streams.len() / max_streams.max(1)).max(1);
    for (k, &(num, gen)) in streams.iter().enumerate() {
        if k % step != 0 {
            continue;
        }
        let ours = doc.get_xref().unwrap().fetch(num, gen);
        let s = ours.get_stream().unwrap();
        let ours = match s.get_decoded_data() {
            Ok(d) => d,
            Err(_) => continue, // image codecs are not implemented
        };
        let theirs = pdftosrc(tool, tmp.path(), &bytes, num, gen);
        match theirs {
            Some(t) if t == ours => {}
            Some(t) => problems.push(format!(
                "{}: stream {} {} differs (ours {} bytes, xpdf {} bytes)",
                path.display(),
                num,
                gen,
                ours.len(),
                t.len()
            )),
            None => problems.push(format!("{}: pdftosrc failed on stream {} {}", path.display(), num, gen)),
        }
    }
    problems
}

#[test]
fn corpus_matches_pdftosrc() {
    set_err_quiet(true);
    let tool = match tl_bin("pdftosrc") {
        Some(t) => t,
        None => {
            eprintln!("pdftosrc not found; skipping");
            return;
        }
    };
    let full = std::env::var("PDFREAD_ORACLE_FULL").is_ok();
    let files = corpus();
    let selected: Vec<_> = if full {
        files.clone()
    } else {
        // every 8th file plus all benchmark outputs (object streams)
        files
            .iter()
            .enumerate()
            .filter(|(i, p)| i % 8 == 0 || p.to_string_lossy().contains("bench"))
            .map(|(_, p)| p.clone())
            .collect()
    };
    let max_streams = if full { usize::MAX } else { 4 };
    let nthreads: usize = std::env::var("PDFREAD_ORACLE_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let chunks: Vec<Vec<_>> = (0..nthreads)
        .map(|t| selected.iter().skip(t).step_by(nthreads).cloned().collect())
        .collect();
    let problems: Vec<String> = std::thread::scope(|sc| {
        let hs: Vec<_> = chunks
            .iter()
            .map(|chunk| {
                let tool = tool.clone();
                sc.spawn(move || {
                    set_err_quiet(true);
                    let mut v = Vec::new();
                    for p in chunk {
                        let probs = check_file(&tool, p, max_streams);
                        for pr in &probs {
                            println!("{}", pr);
                        }
                        v.extend(probs);
                    }
                    v
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    println!("checked {} files against pdftosrc", selected.len());
    assert!(problems.is_empty(), "{} differences against xpdf", problems.len());
}
