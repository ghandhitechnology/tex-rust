# work/images: image inclusion tests

Differential tests for the image ports in `port/engine/src/rt/pdf/images/`:

| file | ports |
|---|---|
| `mod.rs` | writeimg.c (image_array, readimage, writeimage, deleteimage, the getters, dumpimagemeta/undumpimagemeta), image.h |
| `libpng.rs` | the libpng 1.6.55 read path writepng.c uses (chunk handling with the 1.6.47+ table, ICC checks, tRNS/strip alpha/strip 16/gamma/interlace transforms, gamma tables, row unfiltering, Adam7) |
| `writepng.rs` | writepng.c (decode path, PNG copy path, /SMask, transparency group) |
| `writejpg.rs` | writejpg.c (SOFn, JFIF and Exif resolution) |
| `writejbig2.rs` | writejbig2.c (sequential and random-access files, page 0 globals) |
| `pdftoepdf.rs` | pdftoepdf.cc on the pdfread crate (xpdf 4.06 port) |
| `epdf.rs` | epdf.c (font replacement for `\pdfinclusioncopyfonts=0`, on rt/pdf/fonts) |

## Tests

Run from this directory. The engine tested is `port/engine` (or `$TEXRS_ENGINE_DIR`,
or `engine/` here if it exists); the TeX Live tree is `$TEXRS_TEXLIVE` (default
`tools/texlive/2026` at the top of the checkout).

```
python -I tools/mkpng.py cases/png                 # PNG inputs (already in the repository)
python -I tools/mkmisc.py cases/misc <mwe dir>     # JPEG, JBIG2, PDF inputs (likewise)
python -I tools/cases.py [NAME-SUBSTRING ...]      # differential tests (TL pdftex vs the engine)
python -I tools/corpus_all.py                      # all texmf-dist PDFs < 400 KB
```

`cases/corpus.txt` and `cases/corpus_all.txt` list PDFs relative to `texmf-dist` of the
TeX Live installation described by `bench/setup/texlive.profile.template`.

`png_padding_knowndiff` is expected to differ: for images with less than 8 bits per pixel
whose rows end in a partial byte, libpng keeps the padding bits from pdfTeX's malloc'ed,
uninitialized row buffers, so TL's own output varies from run to run there.
