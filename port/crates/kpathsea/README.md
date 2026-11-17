# texrs-kpathsea

A pure-Rust port of **kpathsea 6.4.2**, the path-searching library of TeX
Live 2026, as built for Windows (`kpathsealibw64.dll`). It exists so the Rust
port of pdfTeX finds exactly the same files, reads exactly the same
`texmf.cnf` values and accepts or refuses exactly the same output names as TeX
Live's C pdftex.

- No dependencies. The only foreign calls are a few `kernel32` imports
  (`GetFileAttributes[AW]`, `FindFirstFile[AW]`, `MultiByteToWideChar`, and
  so on). They are the Win32 calls the C library makes, called with the same
  bytes, which is how the behaviour stays identical. `user32!CharLowerA` and
  `advapi32!GetUserNameW` are loaded on demand: only for non-ASCII ls-R bytes
  and for `~user`.
- Strings are byte strings (`&[u8]`/`Vec<u8>`), like the C `char *`.
  Arguments accept `&str`, `String`, `&[u8]` or `Vec<u8>` (`AsRef<[u8]>`).
- Every Rust function's doc comment names the C function it mirrors.
- Results: **1257 differential queries, 0 mismatches**, and **5–6× faster**
  than `kpsewhich.exe` end to end. Details below.

## Building and testing

```
cargo build --release          # library + target/release/rkpsewhich.exe
cargo test --release           # 12 unit + 9 integration tests + 1 doctest
python -I tests/difftest.py    # full differential suite (about 3 minutes)
python -I tests/bench.py 30    # process-level timing vs kpsewhich.exe
cargo run --release --example timing -- <TL>\bin\windows   # in-process timing
cargo run --release --example lsr_parse -- <TL>\bin\windows  # texmf.cnf + ls-R load only
cargo run --release --example initseq -- pdftex             # pdftex start-up call replay
```

The integration tests and the scripts use the TeX Live installation in
`<repo>/tools/texlive/2026`. The integration tests skip themselves if it is
missing.

## C file → Rust module

| C source (texk/kpathsea) | Rust | Notes |
|---|---|---|
| progname.c (WIN32 `kpathsea_set_program_name`, `fix_selfdir`, `mk_suffixlist`), win32lib.c (`kpathsea_getlongpath`, `init_user_info`, `get_home_directory`, `getpwnam`), tex-file.c `kpathsea_reset_program_name`, knj.c `get_command_line_args_utf8` | `progname.rs` | SearchPath emulation; `TEXRS_SELFAUTOLOC` override |
| cnf.c | `cnf.rs` | `do_line`, `read_all_cnf`, `kpathsea_cnf_get`, `-cnf-line` |
| variable.c | `variable.rs` | `var_value`, `var_expand`, recursion guard |
| expand.c, kdefault.c, tilde.c, path-elt.c | `expand.rs` | brace, `~`, `!!`, `KPSE_DOT`, path elements |
| elt-dirs.c, dir.c, str-llist.c | `eltdirs.rs` | `//` expansion (`FindFirstFileW` order), element cache, `dir_links`, list floating |
| db.c | `db.rs` | ls-R and aliases, `match`, `elt_in_db`, `db_insert` |
| hash.c | `hash.rs` | bucket-exact chained table (behaviour depends on it, see below) |
| pathsearch.c, readable.c, str-list.c (`uniqify`), tex-hush.c | `pathsearch.rs` | search, generic search, `absolute_search`, logging, WIN32 `READABLE` |
| tex-file.c (format table, `init_path`, `init_format`, `find_file[_generic]`, suffix logic, `set_program_enabled`, `maketex_option`, `open_file`, fallback resolutions) | `texfile.rs` | all 59 formats in types.h order |
| tex-file.c (`kpathsea_name_ok`, `executable_filep`, `*_name_ok*`) | `nameok.rs` | `openin_any` / `openout_any` paranoia |
| fontmap.c | `fontmap.rs` | texfonts.map with `include` |
| tex-glyph.c, magstep.c, proginit.c | `glyph.rs` | dpi tolerance, aliases, mktexpk, fallback resolutions or font |
| tex-make.c | `make.rs` | `$SELFAUTOLOC\mktex*`, `MAKETEX_MAG`, missfont.log |
| debug.c, debug.h, lib.h (WARNING, FATAL), `perror` | `debug.rs` | stderr stream emulation (text mode, CRT buffering) |
| xputenv.c, CRT `getenv`/`putenv` | `env.rs` | case-insensitive, ANSI-code-page snapshot + overlay |
| absolute.c, find-suffix.c, rm-suffix.c, make-suffix.c, extend-fname.c, xbasename.c, xdirname.c, uppercasify.c, line.c, concat*.c, c-pathch.h, c-ctype.h | `util.rs` | |
| knj.c (`IS_KANJI`, code page conversion) and the Win32 calls of readable.c, dir.c, elt-dirs.c, file-p.c | `lib.rs` (`is_kanji`), `sys.rs` | |
| paths.h (compile-time defaults) | `texfile.rs` | `DEFAULT_TEXMFCNF` as built; every other default is `/nonesuch` (read from the real binary's `-debug=8` output) |
| kpsewhich.c, getopt.c (`getopt_long_only`) | `src/bin/rkpsewhich.rs` | reads the command line twice for pdftex-like names, as on WIN32 |

Not ported, because pdfTeX does not need them or `std` covers them:
kpsestat.c, access.c, readlink.c, xopendir.c, xfseek*/xftell*, xgetcwd.c,
mingw32.c, putenv.c, fn.c (replaced by `Vec<u8>`), the Unix-only
casefolding search (compiled out on Windows by `MONOCASE_FILENAMES`).

## API (crate `texrs_kpathsea`)

```rust
let mut k = Kpse::new("pdftex", None);        // kpse_set_program_name(argv[0], NULL)
k.init_command_line_encoding();               // texmfmp.c: command_line_encoding -> UTF-8 file names
k.reset_program_name("pdflatex");             // kpse_reset_program_name
k.xputenv("engine", "pdftex");                // xputenv
k.add_cnf_line("TEXINPUTS=.;$TEXMF/tex//");   // -cnf-line (kpathsea_cnf_line_env_progname)
k.set_program_enabled(FileFormat::Tfm, true, SrcLevel::Compile);
k.maketex_option("pk", false);                // -mktex / -no-mktex
k.init_prog("PDFTEX", 600, None, None);       // kpse_init_prog
let p: Option<Vec<u8>> = k.find_file("cmr10", FileFormat::Tfm, true);
let all = k.find_file_generic("texmf.cnf", FileFormat::Cnf, true, true);
let (path, info) = k.find_glyph("cmr10", 600, FileFormat::Pk).unwrap();  // GlyphFileInfo{name,dpi,format,source}
let v = k.var_value("TEXMFVAR");              // Option<Vec<u8>>
let e = k.var_expand("$TEXMF");               // also brace_expand, path_expand, expand
k.in_name_ok("x");  k.out_name_ok("../x.tex"); // + _silent / _extended variants
k.set_debug(32);                              // kpathsea_debug |= ...
k.set_record_input(Some(Box::new(|f: &[u8]| { /* .fls INPUT */ })));
k.set_record_output(Some(Box::new(|f: &[u8]| { /* .fls OUTPUT */ })));
Kpse::absolute_p("c:/x", false);
```

Also available:

- `with_exe_path` (sets SELFAUTOLOC from an explicit executable path).
- `readable_file`, `path_search_first`, `all_path_search`, `init_format`,
  `format_path`, `fontmap_lookup`, `make_tex`, `db_insert`, `open_file`.
- `set_make_tex_discard_errors`, `set_fallback_font`,
  `set_fallback_resolutions_string`, `set_client_path`, `set_override_path`,
  `set_suffixes`, `program_enabled`.
- `getenv`, `env_changes` (the variables to pass to `\write18` children),
  `to_os_path` (bytes → path for opening), `os_to_bytes`.
- `magstep_fix`, `bitmap_tolerance`, `program_basename`, `find_suffix`,
  `remove_suffix`.
- `stderr_write` and `flush_stderr`, described next.

`set_compat_def(false)` gives kpsewhich-style tracing. By default an instance
acts as the C global `kpse_def`, the way pdfTeX uses kpathsea, so the
`hash_lookup` and `fopen` debug traces that only `kpse_def` prints are
printed.

kpathsea's stderr output emulates the MSVC CRT stream. Line feeds become
CR LF, and output is **buffered when stderr is not a console**: C flushes at
`DEBUGF`, at `WARNING` and at exit, so for example "kpathsea: Running
mktextfm" reaches a redirected stderr after the child's own messages. The
engine should write its own stderr text through `stderr_write` and call
`flush_stderr()` before exiting. Dropping the `Kpse` also flushes.

## Deployment

SELFAUTOLOC is the directory of the executable. SELFAUTODIR,
SELFAUTOPARENT and SELFAUTOGRANDPARENT are its parent, grandparent and
great-grandparent. The C code gets them from `SearchPath(argv[0])` plus
long-name conversion; `Kpse::new` emulates that (application directory,
then the current directory, then `PATH`) and falls back to
`current_exe()`.

Putting the Rust binaries in `tools\texlive\2026\bin\rust-windows\` gives the
same SELFAUTODIR, SELFAUTOPARENT and SELFAUTOGRANDPARENT. It also finds the
same `texmf.cnf` files, because neither `bin\windows` nor `bin\rust-windows`
contains one. **Only `$SELFAUTOLOC` itself differs**, and pdfTeX uses it in
two places:

1. `kpse_make_tex` runs `$SELFAUTOLOC\mktextfm` (and `mktexpk`, `mktexfmt`, …).
2. Restricted `\write18` runs `"$SELFAUTOLOC/<cmd>"` (texmfmp.c).

Both need TeX Live's `bin\windows`. Use one of these:

- Set `TEXRS_SELFAUTOLOC=<TL>\bin\windows` in the environment. It overrides
  the directory in `Kpse::new`, and the differential tests use it.
- Construct with `Kpse::with_exe_path("<TL>/bin/windows/pdftex.exe", None)`,
  for example by mapping `…\bin\rust-windows` to its sibling `bin\windows`.
- Install the Rust binaries directly in `bin\windows`.

The program name comes from the executable name (`pdflatex.exe` →
`pdflatex`) unless one is passed.

Environment variables honoured, as in C: everything named in texmf.cnf (any
`VAR`, `VAR.progname` or `VAR_progname`), `TEXMFCNF`, `TEXMFDBS`,
`KPATHSEA_DEBUG`, `KPATHSEA_DEBUG_OUTPUT`, `KPATHSEA_DEBUG_APPEND`,
`KPATHSEA_WARNING`, `KPSE_DOT`, `TEXMFLOG`, `MISSFONT_LOG`,
`TEXMF_OUTPUT_DIRECTORY`, `TEXMFOUTPUT`, `TEX_HUSH`, `PATHEXT`, `HOME` and
`USERPROFILE`, `SHELL`, `TEXMFTEMP`, `<PROG>FONTS`, `<PROG>HEADERS`,
`<PROG>MAKEPK`, `<PROG>SIZES` and `TEXSIZES`. `TEXRS_SELFAUTOLOC` is the only
addition. The environment is snapshotted when the instance is created, as the
CRT does at start-up; later changes must go through `Kpse::xputenv`.

## Differential testing

`tests/difftest.py` copies `rkpsewhich.exe` to `target/difftest/rbin/kpsewhich.exe`
so the program name is identical, sets `TEXRS_SELFAUTOLOC` to TeX Live's
`bin\windows`, builds fixture trees, and runs every query through both
binaries with the same arguments, environment and working directory. Exit
status, stdout and stderr must match byte for byte. The only normalization is
`argv[0]` in getopt error messages, because the two executables sit in
different places.

The 1257 queries cover:

- names with and without suffix, case-only differences (on-disk case comes
  back via `getlongpath`), spaces, dots, `./` and `../`, backslashes, a
  directory named like a file, `$VAR` and `~` in names, and empty names;
- every format by name, abbreviation and number, under three program names;
- `-all`, `-must-exist` and `-subdir`, including the doubled output that
  pdftex-like program names produce in C;
- `-var-value`, `-var-brace-value`, `-expand-var`, `-expand-braces`,
  `-expand-path`, and `-show-path` for all 59 formats;
- `-path`, including `//`, `!!` and brace expansion;
- environment overrides: `TEXINPUTS` in all its forms (`.progname`,
  `_progname`, leading, trailing or doubled `;`, `!!`, backslashes),
  `KPSE_DOT`, `TEXMFCNF` (extra and missing directories), `TEXMFHOME`,
  `TEXMF` and `TEXMFDOTDIR`;
- a private texmf.cnf with every `do_line` error and comment form,
  continuation lines and self-reference;
- `-cnf-line`;
- a private ls-R tree: CRLF ls-R, hidden directories, entries missing on
  disk, files missing from ls-R, aliases including the upper-case-alias hash
  quirk, and `dir/name` lookups;
- texfonts.map aliases and `include`;
- PK lookups with dpi tolerance, `dpiNNN/` and `NNNpk` layouts, and `-mode`;
- the `-safe-*-name` checks with all `openout_any` values, `PATHEXT`,
  `TEXMFOUTPUT` and `TEXMF_OUTPUT_DIRECTORY`;
- `-debug` levels 1, 4, 8, 16, 32, 64, 63, 40 and −1, with stderr compared
  byte for byte;
- `-help-formats`, `-version` and getopt edge cases (abbreviations,
  ambiguity, missing arguments, `--`);
- the real `mktextfm` and `mktexpk` scripts with missfont.log handling;
- non-ASCII names in UTF-8 mode (pdftex) and ANSI mode;
- three other working directories.

Latest run: **1257 queries, 0 mismatches.**

`examples/initseq.rs` replays pdftex.exe's own start-up calls. Its stderr
with `KPATHSEA_DEBUG` set to 2, 4, 6 or −1 matches the real
`pdftex.exe -ini` byte for byte (148 lines at −1) once pointer values are
masked. `TEXMFLOG` files match apart from timestamps, and
`KPATHSEA_DEBUG_OUTPUT` files match.

## Timing

Measured on this machine, which is shared and noisy; the numbers are minima
over 30 or more runs.

Process level (`tests/bench.py`), one process per query, including process
creation (about 9 ms for any small exe from Python):

| scenario | C kpsewhich.exe | Rust rkpsewhich |
|---|---:|---:|
| `--version` (no kpathsea work) | 14.6 ms | 9.6 ms |
| `-var-value=TEXMF` (reads texmf.cnf and all ls-R) | 99.1 ms | 15.4 ms |
| cmr10.tfm, article.cls, pdftex.map, pdflatex.fmt | 113.6 ms | 21.7 ms |
| the 49 tree files a real `pdflatex` run of bench/corpus/02-article.tex reads | 147.1 ms | 24.4 ms |

In process (`examples/timing.rs`), Rust only:

| phase | time |
|---|---:|
| `set_program_name` (SELFAUTO*, environment snapshot) | 0.37 ms |
| texmf.cnf ×2 + 3 ls-R (1.1 MB, 61k entries) | 3.7 ms |
| first `.fmt` and `.tfm` lookups (reads texfonts.map and its includes) | 1.5 ms |
| 48 more lookups of that pdflatex run | 3.7 ms |

C spends about 85 ms initializing (texmf.cnf, then ls-R read one `getc` at a time with a `malloc` per
entry). The Rust port reads each file with one `read`, lower-cases it in place,
and indexes entries by offset into the buffer, using word-at-a-time line
scanning and an open-addressing table. Look-ups make the same file-system
probes as C (`GetFileAttributesW` for each candidate, `FindFirstFileA` for
the long name), because results must not depend on caching file existence.
The only cache C lacks holds the long names of directory prefixes inside
`getlongpath`; a directory's on-disk name cannot change during a run unless
it is renamed. On Windows,
`RUSTFLAGS=-C target-feature=+crt-static` saves a little more process
start-up time if the engine wants it.

## Behaviour that is not (or cannot be) reproduced

- Pointer values in pdfTeX-mode traces. `fopen(...) => 0x…`, `fclose(0x…)`,
  and the `hash_lookup` of the link table, which C prints as `%I64d` of a
  pointer. We print `0x1` and `0x0`, and the link count. The surrounding text
  is identical.
- Stack-overflow depth. A cnf value that brace-expands to itself plus
  something (`X = $X/x`, then `-var-brace-value=X`) recurses without end in
  both implementations. Both die with the same exit status, but after
  printing different numbers of "references itself" warnings.
- C crashes are emulated by their observable effect. `MISSFONT_LOG=0`
  followed by a failed mktex makes C dereference NULL: buffered stderr is
  lost and the exit status is 0xC0000005. We do the same. Other undefined
  behaviour, such as reading past the end of strings for `${` without `}`
  inside braces or an `aliases` line with no alias, follows the evident
  intent.
- `KPATHSEA_DEBUG_OUTPUT` redirects kpathsea's own stderr output. C
  redirects the whole process's fd 2.
- CJK ANSI code pages (932, 936, 950). Double-byte handling (`IS_KANJI`) is
  implemented where it uses the instance's code page. It is not implemented
  where C consults the global `kpse_def` (`xdirname`, `xbasename`,
  `make_suffix`, `hash()`). This machine's code page, 949, does not enable
  it.
- `SearchPath` emulation skips the system directories. Nobody runs pdftex
  from System32.
- Arguments to mktex scripts are quoted by Rust's `Command` when they contain
  spaces. C's `_spawnvp` does not quote. Font names are restricted to
  `[A-Za-z0-9+-_./\]`, so only an unusual `-mode` value could differ.
- Non-Windows builds compile and mostly work through `std::fs`, but parity
  is claimed only for Windows.
