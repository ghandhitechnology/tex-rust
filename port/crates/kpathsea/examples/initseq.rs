//! Replays the first kpathsea calls pdfTeX makes at start-up (texmfmp.c on
//! Windows): `kpse_set_program_name (argv[0], NULL)` then
//! `kpse_var_value ("command_line_encoding")`, followed by `kpse_var_value`
//! of every variable named on the command line.  Used to compare the
//! pdfTeX-only debug traces (hash_lookup, fopen) with the real pdftex.exe:
//!
//!   KPATHSEA_DEBUG=2 TEXRS_SELFAUTOLOC=<TL bin> initseq pdftex VAR...

fn main() {
    let mut args = std::env::args().skip(1);
    let argv0 = args.next().unwrap_or_else(|| "pdftex".to_string());
    let mut k = texrs_kpathsea::Kpse::new(&argv0, None);
    k.init_command_line_encoding();
    for v in args {
        k.var_value(v.as_bytes());
    }
}
