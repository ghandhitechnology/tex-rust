# texrs-zlib

A pure-Rust port of **zlib 1.3.2**, exactly as TeX Live 2026 bundles it
(`libs/zlib/zlib-src`), for the texrs pdfTeX port. It has no dependencies, no
C code, no `build.rs`, and no dev-dependencies.

The goal is byte identity, not just compatibility. The same input and the
same sequence of calls give the same output bytes, return codes,
`avail_in`/`avail_out`, `total_in`/`total_out`, `adler`, `data_type` and
`msg` as C zlib 1.3.2. That includes the cases that depend on buffer sizes,
such as level-0 stored blocks and repeated flushes into a full output buffer.

## What is ported from where

| C file | Rust module | Notes |
|---|---|---|
| `deflate.c`, `deflate.h` | `src/deflate.rs` | State machine, `fill_window`, hash chains, `longest_match`, `deflate_stored/fast/slow/rle/huff`, config table, gzip header writer, `deflateParams/Reset/SetDictionary/Prime/Tune/Bound/Pending/Used/Copy` |
| `trees.c`, `trees.h` | `src/trees.rs`, `src/tables.rs` | Huffman construction, block-type choice, 16-bit bit buffer. `sym_buf` is overlaid on `pending_buf` as in C. The tables are copied verbatim from `trees.h` by `tools/gen_tables.py` |
| `inflate.c`, `inflate.h` | `src/inflate.rs` | The full mode machine, including gzip headers, `Z_BLOCK`/`Z_TREES`, sync, dictionaries, prime, mark, validate |
| `inftrees.c`, `inffixed.h` | `src/inftrees.rs`, `src/tables.rs` | `inflate_table`. The fixed tables are copied verbatim from `inffixed.h` |
| `inffast.c` | `src/inffast.rs` | `inflate_fast` |
| `adler32.c`, `crc32.c` | `src/adler32.rs`, `src/crc32.rs` | `uLong` arithmetic is 32-bit, as on Windows. CRC uses slicing-by-16, which gives the same values. Both `*_combine` functions are included |
| `compress.c`, `uncompr.c` | `src/compress.rs` | Same chunking loops (`uInt`-sized pieces) |
| `gzlib.c`, `gzwrite.c`, `gzread.c`, `gzclose.c`, `gzguts.h` | `src/gz.rs` | Works over any `std::io::Write`/`Read` instead of a file descriptor. `GZBUFSIZE` 8192, `want`/`size`, the `2*want` input buffer, buffered-or-direct writes and `gz_comp`'s flush logic all match C |
| `zlib.h` API | `src/lib.rs`, `src/zlib.rs` | The safe API plus C-named wrappers |

Other compile-time choices match TeX Live's Windows build. `OS_CODE` is 10 on
Windows (zutil.h, `WIN32`), 3 on Unix and 19 on macOS. `MAX_MEM_LEVEL` is 9.
`LIT_MEM`, `FASTEST`, `ZLIB_DEBUG`, `INFLATE_STRICT` and
`INFLATE_ALLOW_INVALID_DISTANCE_TOOFAR_ARRR` are not set. The release-build
`_tr_tally_*` macros are used, so `s->matches` is only counted by
`deflate_stored`.

## API mapping

Safe API: each call takes the caller's input slice (`next_in`/`avail_in`)
and output slice (`next_out`/`avail_out`). It returns a `StreamResult
{ status, consumed, produced }`. Advancing your slices by `consumed` and
`produced` is exactly C's pointer update. Lengths are capped at `u32::MAX`
per call, as with `uInt`.

| C | Rust (safe) | Rust (`texrs_zlib::zlib`, C names) |
|---|---|---|
| `deflateInit(s, level)` | `Deflate::new(level)` | `deflateInit(&mut z_stream, level)` |
| `deflateInit2(s, l, m, w, ml, st)` | `Deflate::with_params(l, m, w, ml, st)` | `deflateInit2(..)` |
| `deflate(s, flush)` | `d.deflate(input, output, Flush::X)` / `deflate_raw(.., i32)` | `unsafe deflate(&mut z_stream, flush)` |
| `deflateParams` | `d.params(input, output, level, strategy)` (it may run `deflate(Z_BLOCK)`) | `unsafe deflateParams(..)` |
| `deflateReset`, `deflateResetKeep` | `d.reset()`, `d.reset_keep()` | same names |
| `deflateEnd` | `d.end()` → `Status::DataError` if busy | `deflateEnd(..)` |
| `deflateBound` / `deflateBound_z` | `d.bound(n) -> u64`, `deflate_bound_no_stream(n)` | `deflateBound(Option<&z_stream>, uLong) -> uLong` (saturates like C), `deflateBound_z` |
| `deflateSetDictionary` / `GetDictionary` | `d.set_dictionary(&[u8])`, `d.get_dictionary()` | same names |
| `deflateSetHeader` | `d.set_header(GzHeader)` | `deflateSetHeader` |
| `deflatePrime`, `deflateTune`, `deflatePending`, `deflateUsed` | `prime`, `tune`, `pending`, `used` | same names |
| `deflateCopy` | `d.clone()` | `deflateCopy` |
| `inflateInit`, `inflateInit2(w)` | `Inflate::new()`, `Inflate::with_window_bits(w)` (8..15, 0, -8..-15, +16, +32) | same names |
| `inflate(s, flush)` | `i.inflate(input, output, Flush::X)` / `inflate_raw` | `unsafe inflate(..)` |
| `inflateReset`, `Reset2`, `ResetKeep`, `End` | `reset`, `reset2`, `reset_keep`, `end` | same names |
| `inflateSetDictionary` / `GetDictionary` | `set_dictionary`, `get_dictionary` | same names |
| `inflateSync`, `SyncPoint`, `Prime`, `Mark`, `CodesUsed`, `Validate`, `Undermine` | `sync(input)`, `sync_point`, `prime`, `mark`, `codes_used`, `validate`, `undermine` | same names |
| `inflateGetHeader` | `i.get_header(GzHeader)`, then `i.header()` | `inflateGetHeader`, `inflateHeader` |
| `compress`, `compress2`, `compressBound` | `compress(dest, src)`, `compress2(dest, src, level) -> (code, len)`, `compress_bound`, `compress_to_vec` | `compress2(dest, &mut destLen, src, level)` etc. |
| `uncompress`, `uncompress2` | `uncompress(..)`, `uncompress2(..) -> (code, dest_len, src_used)`, `uncompress_to_vec` | same signatures with `&mut uLong` |
| `adler32`, `adler32_z`, `crc32`, `crc32_z`, `*_combine*`, `get_crc_table` | same names | re-exported |
| `zError`, `zlibVersion` | `err_msg`, `zlib_version` | `zError`, `zlibVersion` |
| `gzdopen(fd, "wb")`, `gzopen` | `GzWriter::with_mode(w, mode, name)`, `GzWriter::new(w)`, `GzWriter::create(path, mode)` | `gzdopen(File, mode)`, `gzopen(path, mode)` → `gzFile` |
| `gzdopen(fd, "rb")` | `GzReader::with_mode(r, mode, name)`, `GzReader::new(r)`, `GzReader::open(path)` | same |
| `gzwrite`, `gzfwrite`, `gzputc`, `gzputs`, `gzprintf`, `gzflush`, `gzsetparams`, `gzbuffer`, `gzseek`, `gztell`, `gzerror`, `gzclearerr`, `gzclose` | methods of the same name on `GzWriter` (`gzprintf` takes a formatted `&str`; `finish()` returns the writer) | functions of the same name |
| `gzread`, `gzfread`, `gzgetc`, `gzungetc`, `gzgets`, `gzeof`, `gzdirect`, `gzseek`, `gzrewind`, `gztell`, `gzclose` | methods on `GzReader` (`gzrewind`/`gzseek` need `Read + Seek`; `gzseek_forward` works on any `Read`) | functions of the same name |

`GzReader` implements `std::io::Read`. `GzWriter` implements
`std::io::Write`; its `flush()` flushes only the underlying writer, because
calling `gzflush` would change the bytes. In `zlib::z_stream`, `uLong` is the
platform `c_ulong`, 32 bits on Windows as in C. Custom allocators are not
supported: `zalloc`/`zfree`/`opaque` do not exist, and allocation failure
aborts instead of returning `Z_MEM_ERROR`.

pdfTeX's two uses translate one to one:
- `writezip()`: `deflateInit(level)`, a persistent 32 KiB output buffer,
  `deflate(Z_NO_FLUSH)` / `deflate(Z_FINISH)`, `deflateReset`, and
  `deflateEnd`/`deflateInit` when the level changes. A literal
  transliteration is in `tests/compat.rs`.
- Format files: `gzdopen(fd, "wb")`, `gzsetparams(f, 1, Z_DEFAULT_STRATEGY)`,
  `gzwrite`…, `gzclose`; and `gzdopen(fd, "rb")`, `gzread`, `gzclose`.

## Tests and results

Run `cargo test --release`, or `cargo test` with overflow checks; both pass.
They need Python 3 (`python -I`, run from the tests) with `zlib` 1.3.2, and
a C zlib 1.3.2 DLL. By default that is Git for Windows'
`C:\Program Files\Git\mingw64\bin\zlib1.dll`, which is 1.3.2 and exports the
whole API; set `TEXRS_ZLIB_ORACLE_DLL` to use another. The oracle refuses to
run unless `zlibVersion()` is `"1.3.2"`.

Three independent oracles are used:

1. **Call-by-call replay against C zlib 1.3.2** (`tests/common/mod.rs`,
   `tests/oracle/zoracle.py`). Each case is a sequence of explicit calls
   with explicit `avail_in`, `avail_out` and flush values. Rust runs the
   calls and records a trace. The script replays the same calls through
   ctypes against C. The two traces must be identical: return code,
   `avail_*`, `total_*`, `adler`, `data_type`, `msg` and the CRC of each
   call's output. Every output byte of every case must also match.
2. **CPython's own `zlib` module**, also 1.3.2 (`tests/python_zlib.rs`):
   `compressobj(level, method, wbits, memLevel, strategy)` with
   `compress()`/`flush(mode)`, plus `zlib.compress()`. The Rust side copies
   CPython's output-buffer growth schedule, so level 0 matches too.
3. **Real TeX Live output** (`tests/texlive_fmt.rs`). Every installed TL 2026
   format file written by pdftex.dll / euptex.dll is decompressed and then
   recompressed with `GzWriter` using pdfTeX's call pattern. The result must
   equal the installed file.

| Test | Cases | Calls / bytes | Result |
|---|---|---|---|
| deflate: levels 0–9 × strategies 0–4 × 15 inputs (empty, 1–3 B, tiny, repetitive ×4, random, text, PDF content streams, fmt-like binary, far matches, 1.55 MB mix), in pdfTeX writezip() pattern | 750 | 11,010 | identical |
| deflate: windowBits 8–15 × raw/zlib/gzip × memLevel 1–9 × 8 level/strategy pairs (+ deflateBound) | 3,456 | 64,108 | identical |
| deflate: input chunks 1 B…64 KB/random × output buffers 1 B…100 KB/random, all levels (⅓ level 0), wbits/memLevel mix | 700 | 1,513,314 | identical |
| deflate: random mixed flushes (NO/PARTIAL/SYNC/FULL/BLOCK, repeated), then FINISH and useless calls after it | 500 | 32,904 | identical |
| deflate: `deflateParams` mid-stream (all levels/strategies incl. 0↔n, pending input, tiny output, `Z_BUF_ERROR` retries) | 500 | 8,271 | identical |
| deflate: pdfTeX reuse (`deflateReset`, End/Init on level change), `deflateResetKeep`, `deflateCopy` | 24 | 708 | identical |
| deflate: dictionaries, prime, tune (odd values), bound (incl. >4 GiB), pending/used, gzip headers (name/comment/extra/hcrc, 1–64 B output), init/flush/state errors | 144 | 21,323 | identical |
| adler32/crc32 (NMAX edges, invalid init values), combine, compressBound, compress2 (dest 0/10/large, invalid level) | 2 | 981 | identical |
| inflate: 120 stream kinds (all levels, strategies, wrappers, flushes) × 3 windowBits choices (exact/0/auto) × random 1 B…100 KB in / 1 B…200 KB out × NO_FLUSH/SYNC/FINISH/BLOCK/TREES | 360 | 1,189,576 | identical |
| inflate: 700 corrupted streams (bit flips, truncation, header/trailer damage, garbage) + 17 crafted ones. All 18 inflate error messages occur; uncompress2 | 718 | 6,451 | identical |
| inflate: dictionaries/`Z_NEED_DICT`, `inflateSync`, gzip header capture, multi-member gzip, validate, prime, reset2, init errors | 56 | 35,339 | identical |
| gz: pdfTeX fmt dump pattern (gzsetparams(1,0), 4/8-byte and large writes) on 2.5 MB / 600 KB data | 12 | 1,084 | identical files |
| gz: 14 mode strings (levels, f/h/R/F, T), gzbuffer, gzputc/puts/printf, gzflush, gzsetparams, gzseek, multiple members | 160 | 5,413 | identical files |
| gz: gzread/getc/ungetc/gets/eof/direct/seek/tell/error on valid, multi-member, garbage-trailed, transparent, empty, truncated and corrupt files | 240 | 9,442 | identical |
| CPython `zlib` module: 750 level×strategy, 3,024 wbits×memLevel×level, 400 chunked/mixed-flush, 660 `zlib.compress` | 4,834 | 577.6 MB output | identical |
| TeX Live 2026 `.fmt` files (pdftex, etex, latex, pdflatex(-dev), amstex, cont-en, mptopdf, pdfetex; ptex, eptex, euptex, platex(-dev), uptex, uplatex(-dev)) | 18 | 90.7 MB uncompressed | identical files |
| Round trips (all levels × strategies × 6 window/memLevel sets × 15 inputs) | 4,500 | — | all OK |
| Compat layer: literal writezip() port, `z_stream` inflate, gz functions | 3 | — | OK |

In total, 7,622 replayed cases with 2.90 M calls are identical to C,
4,834 CPython cases are identical, and the 18 TeX Live files are identical.
No case differs.

### Known deviations: only where C has undefined behaviour

The replay oracle found three places where C zlib 1.3.2 reads uninitialized
memory. C's output there depends on heap contents. The port uses the values
the code intends, and the tests keep away from these paths:

1. Calling `deflatePrime()` while output is still pending (`pending_out !=
   pending_buf`) makes C append at `pending_buf + pending` and later emit
   bytes of `pending_buf` that were never written. zlib.h only supports
   priming before the first `deflate()`, and that use is identical.
2. In write mode, `gz_reset()` never initializes `state->past`. `gztell()`
   and `gzseek(.., SEEK_CUR)` read it while a seek is pending. The port
   uses 0.
3. A `gzseek()` before the first write leaves `gz_zero()`'s one-time
   `memset` running on a buffer that is not allocated yet (length 0). C then
   writes uninitialized heap bytes where the zeros should be. The port
   writes zeros.

The other differences cannot change output:
- `Z_MEM_ERROR` is never returned, because allocation failure aborts.
- gz error texts use `std::io::Error` messages instead of `strerror()`.
- After a non-blocking stall while compressing straight from the caller's
  buffer, `GzWriter` does not keep a pointer into that buffer. The caller
  passes the rest again.

## Speed

`cargo run --release --example bench -- <file>` runs the Rust side and
`python -I tools/bench_python.py <file>` runs the C side. The input is the
10 MiB corpus from `tools/make_corpus.py`: 4 MiB of LaTeX sources, 4 MiB of
uncompressed pdflatex.fmt and 2 MiB of synthetic PDF content streams. Each
figure is the best of 7 runs. The ranges cover 3 Rust sessions and 2 C/Python
sessions taken back to back on a shared Windows 11 machine, so expect about
±10% noise. All three produce identical compressed bytes: 3,458,056,
2,895,235 and 2,856,669 bytes.

| best of 7, 10 MiB | texrs-zlib | C zlib 1.3.2 direct (zlib1.dll, one `compress2`/`uncompress`) | CPython `zlib.compress`/`decompress` |
|---|---|---|---|
| deflate level 1 | 89–103 ms | 88–95 ms | 119–135 ms |
| deflate level 6 | 563–568 ms | 531–639 ms | 615–648 ms |
| deflate level 9 | 1574–1753 ms | 1501–1530 ms | 1635–1917 ms |
| inflate of L1 data | 36–38 ms | 33–34 ms | 36–41 ms |
| inflate of L6 data | 30–36 ms | 27–28 ms | 33–36 ms |
| inflate of L9 data | 28–34 ms | 27–31 ms | 27–36 ms |

Deflate is on par with direct C: equal at levels 1 and 6, and up to about 10%
slower at level 9. It beats CPython's `zlib.compress` at every level, because
CPython adds its own buffer handling on top of C. Inflate is within about
10% of direct C and on par with CPython.

## Layout

```
src/        the port (lib.rs, deflate.rs, trees.rs, tables.rs, inflate.rs,
            inftrees.rs, inffast.rs, adler32.rs, crc32.rs, compress.rs,
            gz.rs, zlib.rs)
tests/      oracle harness (common/mod.rs, oracle/zoracle.py) and test suites
tools/      gen_tables.py, make_corpus.py, bench_python.py
examples/   bench.rs, zpipe.rs
```
