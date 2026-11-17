"""Core (DVI-mode) differential suite: command-line options, interaction, files, shell escape.
Writes the test sources to tests/cases/core/ and runs each case through difftest.case().

Usage: python -I port/tests/cases/core_suite.py [name-filter]
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import difftest  # noqa: E402

D = HERE / "core"
D.mkdir(exist_ok=True)
FMT = ["-ini", "-interaction=batchmode", "-translate-file=cp227.tcx", "plain", "\\dump"]


def src(name, text):
    p = D / name
    p.write_text(text, encoding="latin-1", newline="\n")
    return p


errors = src("errors.tex", r"""\hsize=3in
\undefinedcs a b c
\hbox to 1pt{too wide content here}
\vbox to 1pt{\hrule height 10pt}
$x^$ and $$ \over \over $$
\endcsname \end{group}
Text \par
\showbox0 \showlists \showthe\count0 \show\par \meaning\hbox
\errmessage{custom error}
\bye
""")

files = src("files.tex", r"""\immediate\openout3=written.tex
\immediate\write3{\relax line one}
\immediate\write3{line two \the\count0}
\immediate\closeout3
\openin5=written.tex
\read5 to\a \show\a
\ifeof5 \message{eof}\else\message{not eof}\fi
\read5 to\b \show\b
\closein5
\openin6=nonexistent \ifeof6 \message{missing ok}\fi
\input written
\write-1{to log only}\write16{to terminal}
\message{\jobname}
\openout4=late \write4{deferred} \closeout4
Text\par
\bye
""")

second = src("second.tex", r"""\message{in second file: \the\inputlineno}
\endinput
not read
""")
inputs = src("inputs.tex", r"""\input second
\input second.tex
\input "second"
\message{\the\inputlineno}
\bye
""")

shell = src("shell.tex", r"""\immediate\write18{echo shell-ran > fromshell.txt}
\input|"echo \string\message{from pipe}"
\message{shell=\the\pdfshellescape}
\bye
""")

terminal = src("terminal.tex", r"""\undefinedcs
\show\par
\read16 to\x \show\x
\bye
""")

srcspec = src("srcspec.tex", r"""\hsize=3in
First paragraph with some words.\par
\hbox{box} Second paragraph $math$ and display $$a=b$$ end.\par
\vbox{v} \bye
""")

mltex = src("mltex.ini", r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
\input plain
\charsubdef 200=`A
\dump
""")

enc = src("enc.ini", r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
\input plain
\mubyte \foo ^^c3^^a9\endmubyte
\dump
""")

stats = src("stats.tex", r"""\tracingonline1 \tracingparagraphs1 \tracingpages1 \tracingmacros2 \tracingcommands3
\tracingrestores1 \tracinggroups1 \tracingifs1 \tracingscantokens1 \tracingnesting2 \tracingassigns1
\hsize=2in \def\a#1{[#1]}\a{x}
\begingroup \count0=5 \endgroup
Some text to break into lines because tracing paragraphs is on and more words follow here.\par
\showgroups \showtokens{abc} \showifs
\scantokens{\message{scanned}}
\bye
""")

CASES = [
    ("errors", [errors], [FMT, ["-fmt=plain", "-interaction=nonstopmode", "errors"]]),
    ("errors-scroll", [errors], [FMT, ["-fmt=plain", "-interaction=scrollmode", "errors"]]),
    ("errors-fle", [errors], [FMT, ["-fmt=plain", "-file-line-error", "-interaction=nonstopmode", "errors"]]),
    ("errors-halt", [errors], [FMT, ["-fmt=plain", "-halt-on-error", "-interaction=nonstopmode", "errors"]]),
    ("files", [files], [FMT, ["-fmt=plain", "-interaction=nonstopmode", "files"]]),
    ("inputs", [inputs, second], [FMT, ["-fmt=plain", "-interaction=nonstopmode", "inputs"]]),
    ("jobname", [inputs, second], [FMT, ["-fmt=plain", "-jobname=other", "-interaction=nonstopmode", "inputs"]]),
    ("recorder", [inputs, second], [FMT, ["-fmt=plain", "-recorder", "-interaction=nonstopmode", "inputs"]]),
    ("cmdline", [second], [FMT, ["-fmt=plain", "-interaction=nonstopmode", "\\message{cmd}\\input second \\bye"]]),
    ("amp-fmt", [second], [FMT, ["&plain", "\\input second \\bye"]]),
    ("shell-off", [shell], [FMT, ["-fmt=plain", "-no-shell-escape", "-interaction=nonstopmode", "shell"]]),
    ("shell-on", [shell], [FMT, ["-fmt=plain", "-shell-escape", "-interaction=nonstopmode", "shell"]]),
    ("shell-restricted", [shell], [FMT, ["-fmt=plain", "-shell-restricted", "-interaction=nonstopmode", "shell"]]),
    ("srcspecials", [srcspec], [FMT, ["-fmt=plain", "-src-specials", "-output-format=dvi", "srcspec"]]),
    ("srcspecials-list", [srcspec], [FMT, ["-fmt=plain", "-src-specials=cr,display,hbox,math,par,parend,vbox", "srcspec"]]),
    ("mltex", [mltex], [["-ini", "-mltex", "-interaction=batchmode", "-translate-file=cp227.tcx", "mltex.ini"]]),
    ("enctex", [enc], [["-ini", "-enc", "-interaction=batchmode", "-translate-file=cp227.tcx", "enc.ini"]]),
    ("etexmode", [srcspec], [["-ini", "-etex", "-interaction=batchmode", "-translate-file=cp227.tcx", "*plain", "\\dump"],
                             ["-fmt=plain", "-interaction=nonstopmode", "srcspec"]]),
    ("stats", [stats], [["-ini", "-etex", "-interaction=batchmode", "-translate-file=cp227.tcx", "*plain", "\\dump"],
                        ["-fmt=plain", "-interaction=nonstopmode", "stats"]]),
    ("8bit", [srcspec], [["-ini", "-8bit", "-interaction=batchmode", "plain", "\\dump"],
                         ["-fmt=plain", "-8bit", "-interaction=nonstopmode", "srcspec"]]),
    ("output-comment", [srcspec], [FMT, ["-fmt=plain", "-output-comment=hello", "-output-format=dvi", "srcspec"]]),
    ("draftmode", [srcspec], [FMT, ["-fmt=plain", "-draftmode", "srcspec"]]),
    ("pdfmatch", [HERE / "core" / "pdfmatch.tex"], [FMT, ["-fmt=plain", "-interaction=nonstopmode", "-output-format=dvi", "pdfmatch"]]),
    ("version", [], [["-version"]]),
    ("help", [], [["-help"]]),
]


def main():
    flt = sys.argv[1] if len(sys.argv) > 1 else ""
    ok = n = 0
    for name, files_, steps in CASES:
        if flt not in name:
            continue
        n += 1
        r, _, _ = difftest.case("core-" + name, files_, steps)
        ok += r
    print(f"{ok}/{n} identical")


if __name__ == "__main__":
    main()
