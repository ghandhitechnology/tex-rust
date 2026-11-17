"""Generate bigplain.tex: ~150 pages of plain TeX text with math, used for speed and parity."""
import pathlib

WORDS = ("the of and a to in is that for it as was with be by on not he this are or his from at "
         "which but have an they you were her she there one all we their been has would when if "
         "typesetting paragraph glue penalty kerning ligature hyphenation baseline expansion "
         "register dimension difficult efficient office affluent fjord waffle").split()


class Rng:
    def __init__(self, s):
        self.s = s

    def next(self, n):
        self.s = (1103515245 * self.s + 12345) & 0x7FFFFFFF
        return self.s % n


r = Rng(7)
out = ["\\hsize=5in \\vsize=8in \\parindent=1.5em \\baselineskip=13pt \\tolerance=500 \\hbadness=10000"]
for sec in range(1, 61):
    out.append(f"\\bigskip\\noindent{{\\bf Section {sec}.}}\\par")
    for p in range(8):
        ws = []
        for s in range(7):
            n = 6 + r.next(12)
            sent = " ".join(WORDS[r.next(len(WORDS))] for _ in range(n))
            ws.append(sent[0].upper() + sent[1:] + ".")
        if p % 3 == 1:
            ws.insert(3, f"$a_{{{p}}}^2+b^{{{sec}}}=\\sqrt{{c_{{{p}}}}}$")
        out.append(" ".join(ws) + "\\par")
    out.append("$$\\sum_{k=1}^{n} k^2 = {n(n+1)(2n+1)\\over 6}$$")
out.append("\\bye")
pathlib.Path(__file__).with_name("bigplain.tex").write_text("\n".join(out) + "\n")
