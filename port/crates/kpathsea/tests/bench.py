"""Start-up + lookup timing: C kpsewhich.exe vs Rust rkpsewhich.

python -I tests/bench.py [N]

Each binary is run N times (default 40) per scenario through
CreateProcess (subprocess); the minimum and median wall times are
reported.  The Rust binary runs as `kpsewhich.exe` copied next to the
difftest scratch dir with TEXRS_SELFAUTOLOC pointing at the TL bin dir.
"""

import os
import shutil
import statistics
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
ROOT = os.path.abspath(os.path.join(CRATE, "..", "..", ".."))
TLBIN = os.path.join(ROOT, "tools", "texlive", "2026", "bin", "windows")
CKPSE = os.path.join(TLBIN, "kpsewhich.exe")
RUST_SRC = os.path.join(CRATE, "target", "release", "rkpsewhich.exe")
RBIN = os.path.join(CRATE, "target", "difftest", "rbin")
RKPSE = os.path.join(RBIN, "kpsewhich.exe")

# The files a real `pdflatex -recorder` run of bench/corpus/02-article.tex
# reads from the TeX tree (texmf.cnf files and the document excluded).
ARTICLE_FILES = (
    "texfonts.map cmex7.tfm cmex8.tfm msam10.tfm msam7.tfm msbm10.tfm msbm7.tfm cmbx10.tfm "
    "cmbx12.tfm cmbx6.tfm cmbx8.tfm cmex10.tfm cmmi10.tfm cmmi6.tfm cmmi8.tfm cmr10.tfm cmr12.tfm "
    "cmr17.tfm cmr6.tfm cmr8.tfm cmsy10.tfm cmsy6.tfm cmsy8.tfm cmbx10.pfb cmbx12.pfb cmex10.pfb "
    "cmmi10.pfb cmmi8.pfb cmr10.pfb cmr12.pfb cmr17.pfb cmr6.pfb cmr8.pfb cmsy10.pfb cmsy8.pfb "
    "amsfonts.sty amssymb.sty umsa.fd umsb.fd amsbsy.sty amsgen.sty amsmath.sty amsopn.sty "
    "amstext.sty article.cls size11.clo l3backend-pdftex.def pdftex.map pdflatex.fmt"
).split()

SCENARIOS = [
    ("--version", ["--version"]),
    ("-var-value=TEXMF", ["-var-value=TEXMF"]),
    ("4 files (pdflatex)", ["-progname=pdflatex", "-engine=pdftex", "cmr10.tfm", "article.cls",
                            "pdftex.map", "pdflatex.fmt"]),
    ("missing file", ["-progname=pdflatex", "nonexistent-file.sty"]),
    ("49 files of a run", ["-progname=pdflatex", "-engine=pdftex"] + ARTICLE_FILES),
]


def bench(exe, args, n, rust):
    env = dict(os.environ)
    if rust:
        env["TEXRS_SELFAUTOLOC"] = TLBIN
    times = []
    out = None
    for i in range(n + 3):
        t0 = time.perf_counter()
        p = subprocess.run([exe] + args, env=env, capture_output=True)
        t1 = time.perf_counter()
        if i >= 3:
            times.append((t1 - t0) * 1000.0)
        out = p.stdout
    return min(times), statistics.median(times), out


def main():
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 40
    os.makedirs(RBIN, exist_ok=True)
    shutil.copyfile(RUST_SRC, RKPSE)
    print("%-22s %22s %22s" % ("scenario", "C kpsewhich min/med ms", "Rust min/med ms"))
    for label, args in SCENARIOS:
        cmin, cmed, cout = bench(CKPSE, args, n, False)
        rmin, rmed, rout = bench(RKPSE, args, n, True)
        same = "same output" if cout == rout else "OUTPUT DIFFERS"
        print("%-22s %10.1f / %-10.1f %10.1f / %-10.1f %s" % (label, cmin, cmed, rmin, rmed, same))


if __name__ == "__main__":
    main()
