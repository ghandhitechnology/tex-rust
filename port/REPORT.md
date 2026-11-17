# pdfTeX in Rust: port report

**Result.** pdfTeX 1.40.29 (TeX Live 2026) has been ported to Rust with no C dependencies.
On every test below the Rust `pdftex.exe` writes byte-identical output to TeX Live's
`pdftex.exe` (stdout, stderr, exit code, `.log`, `.dvi`, `.pdf`, `.fmt`, `.aux`, `.fls`,
`.synctex`, `\openout` files). The only intended exception is `-version`, which names the
Rust ports instead of C libraries. The Rust engine is faster on all 17 benchmark jobs:
1.20× to 4.78× (median). Measured on an idle Windows 11 desktop with an Intel i5-14400F,
2026-11-18.

## What was built

| Part | How | Size (lines of Rust) |
|---|---|---:|
| Engine core: tex.web + e-TeX + pdfTeX + encTeX + MLTeX + SyncTeX hooks + web2c changes | tie + tangle of the TL 2026 sources (same change files as `pdftexdir/am/pdftex.am`), then `tools/w2rs` (a Pascal-to-Rust translator reproducing web2c's semantics) | 46,578 generated |
| web2c runtime (texmfmp.c, openclose.c, ...), pdfTeX's C library (PDF backend, fonts, images, SyncTeX, utils) | hand ports in `engine/src/rt` | 19,572 |
| zlib 1.3.2 | `crates/zlib` | 9,061 |
| kpathsea | `crates/kpathsea` | 7,004 |
| xpdf 4.06 subset (PDF inclusion) | `crates/pdfread` | 12,126 |
| glibc 2.5 POSIX regex (`\pdfmatch`) | `crates/regex` | 5,828 |
| libpng 1.6.55 read path, libjpeg-free JPEG and JBIG2 writers, MD5 | in `engine/src/rt` | (included above) |

**No C.** `cargo tree` for the engine lists only the four crates above, with no
third-party or `-sys` crates. The binary imports only Windows system DLLs (`KERNEL32`,
`ntdll`, `bcryptprimitives`) and the UCRT/VCRUNTIME that every Rust program built for the
x86_64-pc-windows-msvc target uses. From the UCRT it takes memcpy, process start-up, and
the libm functions behind `f64::powf`, `sin`, `atan`, `floor` and `round`. TeX Live's
pdftex.dll links the same UCRT statically, which is also why libpng's float gamma tables
come out bit-identical.

## How parity is checked

`tests/difftest.py` runs TeX Live's `pdftex.exe` and the Rust one on the same inputs in two
fresh directories, with `SOURCE_DATE_EPOCH` and `FORCE_SOURCE_DATE=1`. It compares every
output file byte for byte (formats after gunzip). It normalises only the run-directory
name where it legitimately appears, such as recorder `PWD` lines.

Format files contain a few words that TeX never initialises, such as
`font_check[null_font]`: C `malloc` garbage that differs between two TeX Live runs. For
formats the harness reruns TeX Live and ignores exactly the dumped items where TeX Live
disagrees with itself.

## Parity results (final binary)

| Suite | Cases | Identical | Notes |
|---|---:|---:|---|
| Knuth's trip test + e-TeX etrip (3 phases) | 4 | 4 | including mktex stderr |
| misc: plain.fmt, pdfetex.fmt, plain TeX to DVI/PDF, TeX Live's own installed formats, primitives, non-ASCII file names, interaction, output directory, ... | 11 | 11 | |
| core CLI: errors in all interaction modes, `-file-line-error`, `-halt-on-error`, `\write18` on/off/restricted, recorder, jobname, source specials, MLTeX, encTeX, TCX/8-bit, draft mode, `\pdfmatch`, `-help`, `-version` | 25 | 24 | `-version` differs on purpose |
| LaTeX: pdflatex.fmt build + 13 documents, each run twice (book, math, TikZ, hyperref, graphics, microtype, babel, tables, amsthm, colors, ...) | 14 | 14 | |
| fonts: map files, Type 1 subsetting, TrueType, PK/Type 3, encodings, ToUnicode, virtual fonts | 22 | 22 | |
| images: PNG (every colour type, bit depth, interlace, gamma, tRNS, damaged files), JPEG, JBIG2, PDF inclusion, image (un)dump in formats | 196 | 195 | see below |
| PDF inclusion of every PDF under 400 KB in texmf-dist (first and last page, `\pdfinclusioncopyfonts=0`, object streams) | 571 | 571 | |
| SyncTeX (`-synctex` = ±1…±15, gz and plain, PDF and DVI, non-ASCII paths) | 19 | 19 | `.synctex.gz` files embed the run directory, so they are compared after gunzip and normalisation; `*abs` cases are byte-identical even compressed |
| regex crate vs pdftex.exe (`\pdfmatch` oracle: patterns × subjects × flags, incl. 3,346 back-reference cases) | 29,286 | 29,286 | plus the glibc and AT&T suites; every case where those expect newer-glibc behaviour matches pdfTeX's glibc 2.5 instead |

The one image difference, `png_padding_knowndiff`, is not a porting difference. For PNGs
with under 8 bits per pixel whose rows end in a partial byte, libpng leaves the padding
bits from pdfTeX's uninitialised `malloc`'d row buffer. TeX Live's own output for that
file changes from run to run (two TeX Live runs today produced different bytes), while the
Rust output is the same every time.

Where the C program crashes, aborts or hangs, the port does the same. Examples: a regex
`assert()` gives the CRT's message and exit code 3; a NULL dereference gives exit code
0xC0000005; back-reference patterns that loop forever in glibc 2.5 also loop forever here.

## Speed

`tests/bench_port.py`, 20 runs per engine and job, pinned to the CPU's performance cores.
hyperfine (`-N`) measures each process, and the two engines alternate in five rounds so
that load drift affects both equally. Every job's output was checked identical before
timing (format builds: the `.log`; the `.fmt` itself is compared by the suites above).
Final build, 2026-11-18; the "first port" column is the same benchmark on the port as
first completed (2026-11-17), before the optimization work.

| Job | TeX Live median (ms) | Rust median (ms) | Speed-up | First port |
|---|---:|---:|---:|---:|
| plain format build (`-ini`) | 114.8 | 25.0 | **4.59x** | 3.90x |
| pdflatex format build (`-ini`) | 4425.7 | 3700.1 | **1.20x** | 1.11x |
| plain TeX, 70 pages, DVI | 107.3 | 25.0 | **4.29x** | 3.25x |
| plain TeX, 70 pages, PDF | 145.2 | 31.5 | **4.61x** | 2.26x |
| plain TeX, 700 pages, PDF | 497.1 | 184.4 | **2.70x** | 1.32x |
| LaTeX hello world | 136.9 | 28.6 | **4.78x** | 3.05x |
| LaTeX article (8 pages) | 194.8 | 60.3 | **3.23x** | 2.45x |
| LaTeX math (49 pages) | 262.4 | 111.4 | **2.36x** | 1.77x |
| LaTeX book (128 pages) | 528.4 | 345.2 | **1.53x** | 1.35x |
| LaTeX TikZ (12 pages) | 1901.4 | 1364.7 | **1.39x** | 1.17x |
| LaTeX heavy preamble | 809.8 | 413.9 | **1.96x** | 1.69x |
| LaTeX graphics (PDF, PNG, JPEG) | 173.9 | 44.3 | **3.93x** | 2.58x |
| LaTeX hyperref | 263.2 | 103.3 | **2.55x** | 2.07x |
| LaTeX microtype | 227.2 | 83.6 | **2.72x** | 2.08x |
| LaTeX hello world, format cache off | 133.9 | 49.1 | **2.73x** | 2.17x |
| LaTeX book, format cache off | 511.6 | 320.2 | **1.60x** | 1.29x |
| LaTeX book, both engines on TeX Live's installed pdflatex.fmt | 517.3 | 307.5 | **1.68x** | 1.38x |

Raw numbers and minima: `bench/results/port-final.md` and `port-final.json`.

Where the speed comes from:

- **Start-up.** TeX Live's pdftex.exe spends about 70-80 ms loading its DLLs; the Rust
  binary is a single executable with the C runtime linked statically and starts in about
  6 ms. kpathsea's ls-R index is cached on disk and memory-mapped (3 -> 1.2 ms). Formats
  are kept inflated in a cache and loaded with large reads straight into TeX's arrays;
  the 4.8 MB `hash_extra` part of eqtb is initialized lazily. pdftex.map is parsed once
  and cached (warnings are replayed, entries decoded on demand): 3 -> 0.65 ms.
- **PDF output.** Streams are compressed on worker threads with exactly the zlib calls
  pdfTeX makes (so the bytes are identical; deflate's output was checked to be
  independent of how the input is split), using provisional file offsets that are fixed
  up before the cross-reference table. Per-character work in the PDF writer
  (`divide_scaled` via cached reciprocals, memoized character widths, inline MLTeX/encTeX
  guards) and Type 1 font embedding were streamlined.
- **TeX's core.** Inline fast paths for reading tokens from token lists and files,
  copying macro arguments, skipping false conditional branches, freeing macro
  parameters, and control-sequence lookup (an index over the hash table). Each fast path
  either has exactly the generated routine's effects or falls back to it; the `verify`
  build (`cargo build --profile verify`) cross-checks several of them at run time.
- **Limits.** Expansion-heavy jobs (format builds, TikZ, book) are dominated by TeX's
  own macro expansion. The engine reproduces TeX's memory exactly (formats dump it, logs
  report its peak), so speeding this up further means hand-optimizing that machinery.
  Profile-guided optimisation gave nothing measurable.

## Open points

- During one earlier benchmark the Rust engine exited with code 1 on one TikZ run (TeX's
  "error occurred" status). It was not reproduced in about 260 further runs of the same
  job, under load and idle. Its log was overwritten before it could be inspected, so the
  cause is unknown.
- The port targets the parity reference, TeX Live's Windows x86_64 pdftex. The engine uses
  Win32 calls directly (console Ctrl-C handler, local-time conversion, DOS device names),
  so other platforms would need those few runtime pieces ported.
- `-version` names the Rust ports (`Compiled with Rust ports of libpng 1.6.55, zlib
  1.3.2, ...`) instead of claiming C libraries the binary does not contain.

## Reproducing

TeX Live 2026 is expected in `tools/texlive/2026` at the top of the checkout, or wherever
`TEXRS_TEXLIVE` points (see `bench/setup/texlive.profile.template` for the installed
collections).

```
cd port/engine && cargo build --release
cd .. && set TEXRS_PROFILE=release
python -I tests/regress.py                                   # trip, misc, core, LaTeX
python -I tests/difftest.py --suite tests/cases/fonts_suite.json
set TEXRS_ENGINE_DIR=%CD%\engine
python -I work/images/tools/cases.py                         # images
python -I work/images/tools/corpus_all.py                    # 571 texmf-dist PDFs
python -I -X utf8 work/synctex/tools/suite.py                # SyncTeX
cd crates/regex && cargo test --release                      # regex vs pdftex oracle
cd ../.. && python -I tests/bench_port.py 20                 # speed (idle machine)
```

`tools/sampler.py` is a sampling profiler for the engine that needs no admin rights.
`tests/ab_time.py` gives quick interleaved A/B timings.
