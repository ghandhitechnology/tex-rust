# texrs-pdfread

A pure-Rust port of the parts of **xpdf 4.06** (as shipped in TeX Live 2026,
`libs/xpdf/xpdf-src`, no TeX Live patches touch these files) that pdfTeX's
`pdftoepdf.cc` uses to include PDF pages (`\pdfximage`). No dependencies and no
C code.

pdfTeX rewrites the included PDF's objects from xpdf's in-memory objects, so this
crate reproduces xpdf's parse exactly, including its handling of damaged input.
That covers int/real decisions, string bytes, dictionary order, duplicate keys,
xref and object-stream resolution, damaged-file recovery, decryption and decoded
stream bytes. Every function notes the C++ function it mirrors.

## Verification

There is no standalone xpdf binary, but TeX Live ships two programs built on the
same xpdf 4.06, and the tests use them as oracles. They are skipped if the
binaries are missing.

* `pdftosrc.exe file.pdf -1` dumps xpdf's xref table. `pdftosrc.exe file.pdf N`
  writes xpdf's fully decoded bytes of stream object N. `tests/xpdf_oracle.rs`
  compares both against this crate.
* `pdftex.exe` includes page 1 of a PDF; its `pdftoepdf.cc` re-serializes xpdf's
  objects. `tests/common/epdf.rs` then checks every object pdfTeX wrote against
  what `copyObject()` would write from *our* objects. That covers numbers via
  `convertNumToPDF`, string bytes, names, dictionary key order, arrays, the
  reference graph, undecoded stream bytes, decoded content arrays and the
  page-tree resource merge.
* With `\pdfinclusioncopyfonts=0`, pdfTeX replaces embedded Type 1 fonts and
  writes the encoding from xpdf's `Gfx8BitFont::getCharName()`. These encodings
  are compared with `GfxFont` (`tests/font_encodings.rs`, `tests/font_types.rs`).

Corpus: every `*.pdf` under `tools/texlive/2026/texmf-dist` and `bench/out` (802
files). By default the oracle tests use a deterministic sample.
`PDFREAD_ORACLE_FULL=1` checks everything, and `PDFREAD_ORACLE_THREADS=n` sets
the parallelism.

| test file | what |
|---|---|
| `src/*` unit tests | crypto vectors (MD5, SHA-2, AES FIPS-197, RC4), filters, predictor quirks |
| `tests/lexer_parser.rs` | lexer and parser edge cases |
| `tests/edge_cases.rs` | hand-made PDFs: numbers, strings, names, duplicate keys, xref streams with `/Index` and predictors, object streams with `/Extends`, hybrid files, `/Prev` chains, broken offsets, missing xref, garbage before the header, wrong, indirect, negative, missing or self-referential `/Length` (which
recurses 500 levels deep and still fits in a 1 MiB stack), every filter, page-tree inheritance, named destinations. Each file also goes through both oracles. |
| `tests/encryption.rs` | RC4-40 (R2), RC4-128 (R3), AES-128 (V4/R4), AES-256 (V5/R6), plus Identity, invalid and unknown handlers |
| `tests/corpus.rs` | opens all 802 corpus PDFs, walks every page and fetches and decodes every object without panicking |
| `tests/xpdf_oracle.rs` | xref dumps and decoded streams against `pdftosrc` |
| `tests/pdftex_inclusion.rs` | page 1 of corpus files included by `pdftex` and compared object by object |
| `tests/font_encodings.rs`, `tests/font_types.rs` | `GfxFont` encodings against xpdf, including all 876 embedded Type 1 and Type 1C fonts of the corpus |

## API: C++ → Rust mapping

xpdf's out-parameter style (`Object *lookup(key, Object *obj)`) becomes return
values. `Object::free()` is not needed, because compound values are reference
counted (`Rc`) and `Object::clone()` plays the role of `Object::copy()`. Getters
that assume the right type in C++, where anything else is undefined behaviour,
return a neutral value here (`0`, `0.0`, empty slice) or an `Option`.

| xpdf (C++) | this crate |
|---|---|
| `new PDFDoc(GString *fileName)` | `PdfDoc::open(path)` (also tries the lower-cased and upper-cased name, like xpdf); `PdfDoc::from_bytes(vec)` |
| `doc->isOk()`, `getErrorCode()` | `doc.is_ok()`, `doc.get_error_code()` (`error::codes::ERR_*`) |
| `doc->okToPrint()` | `doc.ok_to_print(false)` |
| `doc->getPDFVersion()` | `doc.get_pdf_version()` (`f64`, `atof` of the header token) |
| `doc->getXRef()`, `getCatalog()` | `doc.get_xref() -> Option<&Rc<XRef>>`, `doc.get_catalog() -> Option<&Catalog>` |
| `doc->findDest(GString*)` | `doc.find_dest(&[u8]) -> Option<LinkDest>` |
| `doc->getDocInfoNF(&obj)` / `getDocInfo` | `doc.get_doc_info_nf()` / `doc.get_doc_info()` |
| `catalog->getNumPages()` | `cat.get_num_pages()` |
| `catalog->getPage(i)` | `cat.get_page(i) -> Option<Rc<Page>>` (`None` when out of range) |
| `catalog->getPageRef(i)` | `cat.get_page_ref(i) -> Option<Ref>` (`{-1,-1}` for a page missing from the tree) |
| `catalog->findPage(num, gen)` | `cat.find_page(num, gen)` |
| `LinkDest::isOk()`, `getPageRef()` | `d.is_ok()`, `d.get_page_ref()` |
| `page->getMediaBox()` … `getArtBox()` | `page.get_media_box()` … `get_art_box()` → `&PDFRectangle {x1,y1,x2,y2}` |
| `page->getRotate()` | `page.get_rotate()` (normalized to 0..359) |
| `page->getGroup()`, `getMetadata()`, `getPieceInfo()`, `getSeparationInfo()`, `getBoxColorInfo()` | same names, `Option<&Rc<Dict>>` / `Option<&Rc<Stream>>` |
| `page->getResourceDict()` | `page.get_resource_dict() -> Option<&Rc<Dict>>` |
| `page->getContents(&obj)` | `page.get_contents() -> Object` (resolved) |
| `xref->fetch(num, gen, &obj)` | `xref.fetch(num, gen) -> Object` |
| `xref->getNumObjects()`, `getEntry(i)`, `getTrailerDict()` | `get_num_objects()`, `get_entry(i) -> XRefEntry`, `get_trailer_dict()` |
| `Object::isInt()` etc., `getType()`, `getTypeName()` | `is_int()` etc., `get_type()`, `get_type_name()` |
| `isName("X")`, `isDict("T")`, `isCmd("c")` | `is_name_of("X")`, `is_dict_of("T")`, `is_cmd_of("c")` |
| `getInt()`, `getReal()`, `getNum()`, `getBool()` | same, snake_case |
| `getString()` (GString) | `get_string() -> &[u8]` (may contain NUL bytes) |
| `getName()` (char*) | `get_name() -> &[u8]` (never contains NUL) |
| `getRef()`, `getRefNum()`, `getRefGen()` | same, snake_case |
| `getArray()`, `getDict()`, `getStream()` | `Option<&Rc<Array>>`, `Option<&Rc<Dict>>`, `Option<&Rc<Stream>>` |
| `obj->fetch(xref, &obj2)` | `obj.fetch(Some(&xref))` |
| `arrayGetLength()`, `arrayGet(i)`, `arrayGetNF(i)` | `array_get_length()`, `array_get(i)`, `array_get_nf(i)` (also on `Array`: `get_length`, `get`, `get_nf`) |
| `dictGetLength()`, `dictLookup(k)`, `dictLookupNF(k)`, `dictGetKey(i)`, `dictGetVal(i)`, `dictGetValNF(i)` | `dict_get_length()`, `dict_lookup(k)`, `dict_lookup_nf(k)`, `dict_get_key(i)`, `dict_get_val(i)`, `dict_get_val_nf(i)` (and on `Dict`: `get_length`, `lookup`, `lookup_nf`, `get_key`, `get_val`, `get_val_nf`, `iter`) |
| `obj->initDict(xref)` + `dictAdd(key, val)` | `Dict::new(Some(&xref))` + `dict.add(key, val)`; `Object::dict_add` |
| `streamGetDict()` | `obj.stream_get_dict()` / `stream.get_dict()` |
| `getStream()->getUndecodedStream()` + `reset()` + `getChar()` loop | `stream.get_undecoded_data() -> Vec<u8>` or `stream.get_undecoded_stream() -> StreamReader` (`reset`, `get_char` returning `EOF` = -1, `look_char`, `get_block`) |
| `getStream()` + `reset()` + `getChar()` loop (decoded) | `stream.get_decoded_data() -> Result<Vec<u8>, UnsupportedFilter>` or `stream.reader()` |
| `GfxFont::makeFont(xref, tag, ref, dict)` | `GfxFont::make_font(&xref, tag, ref, &dict)` |
| `font->isCIDFont()`, `((Gfx8BitFont*)font)->getCharName(i)` | `font.is_cid_font()`, `font.get_char_name(i) -> Option<&[u8]>` |
| `globalParams->setErrQuiet()`, `setErrorCallback()` | `set_err_quiet()`, `set_error_callback()` |

## What was ported from where

| xpdf file | Rust module | notes |
|---|---|---|
| `Object.h/.cc`, `Array.*`, `Dict.*` | `object.rs` | Dict keeps insertion order. A hash index is added once a dict has more than 16 entries (xpdf always hashes); this is not observable. |
| `Lexer.*` | `lexer.rs` | literal port |
| `Parser.*` | `parser.rs` | includes `makeStream` and the inline-image `ID` quirk |
| `XRef.*` | `xref.rs` | tables, xref streams, hybrid, `/Prev`, object streams with the 128-entry cache, the 16-entry object cache, `constructXRef` with its 4 KB scan buffer |
| `Stream.*` (Stream, FileStream, EmbedStream, ASCIIHex, ASCII85, LZW, RunLength, Flate, StreamPredictor, EOFStream, `addFilters`/`makeFilter`) | `stream.rs`, `filters.rs` | Decoders are applied to the whole input (identical output). FlateDecode is xpdf's **own** inflate (`FlateStream`), ported literally. |
| `Decrypt.*`, `SecurityHandler.*` | `decrypt.rs`, `security.rs` | standard handler R2–R6, RC4/AES-128/AES-256, MD5/SHA-256/384/512 |
| `Catalog.*` | `catalog.rs` | page tree (lazy `loadPage2`), `countPageTree`, `findPage`, `findDest`/`findDestInTree` |
| `Page.*` | `page.rs` | `PageAttrs` (inheritance, resource merge, `readBox`, `clipBoxes`), `Page` |
| `Link.*` (LinkDest) | `link.rs` | |
| `PDFDoc.*` | `doc.rs` | `setup`/`setup2` (repair retry), `checkHeader`, `checkEncryption` |
| `GfxFont.*` (makeFont, getFontType, Gfx8BitFont encoding) | `gfxfont.rs` | widths, ToUnicode and font descriptors are not ported (pdfTeX never reads them) |
| `fofi/FoFiIdentifier`, `FoFiType1`, `FoFiType1C` (name, encoding, charset) | `fofi.rs` | |
| `FontEncodingTables.cc`, `fofi/FoFiEncodings.cc` | `font_encoding_tables.rs`, `fofi_tables.rs` | generated from the C sources |
| `Error.*` (with TeX Live's `libxpdf:` label) | `error.rs` | messages go to stderr as `\nlibxpdf: <Category> (<pos>): <msg>\n` |

Not ported, because pdfTeX does not use them or they have no effect on what it
reads: rendering (Gfx, OutputDevs), AcroForm loading, outlines, optional
content, page labels, embedded-file lists, and the CCITTFax, DCT, JBIG2 and JPX
decoders. A stream whose filter chain contains one of these returns
`Err(UnsupportedFilter)` from `get_decoded_data()`; its undecoded data is still
available. pdfTeX decodes streams only for page content arrays and embedded
font files, where these filters do not occur in valid files.

**Inflate / zlib.** xpdf does not use zlib for reading: `FlateStream` is its own
inflate, and it differs from zlib on damaged data. There is no Adler-32 check,
partial codes are accepted at EOF, length codes 286 and 287 are accepted,
buffered bytes are discarded before a stored block, and decoding is cut off at
the decompression-bomb limit (50 MB output at a 200:1 ratio). The single entry
point `filters::flate_decode` must therefore stay this port, and swapping in
`port/crates/zlib` would change behaviour. The byte-exact zlib port is needed for
*writing* only.

## Encryption decision

**Ported.** pdfTeX can reach it with default settings. `new PDFDoc(name)` passes
no passwords, xpdf then tries the empty user password, and pdfTeX rejects the
file only if `!okToPrint()`. So any encrypted PDF with an empty user password
and print permission is included. Its strings are decrypted, and its streams
are copied as `getUndecodedStream()`, which for an encrypted file is the
*decrypted* but still filtered data, because `DecryptStream::getUndecodedStream()`
returns itself. All of the standard security handler is ported:

* V1/V2 R2/R3 RC4
* V4 R4 crypt filters V2 (RC4) and AESV2
* V5 R5/R6 AESV3, including the R6 hash
* `/EncryptMetadata false`
* metadata streams marked with the `/Crypt` filter
* xpdf's rule that an Encrypt dictionary missing V/R/P/O/U makes the file
  *unencrypted*
* Identity crypt filters, which fail to open

## Behaviour that affects re-serialization

**Numbers** (`Lexer::getObj`):
* A token is a real only if it contains `.`. There is no exponent syntax:
  `1e5` lexes as `1` followed by the command `e5`.
* Integers accumulate in 32-bit `int` and wrap on overflow, as compiled C does:
  `2147483648` gives `-2147483648` and `4294967296` gives `0`. Large integers do
  *not* become reals (that is poppler behaviour, not xpdf).
* A real is accumulated as `xf = xf + scale*digit; scale *= 0.1`, which is not
  the correctly rounded decimal value. The integer part stops accumulating once
  it reaches 1e20.
* Adobe quirks: `--5` → `0`, `50-100` → `50`, `-` and `+` → `0`, `-.` → real
  `-0.0`, and a `-` inside the fraction is skipped with a warning (`1.-2` →
  `1.2`). `+3` → `3`, `.5` → `0.5`, `1.` → `1.0` (real), `-.0` → `-0.0`.

**Strings:**
* CR, LF and CRLF inside `(...)` become LF.
* Backslash-newline (`\`+LF, `\`+CR, `\`+CRLF) is a line continuation.
* Octal escapes take up to three digits and keep the low 8 bits (`\777` →
  `0xFF`). Unknown escapes drop the backslash.
* A string unterminated at EOF keeps what was read.
* Hex strings skip PDF whitespace. An odd final digit is padded with 0
  (`<414>` → `A@`). An illegal character still counts as a digit with value 0,
  and lexing stops after 100 illegal characters, leaving the rest unlexed.

**Names:**
* `#xx` is decoded.
* If the first character after `#` is not hex, the `#` is kept.
* If only the second is bad, the *nibble value* of the first is stored, e.g.
  `/a#2 ` → `a\x02`.
* A NUL produced that way truncates the name, because xpdf's names are C
  strings (`/a#0G` → `a`).
* `#00` makes the whole token an error object.
* Names have no length limit. Command tokens are cut at 127 characters, and the
  128th character is dropped.

**Dictionaries:**
* Entries keep the order of first occurrence.
* A repeated key keeps its first position and takes the *last* value
  (`<< /A 1 /B 2 /A 3 >>` → `A 3, B 2`).
* Non-name keys are skipped one token at a time.
* A key followed by EOF or an error token is dropped.
* `/K >>` takes the `>>` command as the value, and parsing continues.

**Arrays and objects:**
* Damaged input can leave `Cmd`/`Error` objects inside arrays.
* `n g R` with a negative number gives an error object.
* Nesting deeper than 500 returns the `[`/`<<` token as a command.

**References:**
* `lookup()`/`get()` resolve references through the XRef the container was
  parsed with. Trailer dictionaries and xref-stream dictionaries have none until
  `setXRef`.
* A reference to a free, missing or out-of-range object, or to an uncompressed
  object with the wrong generation, fetches as `Null`.
* Compressed objects ignore the generation number.
* An object stream index that doesn't match the object number gives `Null`.

**Streams** (`Parser::makeStream`):
* `/Length` is resolved through the XRef. A negative value is taken as unsigned
  32-bit.
* If `endstream` does not follow, within 100 whitespace characters, xpdf adds
  5000 to the length. The undecoded data then runs into the following bytes
  (clamped at EOF).
* If `/Length` is not an integer, xpdf searches for `endstream`. After every `e`
  it reads 8 bytes without rewinding, so an `e` just before `endstream` hides it
  and the stream becomes an error object.
* In a repaired file the length comes from the `endstream` positions found by
  the scan.
* Decoded quirks:
  * ASCIIHex without `>` ends with an extra 0 byte.
  * ASCII85 uses the value of the terminating `~`/EOF at its own position when
    padding a final partial group.
  * RunLength writes 0xFF for bytes missing at EOF.
  * A PNG/TIFF predictor outputs a truncated last row in full, with the
    previous row's stale tail.
  * Invalid predictor parameters drop the predictor silently.
  * An unknown filter yields an empty stream.

**XRef:**
* The `%PDF-` header may be preceded by up to 1019 bytes of garbage. All
  offsets are relative to the header.
* `startxref` is searched in the last 1024 bytes.
* `/Prev` loops are detected, and `/Prev n 0 R` is accepted.
* `/XRefStm` entries fill only unset slots.
* The first section read (the newest) wins.
* The IBM "xref starts at 1" bug is fixed up.
* A file is repaired when the xref can't be read, the trailer has no `/Root`,
  or the catalog is invalid. The repair scan runs over a 4 KB buffer, exactly
  as xpdf's does. In repair mode object streams are found by scanning, and in
  encrypted files they are parsed *undecrypted* (as in xpdf).

**Page tree:**
* MediaBox, CropBox and Rotate are inherited. The other boxes default to the
  crop box and are clipped to the media box.
* Rotate is normalized to the range 0–359.
* Resources are *merged* when both a parent and a child node have them.
  Parent sub-dictionaries come first, child entries are added in place, and
  non-dictionary resource entries such as `/ProcSet` are *dropped* from the
  merged dictionary.
* A page that cannot be loaded becomes a 50×50 dummy page with the ref
  `{-1,-1}`.
* `/Count` 0 or above 50000 triggers a tree count.

**Encodings (GfxFont):**
* The base encoding is taken from `/Encoding` (or `/BaseEncoding`), then from
  the embedded font's built-in encoding (FoFiType1: `/Encoding
  StandardEncoding def` or `dup n /name put` lines; FoFiType1C: CFF charset and
  encoding), then from the default.
* A Type 1C encoding taken from the font file has its gaps filled from
  StandardEncoding.
* `/Differences` is applied last.

## Places where exact equality with xpdf is uncertain or impossible

* **Uninitialized memory in xpdf.** None of these affect parsing results in
  practice:
  * `gen` of never-set xref entries (`pdftosrc` dumps garbage there; the tests
    ignore it).
  * `LinkDest::getPageRef()` for a destination given by page *number* (a
    union; this crate returns generation 0).
  * `getNum()` on a non-number, which pdfTeX does for a missing `/StemV`.
  * Stale CFF operands when an operator has no operands.
* **Signed integer overflow** in the lexer is assumed to wrap. The oracle tests
  confirm this for the TL Windows binary.
* **Out of memory.** xref tables that would need more than 2^27 entries are
  treated as an allocation failure. xpdf would abort in `greallocn`.
* **Decoder state on re-read.** xpdf's `FlateStream` does not clear its 32 KB
  window on `reset()`, so a second read of the *same* stream object with an
  invalid back-reference distance could differ. Here every read starts from a
  zeroed window.
* **Error message positions.** The `(pos)` printed for diagnostics is
  approximate for some messages. The message texts follow xpdf.
* **Object cache.** The 16-entry cache is ported exactly. The catalog does not
  load AcroForm, outlines and the like, so the cache contents can differ from
  xpdf after opening. That is only observable in one exotic case: an encrypted
  file whose `/ID` array is an indirect object that is later evicted and
  re-fetched decrypted.
* **`atof`** (PDF header version, CFF reals) is a strtod-compatible
  reimplementation; hex floats are handled approximately.
