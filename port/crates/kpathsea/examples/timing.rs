//! In-process timing of initialization and lookups (no process start-up).
//!
//! cargo run --release --example timing -- <TL bin dir> [reps]

use std::time::Instant;

use texrs_kpathsea::{FileFormat, Kpse};

/// Files read from the TeX tree by `pdflatex -recorder` on
/// bench/corpus/02-article.tex, with the format pdfTeX uses for each.
const RUN: &[(&str, FileFormat)] = &[
    ("pdflatex.fmt", FileFormat::Fmt),
    ("article.cls", FileFormat::Tex),
    ("size11.clo", FileFormat::Tex),
    ("l3backend-pdftex.def", FileFormat::Tex),
    ("amsmath.sty", FileFormat::Tex),
    ("amstext.sty", FileFormat::Tex),
    ("amsgen.sty", FileFormat::Tex),
    ("amsbsy.sty", FileFormat::Tex),
    ("amsopn.sty", FileFormat::Tex),
    ("amssymb.sty", FileFormat::Tex),
    ("amsfonts.sty", FileFormat::Tex),
    ("umsa.fd", FileFormat::Tex),
    ("umsb.fd", FileFormat::Tex),
    ("cmr10", FileFormat::Tfm),
    ("cmr12", FileFormat::Tfm),
    ("cmr17", FileFormat::Tfm),
    ("cmr6", FileFormat::Tfm),
    ("cmr8", FileFormat::Tfm),
    ("cmmi10", FileFormat::Tfm),
    ("cmmi6", FileFormat::Tfm),
    ("cmmi8", FileFormat::Tfm),
    ("cmsy10", FileFormat::Tfm),
    ("cmsy6", FileFormat::Tfm),
    ("cmsy8", FileFormat::Tfm),
    ("cmex10", FileFormat::Tfm),
    ("cmex7", FileFormat::Tfm),
    ("cmex8", FileFormat::Tfm),
    ("cmbx10", FileFormat::Tfm),
    ("cmbx12", FileFormat::Tfm),
    ("cmbx6", FileFormat::Tfm),
    ("cmbx8", FileFormat::Tfm),
    ("msam10", FileFormat::Tfm),
    ("msam7", FileFormat::Tfm),
    ("msbm10", FileFormat::Tfm),
    ("msbm7", FileFormat::Tfm),
    ("pdftex.map", FileFormat::Fontmap),
    ("cmbx10.pfb", FileFormat::Type1),
    ("cmbx12.pfb", FileFormat::Type1),
    ("cmex10.pfb", FileFormat::Type1),
    ("cmmi10.pfb", FileFormat::Type1),
    ("cmmi8.pfb", FileFormat::Type1),
    ("cmr10.pfb", FileFormat::Type1),
    ("cmr12.pfb", FileFormat::Type1),
    ("cmr17.pfb", FileFormat::Type1),
    ("cmr6.pfb", FileFormat::Type1),
    ("cmr8.pfb", FileFormat::Type1),
    ("cmsy10.pfb", FileFormat::Type1),
    ("cmsy8.pfb", FileFormat::Type1),
];

fn main() {
    let dir = std::env::args().nth(1).expect("usage: timing <dir of kpsewhich.exe> [reps]");
    let reps: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(60);
    let exe = format!("{}\\pdftex.exe", dir);
    let mut best = [f64::MAX; 5];
    for _ in 0..reps {
        let t0 = Instant::now();
        let mut k = Kpse::with_exe_path(&exe, None);
        let t1 = Instant::now();
        k.init_command_line_encoding(); // reads texmf.cnf + all ls-R
        k.reset_program_name("pdflatex");
        k.xputenv("engine", "pdftex");
        let t2 = Instant::now();
        let a = k.find_file(RUN[0].0, RUN[0].1, true);
        let b = k.find_file(RUN[13].0, RUN[13].1, true); // first tfm: reads texfonts.map
        let t3 = Instant::now();
        let mut n = 0;
        for (name, f) in RUN {
            if k.find_file(name, *f, true).is_some() {
                n += 1;
            }
        }
        let t4 = Instant::now();
        assert!(a.is_some() && b.is_some() && n == RUN.len());
        let ts = [t0, t1, t2, t3, t4];
        for i in 0..4 {
            best[i] = best[i].min((ts[i + 1] - ts[i]).as_secs_f64() * 1e3);
        }
        best[4] = best[4].min((t4 - t0).as_secs_f64() * 1e3);
    }
    let labels = [
        "set_program_name",
        "texmf.cnf + 3 ls-R",
        "fmt + first tfm (texfonts.map)",
        "48 lookups of a pdflatex run",
        "total (min)",
    ];
    for (l, b) in labels.iter().zip(best.iter()) {
        println!("{:32} {:8.3} ms", l, b);
    }
}
