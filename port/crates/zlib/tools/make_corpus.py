"""Build a 10 MB benchmark corpus from a TeX Live tree.

Usage: python -I tools/make_corpus.py <texlive-2026-root> <out-file>

Contents (deterministic): ~4 MB of LaTeX sources (.sty/.cls/.tex/.dtx from
texmf-dist/tex/latex, sorted by path), ~4 MB of the uncompressed
pdflatex.fmt (pdfTeX format memory dump), and ~2 MB of synthetic PDF content
streams, truncated to exactly 10 MiB.
"""
import gzip
import os
import random
import sys

root, out = sys.argv[1], sys.argv[2]
MB = 1024 * 1024
parts = []

src = bytearray()
base = os.path.join(root, "texmf-dist", "tex", "latex")
for dirpath, dirnames, filenames in sorted(os.walk(base)):
    dirnames.sort()
    for f in sorted(filenames):
        if f.endswith((".sty", ".cls", ".tex", ".dtx", ".def")):
            with open(os.path.join(dirpath, f), "rb") as fh:
                src += fh.read()
        if len(src) >= 4 * MB:
            break
    if len(src) >= 4 * MB:
        break
parts.append(bytes(src[: 4 * MB]))

fmt = os.path.join(root, "texmf-var", "web2c", "pdftex", "pdflatex.fmt")
parts.append(gzip.decompress(open(fmt, "rb").read())[: 4 * MB])

rng = random.Random(1)
words = ("the of and to in is that for theorem proof lemma TeX pdfTeX glyph "
         "kerning paragraph section figure").split()
s = []
n = 0
y = 720.0
while n < 2 * MB + 4096:
    t = (f"BT\n/F{rng.randint(1, 12)} {rng.choice([9.9626, 10.9091, 7.97])} Tf\n"
         f"{72 + rng.random() * 50:.3f} {y:.3f} Td\n[("
         + ")-{}(".format(rng.randint(1, 300)).join(rng.choice(words) for _ in range(rng.randint(1, 6)))
         + ")]TJ\nET\n")
    if rng.random() < 0.2:
        t += f"q 1 0 0 1 {rng.randint(0, 500)} {rng.randint(0, 700)} cm\n0 0 m {rng.random() * 300:.2f} 0 l S\nQ\n"
    y = y - 11.955 if y > 80 else 720.0
    s.append(t)
    n += len(t)
parts.append("".join(s).encode()[: 2 * MB])

data = b"".join(parts)[: 10 * MB]
open(out, "wb").write(data)
print(len(data), "bytes written to", out)
