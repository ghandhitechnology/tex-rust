//! End-to-end check against real TeX Live 2026 output: the installed .fmt
//! files were written by TL's pdftex.dll (C zlib 1.3.2) with
//! gzdopen(fd, "wb"), gzsetparams(f, 1, Z_DEFAULT_STRATEGY), gzwrite()s and
//! gzclose(). Decompress each one with GzReader, recompress it with
//! GzWriter the same way, and require the identical file.
//!
//! The TeX Live tree is found via $TEXRS_TL_ROOT or ../../../tools/texlive/2026
//! relative to this crate; the test is skipped (with a message) if absent.

use std::fs;
use std::io::Read;
use std::path::PathBuf;

use texrs_zlib::{GzReader, GzWriter};

fn tl_root() -> Option<PathBuf> {
    let p = match std::env::var("TEXRS_TL_ROOT") {
        Ok(v) => PathBuf::from(v),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tools/texlive/2026"),
    };
    p.join("texmf-var/web2c").is_dir().then_some(p)
}

fn fmt_files() -> Vec<PathBuf> {
    let Some(root) = tl_root() else {
        return Vec::new();
    };
    let mut v = Vec::new();
    for engine in ["pdftex", "tex", "euptex"] {
        if let Ok(rd) = fs::read_dir(root.join("texmf-var/web2c").join(engine)) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "fmt") {
                    v.push(p);
                }
            }
        }
    }
    v.sort();
    v
}

#[test]
fn texlive_fmt_files_recompress_identically() {
    let files = fmt_files();
    if files.is_empty() {
        eprintln!("SKIP: no TeX Live 2026 format files found (set TEXRS_TL_ROOT)");
        return;
    }
    let mut checked = 0;
    for path in files {
        let orig = fs::read(&path).unwrap();
        if orig.len() < 2 || orig[0] != 0x1f || orig[1] != 0x8b {
            continue; // not gzip-compressed (e.g. LuaTeX formats differ)
        }
        // decompress with gzread() in format-loading sized reads
        let mut r = GzReader::new(&orig[..]);
        let mut raw = Vec::new();
        let mut buf = vec![0u8; 65536];
        loop {
            let n = r.gzread(&mut buf);
            assert!(n >= 0, "{}: gzread error {:?}", path.display(), r.gzerror());
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n as usize]);
        }
        assert_eq!(r.gzclose(), 0);
        // and through std::io::Read
        let mut raw2 = Vec::new();
        GzReader::new(&orig[..]).read_to_end(&mut raw2).unwrap();
        assert!(raw == raw2);

        // recompress as pdfTeX's w_open_out + do_dump: gzdopen "wb",
        // gzsetparams(1, Z_DEFAULT_STRATEGY), dumps of mixed sizes
        let mut w = GzWriter::with_mode(Vec::new(), "wb", "<fd:3>").unwrap();
        assert_eq!(w.gzsetparams(1, 0), 0);
        let mut pos = 0;
        let mut k = 0usize;
        while pos < raw.len() {
            let n = [4usize, 4, 4, 8, 4, 40_000, 4, 1, 300_000][k % 9].min(raw.len() - pos);
            assert_eq!(w.gzwrite(&raw[pos..pos + n]), n as i32);
            pos += n;
            k += 1;
        }
        let (ret, out) = w.finish();
        assert_eq!(ret, 0);
        assert!(
            out == orig,
            "{}: recompressed {} bytes vs original {} bytes (first diff at {:?})",
            path.display(),
            out.len(),
            orig.len(),
            out.iter().zip(orig.iter()).position(|(a, b)| a != b)
        );
        eprintln!(
            "{}: {} -> {} bytes, byte-identical",
            path.file_name().unwrap().to_string_lossy(),
            raw.len(),
            orig.len()
        );
        checked += 1;
    }
    assert!(checked > 0);
    eprintln!("[texlive_fmt] {} TeX Live format files reproduced byte-for-byte", checked);
}
