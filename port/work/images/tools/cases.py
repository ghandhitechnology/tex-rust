"""Image-inclusion differential tests for the images working copy.

usage: python -I cases.py [NAME-SUBSTRING ...]
Writes cases/gen/<name>.tex, then runs port/tests/difftest.py (TL pdftex vs the Rust
engine in work/images/engine): plain format, then the document.
"""
import os
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent.parent  # work/images
ROOT = HERE.parents[2]  # texrs
CASES = HERE / "cases"
GEN = CASES / "gen"
GEN.mkdir(exist_ok=True)
PNG = CASES / "png"
MISC = CASES / "misc"
# TEXRS_TEXLIVE: the TeX Live 2026 installation (default tools/texlive/2026)
TEXMF = pathlib.Path(os.environ.get("TEXRS_TEXLIVE", ROOT / "tools/texlive/2026")) / "texmf-dist"
MWE = TEXMF / "tex/latex/mwe"

ENV = dict(os.environ)
if (HERE / "engine").is_dir():  # a working copy of the engine (default: port/engine)
    ENV.setdefault("TEXRS_ENGINE_DIR", str(HERE / "engine"))
ENV.setdefault("TEXRS_WORK", str(HERE / "test"))

PRE = r"\pdfoutput=1 \nopagenumbers \parindent=0pt \vsize=100in \hsize=100in" "\n"

ALL_PNG = sorted(p.stem for p in PNG.glob("*.png"))
DAMAGED = {"badidatcrc", "truncated"}
# libpng leaves the padding bits of interlaced rows with < 8 bits/pixel from pdfTeX's
# uninitialized row buffers (malloc garbage): known, unfixable difference
PADDING = {"i7x9pad", "pad_pal1", "pad_gray4gamma"}
GOOD_PNG = [n for n in ALL_PNG if n not in DAMAGED | PADDING]


def doc(settings, body):
    return PRE + settings + "\n" + body + "\n\\bye\n"


def imgs(names, ext=".png", opts=""):
    out = []
    for n in names:
        out.append(r"\pdfximage %s{%s%s}\hbox{\pdfrefximage\pdflastximage}" % (opts, n, ext))
    return "\n".join(out)


def tests():
    t = []
    good = imgs(GOOD_PNG)
    pngfiles = [PNG / (n + ".png") for n in ALL_PNG]
    for v in (3, 4, 5, 7):
        for lvl in (0, 9):
            t.append((f"png_v{v}_c{lvl}", doc(rf"\pdfminorversion={v} \pdfcompresslevel={lvl}", good), pngfiles))
    t.append(("png_v1", doc(r"\pdfminorversion=1", good), pngfiles))
    t.append(("png_v2", doc(r"\pdfminorversion=2", good), pngfiles))
    t.append(("png_objc2", doc(r"\pdfminorversion=5 \pdfobjcompresslevel=2", good), pngfiles))
    t.append(("png_objc2_c0", doc(r"\pdfminorversion=5 \pdfobjcompresslevel=2 \pdfcompresslevel=0", good), pngfiles))
    t.append(("png_hicolor0", doc(r"\pdfminorversion=6 \pdfimagehicolor=0", good), pngfiles))
    for g, ig in ((1000, 2200), (2200, 2200), (1000, 1000), (1800, 2500), (500, 4000)):
        t.append((f"png_gamma_{g}_{ig}",
                  doc(rf"\pdfminorversion=5 \pdfimageapplygamma=1 \pdfgamma={g} \pdfimagegamma={ig}", good), pngfiles))
    t.append(("png_gamma_hicolor0", doc(r"\pdfminorversion=5 \pdfimageapplygamma=1 \pdfgamma=1800 \pdfimagehicolor=0", good), pngfiles))
    t.append(("png_res", doc(r"\pdfimageresolution=150", imgs(["phys", "physunk", "gray8"])), pngfiles))
    t.append(("png_res0", doc(r"\pdfimageresolution=0", imgs(["phys", "physunk", "gray8"])), pngfiles))
    t.append(("png_sizes", doc("", imgs(["rgb8"], opts="width 3cm ") + "\n" + imgs(["rgb8"], opts="height 2cm ")
                               + "\n" + imgs(["rgb8"], opts="width 1cm height 4cm ") + "\n" + imgs(["pal8"], opts="depth 1cm ")),
              pngfiles))
    t.append(("png_colorspace", doc(r"\immediate\pdfobj{[/ICCBased 1 0 R]}\edef\cs{\the\pdflastobj}",
                                    imgs(["rgb8", "pal4", "rga8" if False else "rgba8", "gray8", "ga8", "pal4trns"], opts=r"colorspace \cs\space ")),
              pngfiles))
    t.append(("png_attr", doc("", imgs(["rgb8"], opts="attr {/Interpolate true} "))
              , pngfiles))
    t.append(("png_reuse", doc("", r"\pdfximage{rgb8.png}\edef\a{\the\pdflastximage}"
                                   r"\hbox{\pdfrefximage\a}\hbox{\pdfrefximage\a}\pdfximage{rgb8.png}\hbox{\pdfrefximage\pdflastximage}"
                                   "\n" r"\immediate\pdfximage{gray8.png}\showthe\pdflastximagecolordepth"
                                   r"\pdfximage{rgb16.png}\showthe\pdflastximagecolordepth\showthe\pdflastximagepages"),
              pngfiles))
    t.append(("png_unused", doc("", r"\pdfximage{rgba8.png}\pdfximage{ga16.png}\pdfximage{pal8.png}\hrule"), pngfiles))
    t.append(("png_twopages", doc(r"\pdfminorversion=5", imgs(["rgba8", "ga8"]) + r"\vfill\eject " + imgs(["rgba8", "gray8"])), pngfiles))
    t.append(("png_debugcopy", doc(r"\pdfminorversion=5", good), pngfiles, {"TEXMF_DEBUG_PNG_COPY": "1"}))
    for n in sorted(DAMAGED):
        t.append((f"png_dmg_{n}", doc("", imgs([n])), pngfiles))
    t.append(("png_padding_knowndiff", doc("", imgs(sorted(PADDING))), pngfiles))
    t.append(("png_missing", doc("", imgs(["nonexistent"])), pngfiles))

    # ---------------------------------------------------------------- JPEG
    misc = sorted(MISC.iterdir())
    jgood = sorted(p.name for p in MISC.glob("j_*") if not p.name.startswith("j_bad"))
    mwe_jpg = [MWE / f for f in ("example-image.jpg", "example-image-a.jpg", "example-grid-100x100bp.jpg")]
    jall = "\n".join(r"\pdfximage{%s}\hbox{\pdfrefximage\pdflastximage}" % n for n in jgood + [p.name for p in mwe_jpg])
    for v in (3, 5):
        for lvl in (0, 9):
            t.append((f"jpg_v{v}_c{lvl}", doc(rf"\pdfminorversion={v} \pdfcompresslevel={lvl}", jall), misc + mwe_jpg))
    t.append(("jpg_v2_prog", doc(r"\pdfminorversion=2", r"\pdfximage{j_prog.jpg}\hbox{\pdfrefximage\pdflastximage}"), misc))
    t.append(("jpg_res", doc(r"\pdfimageresolution=200", jall), misc + mwe_jpg))
    t.append(("jpg_colorspace", doc(r"\immediate\pdfobj{/DeviceRGB}",
                                    r"\pdfximage colorspace \the\pdflastobj{j_cmyk.jpg}\hbox{\pdfrefximage\pdflastximage}"), misc))
    t.append(("jpg_depth", doc("", r"\pdfximage{j_sof1_12bit.jpg}\showthe\pdflastximagecolordepth"), misc))
    for n in sorted(p.name for p in MISC.glob("j_bad*")):
        t.append((f"jpg_dmg_{n[6:-4]}", doc("", r"\pdfximage{%s}\hbox{\pdfrefximage\pdflastximage}" % n), misc))

    # ---------------------------------------------------------------- JBIG2
    jb = r"""\pdfximage{jb_seq.jb2}\hbox{\pdfrefximage\pdflastximage}
\pdfximage page 2 {jb_globals.jb2}\hbox{\pdfrefximage\pdflastximage}
\pdfximage page 1 {jb_globals.jb2}\hbox{\pdfrefximage\pdflastximage}\showthe\pdflastximagepages
\pdfximage page 1 {jb_random.jb2}\hbox{\pdfrefximage\pdflastximage}
\pdfximage page 2 {jb_random.jb2}\hbox{\pdfrefximage\pdflastximage}
\pdfximage{jb_noeof.jb2}\hbox{\pdfrefximage\pdflastximage}
\pdfximage{jb_page4.jb2}\hbox{\pdfrefximage\pdflastximage}"""
    for lvl in (0, 9):
        t.append((f"jbig2_c{lvl}", doc(rf"\pdfminorversion=4 \pdfcompresslevel={lvl}", jb), misc))
    t.append(("jbig2_objc2", doc(r"\pdfminorversion=5 \pdfobjcompresslevel=2", jb), misc))
    t.append(("jbig2_v3", doc(r"\pdfminorversion=3", r"\pdfximage{jb_seq.jb2}"), misc))
    t.append(("jbig2_badtype", doc(r"\pdfminorversion=4", r"\pdfximage{jb_badtype.jb2}"), misc))
    t.append(("jbig2_nopage", doc(r"\pdfminorversion=4", r"\pdfximage page 3 {jb_globals.jb2}"), misc))
    t.append(("jbig2_page0", doc(r"\pdfminorversion=4", r"\pdfximage page 0 {jb_globals.jb2}"), misc))

    # ---------------------------------------------------------------- PDF
    pdfs = misc
    def px(spec, name):
        return r"\pdfximage %s{%s}\hbox{\pdfrefximage\pdflastximage}" % (spec, name)
    rich = "\n".join(px(s, "p_rich.pdf") for s in
                     ("", "mediabox ", "cropbox ", "bleedbox ", "trimbox ", "artbox ", "page 2 ", "page 3 ", "page 3 mediabox ",
                      "named {second} ", "named {third} trimbox "))
    others = "\n".join(px("", n) for n in ("p_v16.pdf", "p_inherit.pdf"))
    for lvl in (0, 9):
        for oc in (0, 2):
            t.append((f"pdf_rich_c{lvl}_o{oc}", doc(rf"\pdfminorversion=6 \pdfcompresslevel={lvl} \pdfobjcompresslevel={oc}",
                                                   rich + "\n" + others), pdfs))
    t.append(("pdf_v4_warn", doc(r"\pdfminorversion=4", others), pdfs))
    t.append(("pdf_v4_errlevel1", doc(r"\pdfminorversion=4 \pdfinclusionerrorlevel=1", px("", "p_v16.pdf")), pdfs))
    t.append(("pdf_v4_errlevelm1", doc(r"\pdfminorversion=4 \pdfinclusionerrorlevel=-1", px("", "p_v16.pdf")), pdfs))
    t.append(("pdf_pagebox", doc(r"\pdfminorversion=6 \pdfpagebox=5", rich), pdfs))
    t.append(("pdf_suppress", doc(r"\pdfminorversion=6 \pdfsuppressptexinfo=15 \pdfsuppresswarningpagegroup=1",
                                  rich + px("", "p_v16.pdf")), pdfs))
    t.append(("pdf_suppress2", doc(r"\pdfminorversion=6 \pdfsuppressptexinfo=6 \pdfptexuseunderscore=1", rich), pdfs))
    t.append(("pdf_groups", doc(r"\pdfminorversion=6", px("", "p_rich.pdf") + px("", "p_v16.pdf") + r"\vfill\eject" + px("", "p_v16.pdf")), pdfs))
    t.append(("pdf_reuse", doc(r"\pdfminorversion=6", r"\pdfximage page 2 {p_rich.pdf}\edef\a{\the\pdflastximage}"
                                                     r"\hbox{\pdfrefximage\a}\hbox{\pdfrefximage\a}" + px("page 1 ", "p_rich.pdf")
                                                     + r"\showthe\pdflastximagepages"), pdfs))
    t.append(("pdf_ties", doc(r"\pdfminorversion=6", px("", "p_ties.pdf")), pdfs))
    t.append(("pdf_rc4", doc(r"\pdfminorversion=6", px("", "p_rc4.pdf")), pdfs))
    t.append(("pdf_damaged", doc(r"\pdfminorversion=6", px("", "p_damaged.pdf") + px("page 2 ", "p_damaged2.pdf")), pdfs))
    t.append(("pdf_nopage", doc(r"\pdfminorversion=6", px("page 4 ", "p_rich.pdf")), pdfs))
    t.append(("pdf_baddest", doc(r"\pdfminorversion=6", px("named {nosuch} ", "p_rich.pdf")), pdfs))
    mwe_pdf = "\n".join(px(s, n) for n in ("mwe-example-image.pdf", "mwe-example-image-a.pdf", "mwe-example-grid-100x100bp.pdf")
                        for s in ("", "mediabox "))
    for lvl in (0, 9):
        t.append((f"pdf_mwe_replacefonts_c{lvl}", doc(rf"\pdfminorversion=7 \pdfcompresslevel={lvl}", mwe_pdf), pdfs))
        t.append((f"pdf_mwe_copyfonts_c{lvl}", doc(rf"\pdfminorversion=7 \pdfinclusioncopyfonts=1 \pdfcompresslevel={lvl}", mwe_pdf), pdfs))
    # ---------------------------------------------------------------- real PDFs (texmf-dist sample)
    corpus = [TEXMF / l for l in (CASES / "corpus.txt").read_text().split("\n") if l.strip()]
    for k, f in enumerate(corpus):
        n = f.name
        body = (r"\pdfximage{%s}\hbox{\pdfrefximage\pdflastximage}" % n
                + r"\ifnum\pdflastximagepages>1 \pdfximage page \pdflastximagepages{%s}\hbox{\pdfrefximage\pdflastximage}\fi" % n)
        t.append((f"corpus{k:02d}", doc(r"\pdfminorversion=7 \pdfinclusioncopyfonts=1 \pdfobjcompresslevel=2", body), [f]))
        t.append((f"corpusrf{k:02d}", doc(r"\pdfminorversion=7 \pdfobjcompresslevel=2", body), [f]))
    # PDFs written by pdfTeX itself (object streams, xref streams)
    own = [MISC / f"{d}.pdf" for d in ("png_objc2", "pdf_rich_c9_o2", "jbig2_objc2", "png_v5_c0")]
    own = [f for f in own if f.exists()]
    body = "\n".join(r"\pdfximage{%s}\hbox{\pdfrefximage\pdflastximage}" % f.name for f in own)
    t.append(("pdf_own", doc(r"\pdfminorversion=7", body), own))
    # ---------------------------------------------------------------- images in a format
    ini = (r"\input plain \pdfoutput=1 \pdfminorversion=6" "\n"
           r"\pdfximage{rgb8.png}\xdef\imgA{\the\pdflastximage}"
           r"\pdfximage page 2 {p_rich.pdf}\xdef\imgB{\the\pdflastximage}"
           r"\pdfximage page 2 {jb_globals.jb2}\xdef\imgC{\the\pdflastximage}"
           r"\pdfximage{j_rgb_jfif72.jpg}\xdef\imgD{\the\pdflastximage}"
           r"\pdfximage{ga8.png}\xdef\imgE{\the\pdflastximage}"
           r"\pdfximage{p_v16.pdf}\xdef\imgF{\the\pdflastximage}"
           r"\global\setbox1\hbox{\pdfrefximage\imgA}" "\n"
           r"\dump" "\n")
    (GEN / "imgfmt.ini").write_text(ini, encoding="latin-1")
    body = (r"\nopagenumbers \hbox{\pdfrefximage\imgA}\hbox{\pdfrefximage\imgB}\hbox{\pdfrefximage\imgC}"
            r"\hbox{\pdfrefximage\imgD}\hbox{\pdfrefximage\imgE}\hbox{\pdfrefximage\imgF}\box1"
            r"\pdfximage{gray8.png}\hbox{\pdfrefximage\pdflastximage}\showthe\pdflastximage" "\n"
            r"\bye" "\n")
    for lvl in (0, 9):
        t.append((f"fmt_images_c{lvl}", rf"\pdfcompresslevel={lvl} " + body, pngfiles + misc + [GEN / "imgfmt.ini"], None,
                  [["-ini", "-interaction=batchmode", "-translate-file=cp227.tcx", "imgfmt.ini"],
                   ["-fmt=imgfmt", "-interaction=nonstopmode", f"fmt_images_c{lvl}"]]))
    return t


def run(name, tex, files, env=None, steps=None):
    (GEN / f"{name}.tex").write_text(tex, encoding="latin-1")
    e = dict(ENV)
    e.update(env or {})
    args = [sys.executable, "-I", str(ROOT / "port/tests/difftest.py"), name, "--files", str(GEN / f"{name}.tex")]
    args += [str(f) for f in files]
    if steps is None:
        steps = [["-ini", "-interaction=batchmode", "-translate-file=cp227.tcx", "plain", r"\dump"],
                 ["-fmt=plain", "-interaction=nonstopmode", name]]
    for st in steps:
        args += ["--"] + st
    p = subprocess.run(args, env=e, capture_output=True, text=True, errors="replace")
    out = (p.stdout + p.stderr).strip()
    print(out if out else f"[??] {name}")
    return out.startswith("[SAME]")


def main():
    sel = sys.argv[1:]
    ok = tot = 0
    for case in tests():
        name, tex, files = case[0], case[1], case[2]
        env = case[3] if len(case) > 3 else None
        steps = case[4] if len(case) > 4 else None
        if sel and not any(s in name for s in sel):
            continue
        tot += 1
        ok += run(name, tex, files, env, steps)
    print(f"{ok}/{tot} identical")


if __name__ == "__main__":
    main()
