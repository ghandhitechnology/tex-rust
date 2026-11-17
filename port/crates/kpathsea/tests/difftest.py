"""Differential test: TeX Live's C kpsewhich.exe vs the Rust rkpsewhich.

Run from anywhere with:   python -I tests/difftest.py [-v] [-k SUBSTRING]

The Rust binary (target/release/rkpsewhich.exe) is copied into a scratch
directory as `kpsewhich.exe` (so its program name is identical) and runs
with TEXRS_SELFAUTOLOC pointing at the C binary's directory, so both see
the same SELFAUTOLOC/SELFAUTODIR/SELFAUTOPARENT.

For every query the two programs get identical arguments, environment and
working directory; stdout, stderr and the exit status must be byte-for-byte
identical.  Fixture trees (mixed-case names, spaces, `//`-only files, a
private ls-R, a private texmf.cnf, aliases, ...) are created under
target/difftest/.
"""

import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
ROOT = os.path.abspath(os.path.join(CRATE, "..", "..", ".."))
TLBIN = os.path.join(ROOT, "tools", "texlive", "2026", "bin", "windows")
CKPSE = os.path.join(TLBIN, "kpsewhich.exe")
RUST_SRC = os.path.join(CRATE, "target", "release", "rkpsewhich.exe")
WORK = os.path.join(CRATE, "target", "difftest")
RBIN = os.path.join(WORK, "rbin")
RKPSE = os.path.join(RBIN, "kpsewhich.exe")
FX = os.path.join(WORK, "fx")


def fwd(p):
    return p.replace("\\", "/")


def write(path, data=b"x\n"):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data if isinstance(data, bytes) else data.encode("utf-8"))


def make_fixtures():
    if os.path.isdir(FX):
        shutil.rmtree(FX)
    # Working directory files.
    write(os.path.join(FX, "doc.tex"))
    write(os.path.join(FX, "Upper.TEX"))
    write(os.path.join(FX, "MixedCase.sty"))
    write(os.path.join(FX, "with space.tex"))
    write(os.path.join(FX, "noext"))
    write(os.path.join(FX, "refs.bib"))
    write(os.path.join(FX, "dot.name.tex"))
    write(os.path.join(FX, "plain.cls"))
    write(os.path.join(FX, "local.map"))
    write(os.path.join(FX, "my.cfg"))
    write(os.path.join(FX, "pic.eps"))
    write(os.path.join(FX, "font.tfm"))
    write(os.path.join(FX, "only.tex.tex"))
    write(os.path.join(FX, "sub", "deep", "only.sty"))
    write(os.path.join(FX, "sub", "deep", "deeper", "leaf.tex"))
    write(os.path.join(FX, "sub", "Deep2", "x.cls"))
    write(os.path.join(FX, "sub", "a.b", "c.tex"))
    write(os.path.join(FX, "sub", ".hidden", "h.tex"))
    write(os.path.join(FX, "sub", "sp ace", "s.tex"))
    write(os.path.join(FX, "sub", "dup.tex"))
    write(os.path.join(FX, "sub", "deep", "dup.tex"))
    os.makedirs(os.path.join(FX, "sub", "emptydir"), exist_ok=True)
    os.makedirs(os.path.join(FX, "adir.tex"), exist_ok=True)  # a directory named like a file
    # Non-ASCII names (UTF-8 mode for pdftex-like program names).
    write(os.path.join(FX, "sub", "\u00fcml\u00e4ut.tex"))
    write(os.path.join(FX, "\ud55c\uae00.tex"))
    # A private TDS tree with its own ls-R.
    tree = os.path.join(FX, "tree")
    write(os.path.join(tree, "tex", "latex", "pkg", "inlsr.sty"))
    write(os.path.join(tree, "tex", "latex", "pkg", "CamelPkg.sty"))
    write(os.path.join(tree, "tex", "latex", "pkg", "notinlsr.sty"))
    write(os.path.join(tree, "tex", "latex", "other", "inlsr.sty"))
    write(os.path.join(tree, "tex", "generic", "g", "gen.tex"))
    write(os.path.join(tree, "fonts", "tfm", "vendor", "fx", "fxr.tfm"))
    write(os.path.join(tree, "fonts", "pk", "ljfour", "vendor", "fx", "fxr.600pk"))
    write(os.path.join(tree, "fonts", "pk", "ljfour", "vendor", "fx", "dpi300", "fxr.pk"))
    write(os.path.join(tree, "fonts", "map", "fontname", "texfonts.map"),
          "% test map\nfxr  fxalias   extra words\n fxr AliasUpper\n"
          "include special.map\nbad-line-only\n")
    write(os.path.join(tree, "tex", "latex", "pkg", "realname.sty"))
    write(os.path.join(tree, "tex", ".git", "hidden.sty"))
    lsr = (
        "% ls-R -- filename database for kpathsea; do not change this line.\n"
        "./:\nls-R\naliases\ntex\nfonts\n\n"
        "./tex:\nlatex\ngeneric\n.git\n\n"
        "./tex/latex:\npkg\nother\n\n"
        "./tex/latex/pkg:\ninlsr.sty\ncamelpkg.sty\nghost.sty\nrealname.sty\n.\n..\n\n"
        "./tex/latex/other:\ninlsr.sty\n\n"
        "./tex/.git:\nhidden.sty\n\n"
        "./tex/generic/g:\ngen.tex\n\n"
        "./fonts/tfm/vendor/fx:\nfxr.tfm\n\n"
        "./fonts/pk/ljfour/vendor/fx:\nfxr.600pk\ndpi300\n\n"
        "./fonts/pk/ljfour/vendor/fx/dpi300:\nfxr.pk\n\n"
        "./fonts/map/fontname:\ntexfonts.map\n"
    )
    write(os.path.join(tree, "ls-R"), lsr.replace("\n", "\r\n"))
    write(os.path.join(tree, "aliases"), "% aliases\nrealname.sty aliasname.sty\nrealname.sty  Mixed.sty\n")
    # A private texmf.cnf with odd lines.
    cnf = (
        "% private cnf\n"
        "FOO = foo value   % trailing comment\n"
        "FOO = second definition ignored\n"
        "BAR.kpsewhich = bar-for-kpsewhich\n"
        "BAR = bar-plain\n"
        "BAZ = a#b  #comment\n"
        "CONT = first \\\n"
        "   second\\\n"
        "third\n"
        "EMPTY =\n"
        "= novar\n"
        "QUAL. = x\n"
        "WEIRD.$x = y\n"
        "SELFREF = $SELFREF/x\n"
        "BRACES = {a,b,{c,d}}/e\n"
        "TILDE = ~/foo;~\n"
        "USES = $FOO-$BAR\n"
        "NOEQ value-without-equals\n"
        "openout_any = p\n"
    )
    write(os.path.join(FX, "cnf", "texmf.cnf"), cnf)
    write(os.path.join(FX, "cnf2", "texmf.cnf"), "TEXINPUTS = .;" + fwd(FX) + "/sub//\nLAST = x \\")
    write(os.path.join(FX, "kdot", "doc.tex"))


def prepare_binaries():
    if not os.path.isfile(RUST_SRC):
        sys.exit("build first: cargo build --release (missing %s)" % RUST_SRC)
    os.makedirs(RBIN, exist_ok=True)
    shutil.copyfile(RUST_SRC, RKPSE)


def run(exe, args, env_extra, cwd, rust):
    env = dict(os.environ)
    for k in list(env):
        if k.upper() in ("TEXINPUTS", "TEXMFCNF", "TEXMFDBS", "KPATHSEA_DEBUG", "KPSE_DOT",
                         "TEXRS_SELFAUTOLOC", "TEXMF_OUTPUT_DIRECTORY", "TEXMFOUTPUT"):
            del env[k]
    env.update(env_extra)
    if rust:
        env["TEXRS_SELFAUTOLOC"] = TLBIN
    p = subprocess.run([exe] + args, cwd=cwd, env=env, capture_output=True, timeout=120)
    # getopt messages start with argv[0], which is each binary's own path.
    err = p.stderr.replace(exe.encode("mbcs"), b"<argv0>")
    return p.returncode, p.stdout, err


def build_queries():
    q = []

    def add(args, env=None, cwd=FX, compare_stderr=True):
        q.append((list(args), dict(env or {}), cwd, compare_stderr))

    progs = ["pdftex", "pdflatex", None]
    names = [
        "cmr10.tfm", "cmr10", "CMR10.TFM", "article.cls", "article", "Article.CLS", "plain.tex",
        "plain", "latex.ltx", "pdftex.map", "psfonts.map", "pdflatex.fmt", "pdftex.fmt",
        "tex.fmt", "lmodern.sty", "size10.clo", "t1enc.def", "ot1cmr.fd", "hyphen.cfg",
        "texmf.cnf", "updmap.cfg", "fmtutil.cnf", "pdftex.cfg", "glyphlist.txt", "cm-super-t1.enc",
        "cmr10.pfb", "lmr10.pfb", "8r.enc", "doc.tex", "doc", "Upper.TEX", "upper.tex",
        "MixedCase.sty", "mixedcase.sty", "with space.tex", "with space", "noext", "refs.bib",
        "refs", "dot.name.tex", "dot.name", "only.tex", "local.map", "my.cfg", "pic.eps",
        "font.tfm", "font", "nonexistent.tex", "nonexistent", "./doc.tex", ".\\doc.tex",
        "sub/dup.tex", "sub\\dup.tex", "../fx/doc.tex", "adir.tex", "adir", "only.sty",
        "x.cls", "c.tex", "h.tex", "leaf.tex", "s.tex", "lm-ec.enc", "cmr10.vf", "ptmr8r.tfm",
        "ptmr7t.vf", "utmr8a.pfb", "zpzccmry.tfm", "ec-lmr10.tfm", "omlcmm.fd",
        "graphicx.sty", "amsmath.sty", "hyperref.sty", "inputenc.sty", "fontenc.sty",
        "pdftexconfig.tex", "nul", "texsys.cfg", "cmr10.mf", "cmbase.mf", "plain.mf",
        "cmr10.afm", "ls-R", "dvips.ini", "config.ps", "XDvi", "mktex.cnf", "a.b.c.d",
        "tex", ".tex", "", "$HOME", "~", "${SELFAUTOLOC}/kpsewhich.exe", "lualatex.fmt",
        "luaotfload.sty", "texsys.aux",
    ]
    for prog in progs:
        pa = ["-progname=" + prog, "-engine=pdftex"] if prog else []
        for n in names:
            add(pa + [n])
    # Explicit formats.
    fmt_names = [
        ("tex", "article"), ("tex", "article.cls"), ("tfm", "cmr10"), ("tfm", "cmr10.tfm"),
        ("vf", "ptmr7t"), ("enc", "8r"), ("map", "pdftex"), ("type1 fonts", "cmr10"),
        ("type1", "cmr10"), ("pfb", "lmr10"), ("fmt", "pdflatex"), ("cnf", "texmf"),
        ("ls-R", "ls-R"), ("bib", "refs"), ("bst", "plain"), ("ist", "gind"),
        ("web2c", "fmtutil.cnf"), ("othertext", "foo"), ("otherbin", "foo"),
        ("truetype", "DejaVuSans"), ("opentype", "lmroman10-regular"), ("otf", "lmroman10-regular"),
        ("lua", "luaotfload-main"), ("cmap", "Identity-H"), ("sfd", "Unicode"), ("pdftex config", "pdftex.cfg"),
        ("mf", "cmr10"), ("mfpool", "mf"), ("texpool", "tex"), ("pk", "cmr10"), ("gf", "cmr10"),
        ("bitmap font", "cmr10"), ("graphic/figure", "pic.eps"), ("graphic/figure", "pic"),
        ("mpsupport", "mpost"), ("doc", "kpathsea"), ("source", "latex.ltx"), ("dvips config", "config.ps"),
        ("ovf", "x"), ("ofm", "x"), ("opl", "x"), ("ovp", "x"), ("lig", "x"), ("fea", "x"),
        ("cid", "x"), ("mlbib", "refs"), ("mlbst", "plain"), ("clua", "x"), ("ris", "x"),
        ("bltxml", "x"), ("nosuchformat", "x"), ("TFM", "cmr10"), ("26", "article"),
        ("texmfscripts", "fmtutil.pl"), ("web", "tex"), ("cweb", "x"), ("troff fonts", "x"),
        ("misc fonts", "x"), ("type42", "x"), ("PostScript header", "8r.enc"), ("psheader", "tex.pro"),
        ("afm", "ptmr8a"), ("base", "mf"), ("mem", "mpost"), ("ocp", "x"), ("otp", "x"), ("mft", "plain"),
        ("mp", "plain"),
    ]
    for fmt, n in fmt_names:
        add(["-format=" + fmt, n])
        add(["-progname=pdflatex", "-format=" + fmt, n])
    # -all / -must-exist / -subdir.
    for n in ["texmf.cnf", "article.cls", "dup.tex", "inlsr.sty", "fontenc.sty", "README", "Makefile"]:
        add(["-all", n])
        add(["-all", "-must-exist", n])
        add(["-must-exist", n])
    add(["-subdir=latex/base", "article.cls"])
    add(["-subdir=base", "-subdir=tex/latex/base/", "article.cls"])
    add(["-progname=pdflatex", "-subdir=latex/base", "article.cls"])
    add(["-subdir=nonesuch", "article.cls"])
    # Variables.
    vars_ = [
        "TEXMF", "TEXMFDIST", "TEXMFHOME", "TEXMFVAR", "TEXMFSYSVAR", "TEXMFCONFIG", "TEXMFLOCAL",
        "TEXMFROOT", "TEXMFMAIN", "TEXINPUTS", "TFMFONTS", "VFFONTS", "T1FONTS", "ENCFONTS",
        "TEXFONTMAPS", "TEXFORMATS", "TEXMFCNF", "TEXMFDBS", "SELFAUTOLOC", "SELFAUTODIR",
        "SELFAUTOPARENT", "SELFAUTOGRANDPARENT", "progname", "engine", "HOME", "home", "USERPROFILE",
        "openout_any", "openin_any", "shell_escape", "shell_escape_commands", "command_line_encoding",
        "TEXMFOUTPUT", "TEX_HUSH", "try_std_extension_first", "MISSFONT_LOG", "max_print_line",
        "main_memory", "TEXMFAUXTREES", "nonexistentvar", "texinputs", "TEXINPUTS.pdflatex",
        "MKTEXTFM", "MKTEXPK", "TEXMFLOG", "OSFONTDIR", "VARTEXFONTS", "TEXMFDOTDIR", "PATH",
        "SHELL", "MAKETEX_MODE", "MAKETEX_BASE_DPI", "KPATHSEA_DPI", "TEXMFSCRIPTS", "LUAINPUTS",
        "BIBINPUTS", "TEXMFCACHE", "texmf_casefold_search", "guess_input_kanji_encoding",
    ]
    for v in vars_:
        add(["-var-value=" + v])
        add(["-progname=pdflatex", "-var-value=" + v])
    for v in ["TEXINPUTS", "TEXMF", "TEXMFCNF", "TFMFONTS", "TEXFONTMAPS", "nonexistentvar"]:
        add(["-var-brace-value=" + v])
    for s in ["$TEXMF", "${TEXMF}", "$TEXMFDIST/x", "a$$b", "$", "x$", "${unterminated", "${}",
              "$NONEXISTENT/x", "${NONEXISTENT}y", "$TEXMFHOME", "~/x", "$progname$engine",
              "pre${SELFAUTOLOC}post", "$SELFAUTOLOC\\back\\slash", "$a b", "${TEXMF", "$-"]:
        add(["-expand-var=" + s])
    for s in ["{a,b}", "{a,b}{x,y}", "{a,{b,c}}d", "x{,y}", "{a", "a}b", "$TEXMF", "a;b;{c;d}",
              "{$TEXMFDIST,$TEXMFVAR}/fonts", "!!$TEXMFDIST", "~/{a,b}", "{}", "a,b", "${TEXMF}x",
              "{a,b}${X}"]:
        add(["-expand-braces=" + s])
    for s in ["$TEXMF/fonts/map//", "$TEXMFDIST/tex/latex/base", ".", fwd(FX) + "/sub//",
              fwd(FX) + "/sub//deep", fwd(FX) + "/sub//deeper", "!!$TEXMFDIST/fonts/tfm//",
              "{.,sub}", "c:/", "C:\\Windows\\NonExistent", fwd(FX) + "//", "$TEXMFDIST/fonts/enc//public",
              fwd(FX) + "/sub///deep", "//nosuchhost/share", "sub;;sub"]:
        add(["-expand-path=" + s])
    # -show-path for every format.
    for f in ["gf", "pk", "bitmap font", "tfm", "afm", "base", "bib", "bst", "cnf", "ls-R", "fmt",
              "map", "mem", "mf", "mfpool", "mft", "mp", "mppool", "MetaPost support", "ocp", "ofm",
              "opl", "otp", "ovf", "ovp", "graphic/figure", "tex", "TeX system documentation",
              "texpool", "TeX system sources", "PostScript header", "Troff fonts", "type1 fonts",
              "vf", "dvips config", "ist", "truetype fonts", "type42 fonts", "web2c files",
              "other text files", "other binary files", "misc fonts", "web", "cweb", "enc files",
              "cmap files", "subfont definition files", "opentype fonts", "pdftex config",
              "lig files", "texmfscripts", "lua", "font feature files", "cid maps", "mlbib",
              "mlbst", "clua", "ris", "bltxml", "nosuch", "40", "100"]:
        add(["-show-path=" + f])
    for f in ["tex", "tfm", "fmt", "map", "other text files"]:
        add(["-progname=pdflatex", "-show-path=" + f])
        add(["-progname=pdftex", "-engine=pdftex", "-show-path=" + f])
    # Paths given with -path.
    for p, n in [(".", "doc.tex"), (".", "DOC.TEX"), (fwd(FX) + "/sub//", "only.sty"),
                 (fwd(FX) + "/sub//", "dup.tex"), ("sub;.", "dup.tex"), ("$TEXMFDIST/tex//", "article.cls"),
                 ("{sub,.}", "dup.tex"), (".", "nonexistent"), ("!!$TEXMFDIST/tex//", "article.cls")]:
        add(["-path=" + p, n])
        add(["-all", "-path=" + p, n])
    add(["-path=.", "-format=tex", "doc"])
    # Glyph lookups.
    for args in [["cmr10.600pk"], ["cmr10.pk"], ["-dpi=300", "cmr10.pk"], ["-D", "1200", "cmr10.pk"],
                 ["fxr.600pk"], ["fxr.pk"], ["fxr.300pk"], ["fxr.301pk"], ["fxalias.600pk"],
                 ["-mode=ljfour", "fxr.600pk"], ["cmr10.gf"], ["nonesuch.600pk"], ["-all", "fxr.600pk"],
                 ["-format=pk", "fxr"], ["-format=bitmapfont", "fxr"], ["fxr.598pk"]]:
        tree = fwd(FX) + "/tree"
        add(args, {"TEXMFDBS": tree + ";", "PKFONTS": tree + "/fonts/pk//;", "TEXFONTMAPS": tree + "/fonts/map//;"})
    # Private ls-R tree.
    tree = fwd(FX) + "/tree"
    tenv = {"TEXMFDBS": tree + ";", "TEXINPUTS": tree + "/tex//;"}
    for n in ["inlsr.sty", "INLSR.STY", "camelpkg.sty", "CamelPkg.sty", "notinlsr.sty", "ghost.sty",
              "gen.tex", "hidden.sty", "aliasname.sty", "Mixed.sty", "mixed.sty", "realname.sty",
              "pkg/inlsr.sty", "latex/other/inlsr.sty", "other/inlsr.sty", "tex/latex/pkg/inlsr.sty"]:
        add([n], tenv)
        add(["-must-exist", n], tenv)
        add(["-all", n], tenv)
    for n in ["fxr.tfm", "fxr", "fxalias", "fxalias.tfm", "AliasUpper.tfm", "aliasupper", "bad-line-only"]:
        add([n], {"TEXMFDBS": tree + ";", "TFMFONTS": tree + "/fonts/tfm//;",
                  "TEXFONTMAPS": tree + "/fonts/map//;"})
    add(["-debug=2", "inlsr.sty"], tenv)
    add(["-debug=32", "aliasname.sty"], tenv)
    add(["-debug=32", "pkg/inlsr.sty"], tenv)
    add(["-debug=-1", "camelpkg.sty"], tenv)
    # Environment overrides.
    envs = [
        {"TEXINPUTS": "."},
        {"TEXINPUTS": fwd(FX) + "/sub//;"},
        {"TEXINPUTS": ";" + fwd(FX) + "/sub//"},
        {"TEXINPUTS": fwd(FX) + "/sub;;" + fwd(FX)},
        {"TEXINPUTS.kpsewhich": fwd(FX) + "/sub//"},
        {"TEXINPUTS_kpsewhich": fwd(FX) + "/sub//"},
        {"TEXINPUTS.pdflatex": fwd(FX) + "/sub//"},
        {"TEXINPUTS": ""},
        {"TEXINPUTS": "!!" + fwd(FX) + "/sub//"},
        {"TEXINPUTS": fwd(FX).replace("/", "\\") + "\\sub\\\\"},
        {"KPSE_DOT": fwd(FX) + "/kdot"},
        {"TEXMFCNF": fwd(FX) + "/cnf;"},
        {"TEXMFCNF": fwd(FX) + "/cnf2;"},
        {"TEXMFCNF": fwd(FX) + "/nonexistent"},
        {"TEXMFHOME": fwd(FX) + "/tree"},
        {"TEXMF": fwd(FX) + "/tree"},
        {"try_std_extension_first": "f"},
        {"TEXMFDOTDIR": fwd(FX) + "/sub"},
        {"engine": "pdftex"},
        {"KPATHSEA_DEBUG": "64"},
    ]
    for e in envs:
        for args in [["doc.tex"], ["only.sty"], ["leaf.tex"], ["dot.name"], ["dup.tex"],
                     ["-all", "dup.tex"], ["-var-value=TEXINPUTS"], ["-show-path=tex"],
                     ["-progname=pdflatex", "only.sty"], ["article.cls"]]:
            add(args, e)
    cenv = {"TEXMFCNF": fwd(FX) + "/cnf;"}
    for v in ["FOO", "BAR", "BAZ", "CONT", "EMPTY", "SELFREF", "BRACES", "TILDE", "USES", "NOEQ",
              "QUAL", "WEIRD", "openout_any"]:
        add(["-var-value=" + v], cenv)
        # Brace-expanding SELFREF recurses until the stack overflows in both
        # implementations (exit 127); only the depth reached differs.
        add(["-var-brace-value=" + v], cenv, compare_stderr=(v != "SELFREF"))
    add(["-expand-var=$SELFREF"], cenv)
    add(["-expand-braces=$BRACES"], cenv)
    # -cnf-line.
    for cl in ["TEXINPUTS=" + fwd(FX) + "/sub//", "FOO=bar", "FOO.kpsewhich=prog", "  X = y % c",
               "=bad", "BAD.=x", "WEIRD.a;b=1", "TEXINPUTS.pdflatex=" + fwd(FX) + "/sub//",
               "NOVALUE", "openout_any=a"]:
        add(["-cnf-line=" + cl, "-var-value=TEXINPUTS", "only.sty"])
        add(["-cnf-line=" + cl, "-var-value=FOO"])
        add(["-progname=pdflatex", "-cnf-line=" + cl, "-var-value=TEXINPUTS", "only.sty"])
    # Output name checks.
    for n in ["out.tex", "../out.tex", "sub/../out.tex", "sub/out.tex", ".hidden", "c:/tmp/x.log",
              "C:/tmp/x.log", "/abs/x", "x.exe", "x.EXE", "x.bat.", "X.Com  ", "dir\\x.dll", "x.py",
              "a..b", "a/..b", "..", "..\\x", "$TEXMFVAR/x", fwd(FX) + "/x.log", "~/x", "x.js "]:
        add(["-safe-out-name=" + n])
        add(["-safe-in-name=" + n])
        add(["-safe-extended-out-name=" + n])
        add(["-safe-extended-in-name=" + n])
    for e in [{"openout_any": "a"}, {"openout_any": "r"}, {"TEXMFOUTPUT": fwd(FX)},
              {"TEXMF_OUTPUT_DIRECTORY": fwd(FX)}, {"PATHEXT": ".FOO;.BAR"}]:
        for n in ["x.exe", fwd(FX) + "/x.log", "../x", "x.foo", "c:/tmp/x"]:
            add(["-safe-out-name=" + n], e)
            add(["-safe-extended-out-name=" + n], e)
    # Debug output (stderr compared too).
    for d in ["1", "8", "16", "32", "64", "63", "4"]:
        add(["-debug=" + d, "cmr10.tfm"])
        add(["-debug=" + d, "-progname=pdflatex", "article.cls", "nonexistent.sty"])
    add(["-debug=32", "-must-exist", "nonexistent.sty"])
    add(["-debug=32", "fxr.600pk"], {"TEXMFDBS": fwd(FX) + "/tree;", "PKFONTS": fwd(FX) + "/tree/fonts/pk//;"})
    add(["-debug=16", "-expand-path=" + fwd(FX) + "/sub//"])
    add(["-debug=1", "-path=" + fwd(FX) + "/sub//", "leaf.tex"])
    add(["-debug=40", "-show-path=tex"])
    add(["-help-formats"])
    add(["-progname=pdflatex", "-help-formats"])
    add(["--version"])
    add(["-version"])
    # Option parsing.
    add(["--progname", "pdflatex", "article.cls"])
    add(["-prog=pdflatex", "article.cls"])
    add(["-var=TEXMF"])
    add(["-s=x"])
    add(["-nosuchoption"])
    add(["-all=x", "foo"])
    add(["-format"])
    add(["-format="])
    add([])
    add(["-path=.", "-format=tex", "x"])
    add(["--", "-all"])
    add(["article.cls", "-progname=pdflatex"])
    add(["-mktex=foo", "doc.tex"])
    add(["-no-mktex=tfm", "doc.tex"])
    add(["-interactive", "doc.tex"])
    add(["-casefold-search", "upper.tex"])
    add(["-no-casefold-search", "upper.tex"])
    # Non-ASCII names in UTF-8 mode (pdftex) and ANSI mode.
    for prog in ["pdftex", "kpsewhich"]:
        add(["-progname=" + prog, "\u00fcml\u00e4ut.tex"], {"TEXINPUTS": fwd(FX) + "/sub//"})
        add(["-progname=" + prog, "\ud55c\uae00.tex"])
        add(["-progname=" + prog, "-path=" + fwd(FX) + "/sub", "\u00fcml\u00e4ut.tex"])
    # mktex script invocation (real TeX Live scripts; they fail for these
    # nonexistent fonts and the font-creation command goes to missfont.log
    # in the scratch working directory).
    mk = os.path.join(WORK, "mk")
    os.makedirs(mk, exist_ok=True)
    for args in [["-mktex=tfm", "nosuchfontxyz.tfm"], ["-mktex=pk", "nosuchfontxyz.600pk"],
                 ["-mktex=tfm", "-mktex=pk", "-mode=ljfour", "nosuchfontxyz.300pk"],
                 ["-mktex=tfm", "bad name.tfm"], ["-mktex=tfm", "-mktex=tfm", "-no-mktex=tfm", "nosuchfontxyz.tfm"]]:
        add(args, cwd=mk)
    add(["-mktex=tfm", "nosuchfontxyz.tfm"], {"MISSFONT_LOG": "0"}, cwd=mk)
    add(["-mktex=tfm", "nosuchfontxyz.tfm"], {"TEXMF_OUTPUT_DIRECTORY": fwd(mk)}, cwd=mk)
    # Different working directories.
    for cwd in [os.path.join(FX, "sub"), os.path.join(FX, "sub", "deep"), WORK]:
        for args in [["doc.tex"], ["dup.tex"], ["-all", "dup.tex"], ["../doc.tex"], ["-expand-path=."],
                     ["-path=..", "doc.tex"]]:
            add(args, cwd=cwd)
    return q


def main():
    verbose = "-v" in sys.argv
    filt = None
    if "-k" in sys.argv:
        filt = sys.argv[sys.argv.index("-k") + 1]
    prepare_binaries()
    make_fixtures()
    queries = build_queries()
    if filt:
        queries = [x for x in queries if filt in " ".join(x[0])]
    bad = 0
    for i, (args, env, cwd, cmp_err) in enumerate(queries):
        c = run(CKPSE, args, env, cwd, False)
        r = run(RKPSE, args, env, cwd, True)
        ok = c[0] == r[0] and c[1] == r[1] and (not cmp_err or c[2] == r[2])
        if not ok:
            bad += 1
            print("MISMATCH #%d: args=%r env=%r cwd=%s" % (i, args, env, cwd))
            for label, a, b in (("status", c[0], r[0]), ("stdout", c[1], r[1]), ("stderr", c[2], r[2])):
                if a != b:
                    if isinstance(a, bytes):
                        al = a.split(b"\n")
                        bl = b.split(b"\n")
                        for j in range(max(len(al), len(bl))):
                            x = al[j] if j < len(al) else None
                            y = bl[j] if j < len(bl) else None
                            if x != y:
                                print("  %s line %d:\n    C : %r\n    RS: %r" % (label, j, x, y))
                                break
                    else:
                        print("  %s: C=%r RS=%r" % (label, a, b))
        elif verbose:
            print("ok #%d: %r" % (i, args))
    print("queries: %d, mismatches: %d" % (len(queries), bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
