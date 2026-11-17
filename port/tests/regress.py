"""Run every differential suite and print a summary (set TEXRS_PROFILE=release).

Usage: python -I port/tests/regress.py [suite ...]   suites: misc core trip latex pdf
"""
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import difftest  # noqa: E402

C = HERE / "cases"
CORE = C / "core"
FMT = ["-ini", "-interaction=batchmode", "-translate-file=cp227.tcx", "plain", "\\dump"]
EFMT = ["-ini", "-etex", "-interaction=batchmode", "-translate-file=cp227.tcx", "*plain", "\\dump"]

MISC = [
    ("plainfmt", [], [["-ini", "-interaction=nonstopmode", "-translate-file=cp227.tcx", "plain", "\\dump"]]),
    ("pdfetexfmt", [], [["-ini", "-jobname=pdfetex", "-progname=pdftex", "-translate-file=cp227.tcx",
                         "-interaction=nonstopmode", "-etex", "*pdfetex.ini"]]),
    ("plain1", [C / "plain1.tex"], [FMT, ["-fmt=plain", "-interaction=nonstopmode", "-output-format=dvi", "plain1"]]),
    ("bigplain", [C / "bigplain.tex"], [FMT, ["-fmt=plain", "-interaction=batchmode", "-output-format=dvi", "bigplain"]]),
    ("tlfmt-etex", [CORE / "srcspec.tex"], [["-fmt=etex", "-interaction=nonstopmode", "srcspec"]]),
    ("tlfmt-pdftex-dvi", [CORE / "srcspec.tex"], [["-fmt=pdftex", "-output-format=dvi", "-interaction=nonstopmode", "srcspec"]]),
    ("prims", [CORE / "prims.tex"], [EFMT, ["-fmt=plain", "-interaction=nonstopmode", "-output-format=dvi", "prims"]]),
    ("names", [CORE / "my file.tex", CORE / "tëst.tex", CORE / "names ä.tex"],
     [FMT, ["-fmt=plain", "-interaction=nonstopmode", "names ä"]]),
    ("interact", [CORE / "interact.tex", CORE / "interact.in"], [FMT, ["<interact.in", "-fmt=plain", "interact"]]),
    ("outdir", [CORE / "outdir.tex", CORE / "outsub"],
     [FMT, ["-fmt=plain", "-output-directory=outsub", "-interaction=nonstopmode", "-recorder", "outdir"]]),
    ("pdfnofont", [C / "pdfnofont.tex"], [FMT, ["-fmt=plain", "-interaction=nonstopmode", "pdfnofont"]]),
]


def run_misc():
    ok = 0
    for name, files, steps in MISC:
        r, _, _ = difftest.case("misc-" + name, files, steps)
        ok += r
    return ok, len(MISC)


def run_script(script, *args):
    p = subprocess.run([sys.executable, "-I", str(C / script), *args], capture_output=True, text=True,
                       encoding="utf-8", errors="replace")
    out = p.stdout
    print("\n".join(l for l in out.splitlines() if not l.strip().startswith("(")))
    last = [l for l in out.splitlines() if "identical" in l]
    if last:
        a, b = last[-1].split()[0].split("/")
        return int(a), int(b)
    return 0, 0


def main():
    want = sys.argv[1:] or ["misc", "core", "trip", "latex"]
    total_ok = total = 0
    for s in want:
        print(f"===== {s}")
        if s == "misc":
            a, b = run_misc()
        elif s == "core":
            a, b = run_script("core_suite.py")
        elif s == "trip":
            a, b = run_script("trip_suite.py")
        elif s == "latex":
            a, b = run_script("latex_suite.py")
        else:
            continue
        print(f"----- {s}: {a}/{b} identical")
        total_ok += a
        total += b
    print(f"===== TOTAL {total_ok}/{total} identical")


if __name__ == "__main__":
    main()
