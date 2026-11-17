"""How much of each LaTeX benchmark job is its preamble: time the document, then the same
document with everything between \\begin{document} and \\end{document} removed.

Usage: python -I tests/preamble_share.py [runs]
"""
import os
import pathlib
import re
import statistics
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import difftest  # noqa: E402
import pcore  # noqa: E402

pcore.pin_to_pcores()
ROOT = HERE.parents[1]
EXE = ROOT / "port" / "engine" / "target" / "release" / "pdftex.exe"
D = ROOT / "port" / "test" / "_bvb" / "n"
ENV = dict(os.environ, SOURCE_DATE_EPOCH="1700000000", FORCE_SOURCE_DATE="1",
           TEXRS_SELFAUTOLOC=difftest.selfautoloc())


def t(job, runs):
    args = [str(EXE), "-progname=pdflatex", "-fmt=pdflatex", "-interaction=batchmode", job]
    for _ in range(2):
        subprocess.run(args, cwd=D, env=ENV, capture_output=True)
    ts = []
    for _ in range(runs):
        a = time.perf_counter()
        subprocess.run(args, cwd=D, env=ENV, capture_output=True)
        ts.append(time.perf_counter() - a)
    return statistics.median(ts)


runs = int(sys.argv[1]) if len(sys.argv) > 1 else 5
for name in ("01-latex-hello", "02-article", "03-math", "04-book", "05-tikz", "06-preamble", "graphics", "hyperref",
             "microtype"):
    src = (D / f"{name}.tex").read_text(encoding="utf-8")
    pre = re.sub(r"\\begin\{document\}.*\\end\{document\}", r"\\begin{document}\\end{document}", src, flags=re.S)
    (D / f"{name}-pre.tex").write_text(pre, encoding="utf-8", newline="\n")
    full, p = t(name, runs), t(f"{name}-pre", runs)
    print(f"{name:16} full {full*1000:7.1f} ms   preamble only {p*1000:7.1f} ms   body {(full-p)*1000:7.1f} ms   "
          f"preamble share {100*p/full:4.0f}%", flush=True)
