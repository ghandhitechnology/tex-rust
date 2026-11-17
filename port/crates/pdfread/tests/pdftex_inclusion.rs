//! Real-world differential test: include page 1 of corpus PDFs with TeX
//! Live's pdftex (real xpdf + pdftoepdf.cc) and compare every object it
//! re-serialized with what we parsed.  A deterministic sample is used by
//! default; `PDFREAD_ORACLE_FULL=1` checks every corpus file.

mod common;

use common::epdf::check_with_pdftex;
use common::*;
use pdfread::set_err_quiet;

#[test]
fn corpus_inclusion_matches_pdftex() {
    set_err_quiet(true);
    if tl_bin("pdftex").is_none() {
        eprintln!("pdftex not found; skipping");
        return;
    }
    let full = std::env::var("PDFREAD_ORACLE_FULL").is_ok();
    let files = corpus();
    let selected: Vec<_> = files
        .iter()
        .enumerate()
        .filter(|(i, _)| full || i % 10 == 0)
        .map(|(_, p)| p.clone())
        .collect();
    let nthreads: usize = std::env::var("PDFREAD_ORACLE_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let chunks: Vec<Vec<_>> =
        (0..nthreads).map(|t| selected.iter().skip(t).step_by(nthreads).cloned().collect()).collect();
    let problems: Vec<String> = std::thread::scope(|sc| {
        let hs: Vec<_> = chunks
            .iter()
            .map(|chunk| {
                sc.spawn(move || {
                    set_err_quiet(true);
                    let mut v = Vec::new();
                    for p in chunk {
                        let bytes = match std::fs::read(p) {
                            Ok(b) => b,
                            Err(_) => continue,
                        };
                        if let Err(e) = check_with_pdftex(&bytes, 1) {
                            let msg = format!("{}: {}", p.display(), &e[..e.len().min(2000)]);
                            println!("{}", msg);
                            v.push(msg);
                        }
                    }
                    v
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let n = common::epdf::COMPARED.load(std::sync::atomic::Ordering::Relaxed);
    println!("included {} files with pdftex, {} objects compared", selected.len(), n);
    assert!(n > selected.len());
    assert!(problems.is_empty(), "{} files differ from pdftex", problems.len());
}
