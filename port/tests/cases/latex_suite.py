"""LaTeX differential suite: build pdflatex.fmt with each engine (compared), then compile
documents twice each (second run reads .aux/.toc) and compare every output byte for byte.

Usage: python -I port/tests/cases/latex_suite.py [name-filter]   (set TEXRS_PROFILE=release)
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import difftest  # noqa: E402

D = HERE / "latex"
D.mkdir(exist_ok=True)
CORPUS = difftest.ROOT / "bench" / "corpus"
MWE = difftest.TL_ROOT / "texmf-dist" / "tex" / "latex" / "mwe"


def src(name, text):
    p = D / name
    p.write_text(text, encoding="utf-8", newline="\n")
    return p


DOCS = {
    "hyperref": src("hyperref.tex", r"""\documentclass{article}
\usepackage[colorlinks]{hyperref}
\title{Links}\author{texrs}
\begin{document}\maketitle\tableofcontents
\section{One}\label{s1} See \ref{s2}, page \pageref{s2}, \url{https://tug.org} and \href{https://example.org}{a link}.
\section{Two}\label{s2} Back to \autoref{s1}.\footnote{A note.}
\subsection{Deeper} Text.
\end{document}
"""),
    "graphics": src("graphics.tex", r"""\documentclass{article}
\usepackage{graphicx}
\begin{document}
\includegraphics[width=3cm]{example-image-a.pdf}\par
\includegraphics[width=3cm]{example-image-b.png}\par
\includegraphics[width=3cm,angle=30]{example-image-c.jpg}\par
\includegraphics[page=2,width=3cm]{example-image-a4-numbered.pdf}
\end{document}
"""),
    "microtype": src("microtype.tex", r"""\documentclass{article}
\usepackage[T1]{fontenc}\usepackage{lmodern}\usepackage{microtype}
\usepackage{lipsum}
\begin{document}
\lipsum[1-6]
\textls{letterspaced text} and \textsc{Small Caps}.
\end{document}
"""),
    "babel": src("babel.tex", r"""\documentclass{article}
\usepackage[T1]{fontenc}\usepackage[ngerman,english]{babel}
\begin{document}
\selectlanguage{ngerman}
Donaudampfschifffahrtsgesellschaftskapitänsmützenabzeichen und weitere sehr lange
Wörter, die getrennt werden müssen, in einer schmalen Spalte.
\parbox{3cm}{Donaudampfschifffahrtsgesellschaftskapitän Rechtsschutzversicherungsgesellschaften}
\selectlanguage{english}\today
\end{document}
"""),
    "tables": src("tables.tex", r"""\documentclass{article}
\usepackage{booktabs,longtable,array,multirow,xcolor,colortbl}
\begin{document}
\begin{tabular}{l>{\raggedleft}p{2cm}c}\toprule
A & B \tabularnewline\midrule \multirow{2}{*}{x} & 1 & \cellcolor{yellow}y\tabularnewline & 2 & z\tabularnewline\bottomrule
\end{tabular}
\begin{longtable}{ll}\caption{Long}\\ \hline
\endhead
""" + "".join(f"row {i} & value {i*i}\\\\\n" for i in range(80)) + r"""\end{longtable}
\end{document}
"""),
    "amsthm": src("amsthm.tex", r"""\documentclass{article}
\usepackage{amsmath,amssymb,amsthm,mathtools,enumitem}
\newtheorem{thm}{Theorem}[section]
\begin{document}
\section{Results}
\begin{thm}\label{t}For all $n$, $\sum_{k=1}^n k = \binom{n+1}{2}$.\end{thm}
\begin{proof}Induction; see \eqref{e}.\end{proof}
\begin{align}\label{e} f(x) &= \int_0^x \frac{\sin t}{t}\,dt \\ g &\coloneqq \sqrt[3]{x}\end{align}
\begin{enumerate}[label=(\alph*)]\item one \item two\end{enumerate}
\end{document}
"""),
    "colors": src("colors.tex", r"""\documentclass{article}
\usepackage[dvipsnames]{xcolor}\usepackage{tikz}
\begin{document}
\textcolor{red}{red} \colorbox{yellow}{box} {\color{OliveGreen} green \newpage still green}
\begin{tikzpicture}\shade[left color=blue,right color=red] (0,0) rectangle (3,1);
\node[draw,fill=Apricot] at (1,2) {Node};\end{tikzpicture}
\end{document}
"""),
}

CORPUS_DOCS = ["01-latex-hello", "02-article", "03-math", "04-book", "05-tikz", "06-preamble"]


def main():
    flt = sys.argv[1] if len(sys.argv) > 1 else ""
    fmt_dir = str(difftest.WORK / "latex-fmt" / "{SIDE}").replace("\\", "/")
    ok = n = 0
    if not flt or flt in "latex-fmt":
        n += 1
        r, _, _ = difftest.case("latex-fmt", [], [["-ini", "-jobname=pdflatex", "-progname=pdflatex",
                                                  "-translate-file=cp227.tcx", "-interaction=nonstopmode",
                                                  "*pdflatex.ini"]])
        ok += r
    run = ["-progname=pdflatex", f"-fmt={fmt_dir}/pdflatex", "-interaction=nonstopmode"]
    items = [(name, [CORPUS / f"{name}.tex"]) for name in CORPUS_DOCS]
    items += [(name, [p] + ([MWE] if name == "graphics" else [])) for name, p in DOCS.items()]
    for name, files in items:
        if flt and flt not in name:
            continue
        stem = files[0].stem
        steps = [run + [stem], run + [stem]]
        if name == "graphics":
            # the mwe images are found through the TeX tree; no copy needed
            files = files[:1]
        n += 1
        r, _, _ = difftest.case("latex-" + name, files, steps)
        ok += r
    print(f"{ok}/{n} identical")


if __name__ == "__main__":
    main()
