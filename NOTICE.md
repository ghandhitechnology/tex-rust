# Notices

tex-rust is a translation and port of free software. The original programs, their
authors and their licences are listed here. Each port keeps its source's licence.

## Ported programs

| Port | Original | Authors | Licence |
|---|---|---|---|
| `port/engine` (generated and runtime code) | pdfTeX 1.40.29 with e-TeX, encTeX, MLTeX and SyncTeX, built on TeX and web2c as in TeX Live 2026 | Donald E. Knuth (TeX); the NTS team and Peter Breitenlohner (e-TeX); Hàn Thế Thành and the pdfTeX team (pdfTeX); Petr Olšák (encTeX); Michael J. Ferguson (MLTeX); Jérôme Laurens (SyncTeX); Karl Berry, Tomas Rokicki, Tim Morgan and others (web2c) | GPL-2.0-or-later. TeX is © Donald E. Knuth: derived programs must not be called TeX, which is why this binary keeps pdfTeX's name |
| `port/crates/pdfread` | xpdf 4.06, the subset pdfTeX's `pdftoepdf.cc` uses | Glyph & Cog, LLC (Derek Noonburg) | GPL-2.0-only OR GPL-3.0-only ([COPYING](port/crates/pdfread/COPYING), [COPYING3](port/crates/pdfread/COPYING3)) |
| `port/crates/kpathsea` | kpathsea 6.4.2 | Karl Berry and others | LGPL-2.1-or-later ([LICENSE](port/crates/kpathsea/LICENSE)) |
| `port/crates/regex` | the glibc 2.5 POSIX regex engine bundled with pdfTeX | Free Software Foundation, Isamu Hasegawa | LGPL-2.1-or-later ([LICENSE](port/crates/regex/LICENSE)) |
| `port/crates/zlib` | zlib 1.3.2 | Jean-loup Gailly, Mark Adler | zlib licence ([LICENSE](port/crates/zlib/LICENSE)) |
| `port/engine/src/rt/pdf/images/libpng.rs` | the libpng 1.6.55 read path | the libpng authors (Cosmin Truta, Glenn Randers-Pehrson and others) | libpng licence ([LICENSE-libpng](port/engine/src/rt/pdf/images/LICENSE-libpng)) |

`port/web` contains TeX Live's `pdftex.web` with its change files merged (GPL-2.0-or-later,
TeX itself under Knuth's terms) and the Pascal tangled from it.

## Test data from other sources

| Files | Source | Licence |
|---|---|---|
| `port/tests/cases/trip`, `port/tests/cases/tripfiles` | Knuth's trip test and e-TeX's etrip test, from TeX Live | Knuth's terms; e-TeX's licence |
| `port/tests/cases/p631.tex`, `plain_copy.tex` | Knuth's `plain.tex` (the first is shortened and ends in `\dump`) | Knuth's terms (modified copies must not be called `plain.tex`) |
| `port/tests/cases/fonts/*.tfm` | renamed copies of Computer Modern (`cmr10`) and Latin Modern metrics | Knuth's terms (Computer Modern); GUST Font License (Latin Modern) |
| `port/work/images/cases/misc/mwe-*.pdf` | example images from the `mwe` package (Martin Scharrer) | LPPL 1.3 or later |
| `port/crates/regex/tests/data/upstream/glibc` | glibc's regex tests, including ones adapted from PCRE and Boost | LGPL-2.1-or-later; the PCRE and Boost licences noted in the files |
| `port/crates/regex/tests/data/upstream/att` | AT&T's testregex data, via the Go and Rust regex test suites | [AT&T licence](port/crates/regex/tests/data/upstream/att/LICENSE) |

The other test documents, generated images and benchmark documents were written for this
project.
