//! Micro-benchmark of the ls-R read + index build (`init_db`) and of
//! texmf.cnf reading, isolated from process start-up.
//!
//! cargo run --release --example lsr_parse -- <TL bin dir> [reps]

use std::time::Instant;

use texrs_kpathsea::Kpse;

fn main() {
    let dir = std::env::args().nth(1).expect("usage: lsr_parse <dir of kpsewhich.exe> [reps]");
    let reps: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(50);
    let exe = format!("{}\\pdftex.exe", dir);
    let lsr = format!("{}\\..\\..\\texmf-dist\\ls-R", dir);
    let mut best_read = f64::MAX;
    let mut best_init = f64::MAX;
    for _ in 0..reps {
        let t0 = Instant::now();
        let d = std::fs::read(&lsr).unwrap();
        let t1 = Instant::now();
        std::hint::black_box(&d);
        best_read = best_read.min((t1 - t0).as_secs_f64() * 1e3);
        let mut k = Kpse::with_exe_path(&exe, Some("pdflatex"));
        let t2 = Instant::now();
        // The first configuration lookup reads texmf.cnf and every ls-R.
        k.var_value("TEXMF");
        let t3 = Instant::now();
        best_init = best_init.min((t3 - t2).as_secs_f64() * 1e3);
    }
    println!("read texmf-dist/ls-R ({} bytes):   {:.3} ms", std::fs::metadata(&lsr).unwrap().len(), best_read);
    println!("texmf.cnf + all ls-R (init_db):     {:.3} ms", best_init);
}
