# tex-rust

pdfTeX 1.40.29, as released in TeX Live 2026, ported to Rust with no C dependencies. On
every test the Rust `pdftex.exe` writes byte-identical output to TeX Live's `pdftex.exe`:
stdout, stderr, exit code, `.log`, `.dvi`, `.pdf`, `.fmt`, `.aux`, `.fls`, `.synctex`
and `\openout` files. It is also faster: 1.20× to 4.78× on the 17 benchmark jobs (median
wall-clock time).

This is a port of pdfTeX, the engine TeX Live's `pdflatex` runs, not a new TeX. It
reads the same TeX tree, `texmf.cnf` and format files as TeX Live, and passes Knuth's
trip test and e-TeX's etrip test.

The full write-up is [port/REPORT.md](port/REPORT.md), and the code layout is in
[port/ARCHITECTURE.md](port/ARCHITECTURE.md).

## Speed

These are medians over 20 runs per engine and job, on an idle Windows 11 desktop with an
Intel i5-14400F (performance cores). Every job's output was checked identical before timing.

| Job | TeX Live (ms) | Rust (ms) | Speed-up |
|---|---:|---:|---:|
| plain TeX, 70 pages, PDF | 145.2 | 31.5 | 4.61× |
| plain TeX, 700 pages, PDF | 497.1 | 184.4 | 2.70× |
| LaTeX hello world | 136.9 | 28.6 | 4.78× |
| LaTeX article (8 pages) | 194.8 | 60.3 | 3.23× |
| LaTeX math (49 pages) | 262.4 | 111.4 | 2.36× |
| LaTeX book (128 pages) | 528.4 | 345.2 | 1.53× |
| LaTeX TikZ (12 pages) | 1901.4 | 1364.7 | 1.39× |
| LaTeX heavy preamble | 809.8 | 413.9 | 1.96× |
| LaTeX graphics (PDF, PNG, JPEG) | 173.9 | 44.3 | 3.93× |
| LaTeX hyperref | 263.2 | 103.3 | 2.55× |
| pdflatex format build | 4425.7 | 3700.1 | 1.20× |

All 17 jobs and the minima are in [bench/results/port-final.md](bench/results/port-final.md).
Short jobs gain the most because the Rust binary starts in about 6 ms; TeX Live's spends
70-80 ms loading DLLs. Long, expansion-heavy jobs (TikZ, books, format builds) are
dominated by TeX's own macro expansion. The engine reproduces that exactly, down to
memory usage, so they gain less.

## What is in it

| Part | Where | Lines of Rust |
|---|---|---:|
| TeX + e-TeX + pdfTeX + encTeX + MLTeX + SyncTeX hooks + web2c changes, translated from the tangled Pascal by `port/tools/w2rs` | `port/engine/src/gen` (generated) | 46,578 |
| web2c runtime and pdfTeX's C library (PDF backend, fonts, images, SyncTeX, utilities), plus the libpng 1.6.55 read path and the JPEG and JBIG2 writers | `port/engine/src/rt` | 19,572 |
| zlib 1.3.2 (byte-identical deflate/inflate) | `port/crates/zlib` | 9,061 |
| kpathsea 6.4.2 | `port/crates/kpathsea` | 7,004 |
| the xpdf 4.06 subset used for PDF inclusion | `port/crates/pdfread` | 12,126 |
| glibc 2.5 POSIX regex (`\pdfmatch`) | `port/crates/regex` | 5,828 |

`cargo tree` lists only these four crates: there are no third-party or `-sys` crates. The
binary imports only Windows system DLLs and the C runtime that every Rust program built
for `x86_64-pc-windows-msvc` uses, which is linked statically.

## Requirements

- Windows x86_64. The engine targets the parity reference, TeX Live's Windows pdftex, and
  calls Win32 directly in a few places: the console Ctrl-C handler, local-time conversion
  and DOS device names. Other platforms would need those runtime pieces ported.
- Rust stable with the MSVC toolchain.
- TeX Live 2026 for the TeX tree (fonts, macro packages, `texmf.cnf`). The parity tests
  also run TeX Live's `pdftex.exe`. [bench/setup/texlive.profile.template](bench/setup/texlive.profile.template)
  is the portable installation the tests were run against: the default place is
  `tools/texlive/2026` at the top of this checkout, and `TEXRS_TEXLIVE` selects another.
- Python 3 for the test harness, and [hyperfine](https://github.com/sharkdp/hyperfine)
  for the benchmarks.

## Building and running

```
cd port\engine
cargo build --release
```

The binary is `port\engine\target\release\pdftex.exe`. Like TeX Live's, it finds
`texmf.cnf` relative to its own directory (kpathsea's `SELFAUTOLOC`). To run it from the
build directory, point `TEXRS_SELFAUTOLOC` at TeX Live's `bin\windows`:

```
set TEXRS_SELFAUTOLOC=C:/texlive/2026/bin/windows
pdftex.exe -progname=pdflatex -fmt=pdflatex paper.tex
```

It reads TeX Live's installed formats. The formats it writes match TeX Live's after
decompression, except for the few words TeX never initializes, which differ between two
TeX Live runs too (see [port/REPORT.md](port/REPORT.md)).

Three caches speed up start-up. Each is validated against its source file and lives
under `%LOCALAPPDATA%\texrs`:

| Cache | Contents | Turn off | Move |
|---|---|---|---|
| `fmt-cache` | inflated formats | `TEXRS_FMT_CACHE=0` | `TEXRS_FMT_CACHE_DIR` |
| `kpse-cache` | the `ls-R` index | `TEXRS_KPSE_CACHE=0` | `TEXRS_KPSE_CACHE_DIR` |
| `map-cache` | parsed `pdftex.map` | `TEXRS_MAP_CACHE=0` | `TEXRS_MAP_CACHE_DIR` |

`TEXRS_PAR_ZIP=0` turns off compressing PDF streams on worker threads. The output is
identical either way.

## Tests

The parity tests run TeX Live's `pdftex.exe` and the Rust one on the same inputs in fresh
directories and compare every output file byte for byte (`port/tests/difftest.py`).

```
cd port
set TEXRS_PROFILE=release
python -I tests/regress.py                                   # trip, etrip, misc, core CLI, LaTeX
python -I tests/difftest.py --suite tests/cases/fonts_suite.json
python -I work/images/tools/cases.py                         # PNG, JPEG, JBIG2, PDF inclusion
python -I work/images/tools/corpus_all.py                    # 571 PDFs from texmf-dist
python -I -X utf8 work/synctex/tools/suite.py                # SyncTeX
cd crates\regex && cargo test --release                      # regex against pdftex's \pdfmatch
```

Results for the final build:

| Suite | Identical |
|---|---|
| trip, etrip, misc, core CLI, LaTeX | 53 of 54 (`-version` differs on purpose) |
| fonts | 22 of 22 |
| images | 195 of 196 (see below) |
| PDF inclusion corpus | 571 of 571 |
| SyncTeX | 19 of 19 |
| `\pdfmatch` oracle | 29,286 of 29,286 |

The one image difference comes from TeX Live itself. libpng leaves padding bits from an
uninitialized buffer, so TeX Live's output for that file changes from run to run.

`python -I tests/bench_port.py 20` runs the benchmark. `tools/sampler.py` and
`TEXRS_SAMPLE` are sampling profilers that need no admin rights.

## Regenerating the translated code

`port/web` holds TeX Live's `pdftex.web`, its change files merged by tie, and the Pascal
that tangle produces from them. `port/tools/gen-pascal.sh` regenerates it from a
[texlive-source](https://github.com/TeX-Live/texlive-source) checkout at `branch2026`.
Then, from `port/`:

```
python -I tools/w2rs/emit.py web/pdftex.p engine/src/gen
python -I tools/gen_patches.py
python -I tools/gen_fast_macro.py
```

Never edit `engine/src/gen` by hand. Change the translator (`tools/w2rs`) or the runtime
(`engine/src/rt`) instead.

## Licence

The engine is derived from pdfTeX, TeX and web2c and is licensed under the GNU GPL,
version 2 or (at your option) any later version. `port/crates/pdfread` is derived from
xpdf and may be used under GPL version 2 or version 3, but not later versions. The
`pdftex.exe` binary contains both, so it can be distributed under GPL version 2 or
version 3. Texts: [LICENSE](LICENSE) (GPL-2.0) and [LICENSE-GPL-3.0](LICENSE-GPL-3.0).

The other crates keep their original licences:

- `kpathsea` and `regex`: LGPL-2.1-or-later
- `zlib`: the zlib licence
- the libpng port in `engine/src/rt/pdf/images`: the libpng licence

[NOTICE.md](NOTICE.md) lists the authors, licences and third-party test data.
