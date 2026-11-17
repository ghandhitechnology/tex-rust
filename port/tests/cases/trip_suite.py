"""Knuth's trip test and e-TeX's etrip test, run differentially (TeX Live pdftex vs Rust).

The stock configurations (triptrap/texmf.cnf, etexdir/etrip/texmf.cnf) give pool_size
40000/32000, too small for pdfTeX's ~36000 characters of built-in strings: TeX Live's pdftex
stops with "You have to increase POOLSIZE". These runs use copies with pool_size = 50000;
everything else is unchanged. The prepared inputs are kept in tripfiles/; they are
rebuilt from the TeX Live sources when those are present (TEXRS_WEB2C: texk/web2c of a
texlive-source checkout at branch2026, default refs/texlive-source/texk/web2c).

Usage: python -I port/tests/cases/trip_suite.py
"""
import os
import pathlib
import shutil
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import difftest  # noqa: E402

SRC = pathlib.Path(os.environ.get("TEXRS_WEB2C", difftest.ROOT / "refs" / "texlive-source" / "texk" / "web2c"))
TRIP = SRC / "triptrap"
ETRIP = SRC / "etexdir" / "etrip"
OUT = HERE / "tripfiles"


def prepare():
    if not (TRIP / "trip.pl").is_file():
        print(f"no TeX Live sources at {SRC}: using the prepared files in {OUT}")
        return
    for sub in ("trip", "etrip", "tripcnf", "etripcnf"):
        (OUT / sub).mkdir(parents=True, exist_ok=True)
    pltotf = difftest.TL_BIN / "pltotf.exe"
    subprocess.run([str(pltotf), str(TRIP / "trip.pl"), str(OUT / "trip" / "trip.tfm")], check=True,
                   capture_output=True)
    subprocess.run([str(pltotf), str(ETRIP / "etrip.pl"), str(OUT / "etrip" / "etrip.tfm")], check=True,
                   capture_output=True)
    for f in ("trip.tex", "trip1.in", "trip2.in"):
        shutil.copy2(TRIP / f, OUT / "trip" / f)
    for f in ("etrip.tex", "etrip1.in", "etrip2.in", "etrip3.in"):
        shutil.copy2(ETRIP / f, OUT / "etrip" / f)
    shutil.copy2(ETRIP / "trip2.in", OUT / "etrip" / "etrip-trip2.in")
    for src, dst in ((TRIP, "tripcnf"), (ETRIP, "etripcnf")):
        text = (src / "texmf.cnf").read_text(encoding="latin-1")
        lines = []
        for line in text.splitlines():
            if line.replace(" ", "").startswith("pool_size="):
                line = "pool_size = 50000"
            lines.append(line)
        (OUT / dst / "texmf.cnf").write_text("\n".join(lines) + "\n", encoding="latin-1")


def cnf(d):
    return str(OUT / d).replace("\\", "/")


def main():
    prepare()
    t, e = OUT / "trip", OUT / "etrip"
    cases = [
        ("trip", [t / "trip.tfm", t / "trip.tex", t / "trip1.in", t / "trip2.in"],
         [["<trip1.in", "--progname=initex", "--ini"], ["<trip2.in", "--progname=tex"]], "tripcnf"),
        # etriptest.test, part 1: compatibility mode
        ("etrip-compat", [t / "trip.tfm", t / "trip.tex", t / "trip1.in", t / "trip2.in"],
         [["<trip1.in", "--progname=einitex", "--ini"], ["<trip2.in", "--progname=etex"]], "etripcnf"),
        # part 2: trip.tex in extended mode
        ("etrip-ext", [t / "trip.tfm", t / "trip.tex", e / "etrip1.in", e / "etrip-trip2.in"],
         [["<etrip1.in", "--progname=einitex", "--ini"], ["<etrip-trip2.in", "--progname=etex"]], "etripcnf"),
        # part 3: etrip proper
        ("etrip", [t / "trip.tfm", e / "etrip.tfm", e / "etrip.tex", e / "etrip2.in", e / "etrip3.in"],
         [["<etrip2.in", "--progname=einitex", "--ini"], ["<etrip3.in", "--progname=etex"]], "etripcnf"),
    ]
    ok = 0
    for name, files, steps, c in cases:
        r, _, _ = difftest.case(name, files, steps, env={"TEXMFCNF": cnf(c)})
        ok += r
    print(f"{ok}/{len(cases)} identical")


if __name__ == "__main__":
    main()
