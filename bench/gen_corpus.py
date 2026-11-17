"""Generate the deterministic benchmark corpus in bench/corpus/.

Each document isolates a cost that matters for a faster TeX:
  00-plain-hello   plain TeX, one word: process start + plain format load
  01-latex-hello   LaTeX, one word: latex.ltx format + article class load
  02-article       ~10-page article: text, sections, amsmath, a table, \\label/\\ref
  03-math          49 pages of display math: math-list building dominates
  04-book          128-page book with TOC and cross-references: paragraph/page builder
  05-tikz          60 TikZ pictures: heavy macro expansion (pgf), little text
  06-preamble      a heavy package preamble with a one-line body: package-loading cost
Typst twins (02-article.typ, 04-book.typ) carry the same generated text.

Usage: python -I bench/gen_corpus.py   (rewrites bench/corpus/)
"""
import pathlib

OUT = pathlib.Path(__file__).resolve().parent / "corpus"

WORDS = (
    "the of and a to in is that for it as was with be by on not he this are or his from at "
    "which but have an they you were her she there one all we their been has would when if "
    "will its who more no out so said what up about into than them can only other new some "
    "could these two may first then do any like my now over such our man me even most made "
    "after also did many before must through back years where much your way well down "
    "should because each just those people how too little state good very make world still "
    "own see men work long get here between both life being under never day same another "
    "know while last might us great old year off come since against go came right used take "
    "three typesetting paragraph glue penalty kerning ligature hyphenation baseline "
    "expansion register dimension"
).split()


class Rng:
    """Small LCG so the corpus is byte-identical on every machine and Python version."""

    def __init__(self, seed: int):
        self.s = seed & 0xFFFFFFFF

    def next(self, n: int) -> int:
        self.s = (1103515245 * self.s + 12345) & 0x7FFFFFFF
        return self.s % n


def paragraph(rng: Rng, sentences: int = 6) -> str:
    out = []
    for _ in range(sentences):
        n = 8 + rng.next(14)
        words = [WORDS[rng.next(len(WORDS))] for _ in range(n)]
        words[0] = words[0].capitalize()
        out.append(" ".join(words) + ".")
    return " ".join(out)


# --- LaTeX documents ---------------------------------------------------------

LATEX_EQS = [
    r"\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}",
    r"\sum_{k=1}^{n} k^2 = \frac{n(n+1)(2n+1)}{6}",
    r"\left( \sum_{i=1}^n a_i b_i \right)^2 \le \left(\sum_{i=1}^n a_i^2\right)\left(\sum_{i=1}^n b_i^2\right)",
    r"\mathbf{A} = \begin{pmatrix} a_{11} & a_{12} & a_{13} \\ a_{21} & a_{22} & a_{23} \\ a_{31} & a_{32} & a_{33} \end{pmatrix}",
    r"f(x) = \begin{cases} x^2 \sin\frac{1}{x} & x \ne 0 \\ 0 & x = 0 \end{cases}",
    r"\nabla \times \mathbf{E} = -\frac{\partial \mathbf{B}}{\partial t}",
    r"\zeta(s) = \prod_{p \text{ prime}} \frac{1}{1 - p^{-s}}",
    r"\binom{n}{k} = \frac{n!}{k!\,(n-k)!}",
]

TYPST_EQS = [
    r"integral_0^infinity e^(-x^2) dif x = sqrt(pi) / 2",
    r"sum_(k=1)^n k^2 = (n(n+1)(2n+1)) / 6",
    r"(sum_(i=1)^n a_i b_i)^2 <= (sum_(i=1)^n a_i^2)(sum_(i=1)^n b_i^2)",
    r"bold(A) = mat(a_11, a_12, a_13; a_21, a_22, a_23; a_31, a_32, a_33)",
    r"f(x) = cases(x^2 sin 1/x & x != 0, 0 & x = 0)",
    r"nabla times bold(E) = -(partial bold(B)) / (partial t)",
    r"zeta(s) = product_(p \"prime\") 1 / (1 - p^(-s))",
    r"binom(n, k) = n! / (k! (n-k)!)",
]


def latex_article() -> str:
    rng = Rng(2)
    body = []
    for s in range(1, 9):
        body.append(f"\\section{{Section {s}}}\\label{{sec:{s}}}")
        for p in range(4):
            body.append(paragraph(rng))
            if p == 1:
                eq = LATEX_EQS[(s + p) % len(LATEX_EQS)]
                body.append(f"\\begin{{equation}}\\label{{eq:{s}}}\n{eq}\n\\end{{equation}}")
        body.append(f"As shown in Section~\\ref{{sec:{max(1, s - 1)}}} and Equation~\\eqref{{eq:{s}}}.")
        if s == 4:
            body.append(
                "\\begin{table}[h]\\centering\n\\begin{tabular}{lrrr}\\hline\n"
                "Engine & Run 1 & Run 2 & Run 3 \\\\ \\hline\n"
                + "".join(f"Row {i} & {i*3}.{i} & {i*5}.{i} & {i*7}.{i} \\\\\n" for i in range(1, 9))
                + "\\hline\\end{tabular}\n\\caption{A table.}\\label{tab:1}\\end{table}"
            )
    return (
        "\\documentclass[11pt]{article}\n\\usepackage{amsmath,amssymb}\n"
        "\\title{Benchmark Article}\\author{texrs}\\date{}\n"
        "\\begin{document}\n\\maketitle\n" + "\n\n".join(body) + "\n\\end{document}\n"
    )


def latex_math() -> str:
    rng = Rng(3)
    body = []
    for i in range(400):
        eq = LATEX_EQS[i % len(LATEX_EQS)]
        body.append(f"\\begin{{equation}}\n{eq}\n\\end{{equation}}")
        if i % 4 == 0:
            body.append(
                "\\begin{align}\n"
                f"  a_{{{i}}} &= \\sum_{{j=0}}^{{{i}}} \\frac{{x^j}}{{j!}} + \\sqrt[3]{{1+y_{{{i}}}}} \\\\\n"
                f"  b_{{{i}}} &= \\lim_{{n\\to\\infty}} \\left(1+\\frac{{1}}{{n}}\\right)^{{n}}\n"
                "\\end{align}"
            )
        if i % 10 == 0:
            body.append(paragraph(rng, 3))
    return (
        "\\documentclass[11pt]{article}\n\\usepackage{amsmath,amssymb}\n"
        "\\begin{document}\n" + "\n".join(body) + "\n\\end{document}\n"
    )


def latex_book() -> str:
    rng = Rng(4)
    body = []
    for c in range(1, 21):
        body.append(f"\\chapter{{Chapter {c}}}\\label{{ch:{c}}}")
        for s in range(1, 6):
            body.append(f"\\section{{Section {c}.{s}}}\\label{{sec:{c}.{s}}}")
            for _ in range(5):
                body.append(paragraph(rng, 7))
            body.append(f"See Chapter~\\ref{{ch:{max(1, c - 1)}}}, page~\\pageref{{sec:{c}.{s}}}.")
    return (
        "\\documentclass[11pt]{book}\n\\usepackage{amsmath}\n"
        "\\begin{document}\n\\tableofcontents\n" + "\n\n".join(body) + "\n\\end{document}\n"
    )


def latex_tikz() -> str:
    pics = []
    for i in range(60):
        k = 3 + i % 7
        pics.append(
            "\\begin{tikzpicture}[scale=0.6]\n"
            "\\draw[step=0.5,gray!30,very thin] (-2,-2) grid (2,2);\n"
            "\\draw[->] (-2.2,0) -- (2.2,0) node[right] {$x$};\n"
            "\\draw[->] (0,-2.2) -- (0,2.2) node[above] {$y$};\n"
            f"\\draw[domain=-2:2,smooth,variable=\\x,blue,thick] plot ({{\\x}},{{sin(\\x*{60 + i} r)*1.5}});\n"
            f"\\foreach \\a in {{1,...,{k}}} \\node[circle,draw,fill=red!20,inner sep=1pt] "
            f"at ({{360/{k}*\\a}}:1.5) {{\\a}};\n"
            "\\end{tikzpicture}"
        )
    return (
        "\\documentclass{article}\n\\usepackage{tikz}\n"
        "\\begin{document}\n" + "\n\\par\\medskip\n".join(pics) + "\n\\end{document}\n"
    )


def latex_preamble() -> str:
    return (
        "\\documentclass{article}\n"
        "\\usepackage{amsmath,amssymb,amsthm,mathtools}\n"
        "\\usepackage{graphicx,xcolor,booktabs,array,longtable,multirow}\n"
        "\\usepackage{tikz,pgfplots}\\pgfplotsset{compat=newest}\n"
        "\\usepackage{siunitx,enumitem,geometry,caption,subcaption,float}\n"
        "\\usepackage{microtype}\n"
        "\\usepackage{hyperref,cleveref}\n"
        "\\begin{document}\nHello \\SI{3}{\\metre}.\n\\end{document}\n"
    )


# --- Typst twins -------------------------------------------------------------


def typst_article() -> str:
    rng = Rng(2)
    body = [
        '#set page(paper: "us-letter", margin: 1.5in)\n#set text(size: 11pt)\n'
        "#set heading(numbering: \"1\")\n#set math.equation(numbering: \"(1)\")\n"
        '#align(center)[#text(size: 17pt)[Benchmark Article] \\ texrs]\n'
    ]
    for s in range(1, 9):
        body.append(f"= Section {s} <sec{s}>")
        for p in range(4):
            body.append(paragraph(rng))
            if p == 1:
                body.append(f"$ {TYPST_EQS[(s + p) % len(TYPST_EQS)]} $ <eq{s}>")
        body.append(f"As shown in @sec{max(1, s - 1)} and @eq{s}.")
        if s == 4:
            cells = ", ".join(f"[Row {i}], [{i*3}.{i}], [{i*5}.{i}], [{i*7}.{i}]" for i in range(1, 9))
            body.append(
                "#figure(table(columns: 4, [Engine], [Run 1], [Run 2], [Run 3], "
                + cells
                + "), caption: [A table.]) <tab1>"
            )
    return "\n\n".join(body) + "\n"


def typst_book() -> str:
    rng = Rng(4)
    body = [
        '#set page(paper: "us-letter", margin: 1.5in, numbering: "1")\n#set text(size: 11pt)\n'
        '#set heading(numbering: "1.1")\n'
        "#show heading.where(level: 1): it => { pagebreak(weak: true); it }\n"
        "#outline()\n"
    ]
    for c in range(1, 21):
        body.append(f"= Chapter {c} <ch{c}>")
        for s in range(1, 6):
            body.append(f"== Section {c}.{s} <sec{c}-{s}>")
            for _ in range(5):
                body.append(paragraph(rng, 7))
            body.append(f"See @ch{max(1, c - 1)}, page #context counter(page).at(<sec{c}-{s}>).first().")
    return "\n\n".join(body) + "\n"


DOCS = {
    "00-plain-hello.tex": "Hello\n\\bye\n",
    "01-latex-hello.tex": "\\documentclass{article}\n\\begin{document}\nHello\n\\end{document}\n",
    "02-article.tex": latex_article(),
    "03-math.tex": latex_math(),
    "04-book.tex": latex_book(),
    "05-tikz.tex": latex_tikz(),
    "06-preamble.tex": latex_preamble(),
    "02-article.typ": typst_article(),
    "04-book.typ": typst_book(),
}


def main() -> None:
    OUT.mkdir(exist_ok=True)
    for name, text in DOCS.items():
        (OUT / name).write_text(text, encoding="utf-8", newline="\n")
        print(f"{name:20} {len(text):>8} bytes")


if __name__ == "__main__":
    main()
