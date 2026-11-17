//! `GfxFont` encodings vs real xpdf: pdftex replaces embedded Type 1 /
//! Type 1C fonts of included PDFs (\pdfinclusioncopyfonts=0) and writes
//! the encoding obtained from `Gfx8BitFont::getCharName()`; ours must
//! give the same glyph names for all 256 codes.

mod common;

use common::epdf::check_font_encodings;
use common::*;
use pdfread::set_err_quiet;

#[test]
fn corpus_font_encodings_match_xpdf() {
    set_err_quiet(true);
    if tl_bin("pdftex").is_none() {
        eprintln!("pdftex not found; skipping");
        return;
    }
    let full = std::env::var("PDFREAD_ORACLE_FULL").is_ok();
    let files = corpus();
    let selected: Vec<_> =
        files.iter().enumerate().filter(|(i, _)| full || i % 6 == 0).map(|(_, p)| p.clone()).collect();
    let nthreads: usize = std::env::var("PDFREAD_ORACLE_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let chunks: Vec<Vec<_>> =
        (0..nthreads).map(|t| selected.iter().skip(t).step_by(nthreads).cloned().collect()).collect();
    let results: Vec<(usize, Vec<String>)> = std::thread::scope(|sc| {
        let hs: Vec<_> = chunks
            .iter()
            .map(|chunk| {
                sc.spawn(move || {
                    set_err_quiet(true);
                    let mut n = 0;
                    let mut v = Vec::new();
                    for p in chunk {
                        let bytes = match std::fs::read(p) {
                            Ok(b) => b,
                            Err(_) => continue,
                        };
                        match check_font_encodings(&bytes) {
                            Ok(k) => n += k,
                            Err(e) => {
                                let msg = format!("{}: {}", p.display(), e);
                                println!("{}", msg);
                                v.push(msg);
                            }
                        }
                    }
                    (n, v)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let fonts: usize = results.iter().map(|r| r.0).sum();
    let problems: Vec<&String> = results.iter().flat_map(|r| r.1.iter()).collect();
    println!("{} files, {} replaced fonts compared", selected.len(), fonts);
    assert!(problems.is_empty(), "{} files with encoding differences", problems.len());
}
