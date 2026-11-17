"""SyncTeX differential suite for the work/synctex engine copy.

python -I tools/suite.py [NAME-SUBSTRING...]

Each case runs difftest (TeX Live pdftex vs Rust, every output file byte for byte) and
then compares the .synctex/.synctex.gz files again after normalizing the c/r test directory
names (Input: lines contain the absolute path of the job directory, so byte equality of
those files is only possible when every input file is given by an absolute path; cases
named *abs* do that and must be byte-identical, .gz included).
"""
import os
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
WORK = HERE.parent
PORT = WORK.parents[1]
if (WORK / "engine").is_dir():  # a working copy of the engine (default: port/engine)
    os.environ.setdefault("TEXRS_ENGINE_DIR", str(WORK / "engine"))
os.environ.setdefault("TEXRS_WORK", str(WORK / "test"))
sys.path.insert(0, str(PORT / "tests"))
sys.path.insert(0, str(HERE))
import difftest  # noqa: E402
import gen_st8  # noqa: E402

C = gen_st8.abs_cases()  # TeX Live kpathsea reports the drive letter in lower case
FMT = ["-ini", "-interaction=batchmode", "-translate-file=cp227.tcx", "plain", "\\dump"]
NS = "-interaction=nonstopmode"


def f(*names):
    return [WORK / "cases" / n for n in names]


CASES = [
    # name, files, steps after the format step
    ("st1", f("st1.tex", "st1b.tex"), [
        ["-fmt=plain", "-synctex=-1", NS, "st1"],
        ["-fmt=plain", "-synctex=1", NS, "-jobname=st1gz", "st1"],
        ["-fmt=plain", "-synctex=-1", "-interaction=batchmode", "-jobname=st1batch", "st1"],
    ]),
    ("st1stale", f("st1.tex", "st1b.tex", "stale/st1.synctex.gz", "stale/st1gz.synctex"), [
        ["-fmt=plain", "-synctex=-1", NS, "st1"],
        ["-fmt=plain", "-synctex=1", NS, "-jobname=st1gz", "st1"],
    ]),
    ("st2", f("st2.tex", "st2b.tex"), [
        ["-fmt=plain", "-synctex=-1", NS, "st2"],
        ["-fmt=plain", "-synctex=1", NS, "-jobname=st2gz", "st2"],
    ]),
    ("st3", f("st3.tex", "st2b.tex"), [["-fmt=plain", "-synctex=-1", NS, "st3"]]),
    ("st4", f("st4.tex", "st2b.tex"), [["-fmt=plain", NS, "st4"]]),
    ("st5", f("st4.tex", "st2b.tex"), [["-fmt=plain", "-synctex=0", NS, "st4"]]),
    ("st6", f("st6.tex", "st2b.tex", "outdir"), [
        ["-fmt=plain", "-synctex=-1", NS, "-output-directory=outdir", "st6"],
        ["-fmt=plain", "-synctex=1", NS, "-output-directory=outdir", "-jobname=st6gz", "st6"],
    ]),
    ("st8abs", [], [
        ["-fmt=plain", "-synctex=1", NS, C + "/st8.tex"],
        ["-fmt=plain", "-synctex=-1", NS, "-jobname=st8p", C + "/st8.tex"],
        ["-fmt=plain", "-synctex=-9", NS, "-jobname=st8h", C + "/st8.tex"],
        ["-fmt=plain", "-synctex=9", NS, "-jobname=st8hgz", C + "/st8.tex"],
    ]),
    ("st9", f("st9.tex", "st9b.tex"), [
        ["-fmt=plain", "-synctex=-1", NS, "st9"],
        ["-fmt=plain", "-synctex=1", NS, "-jobname=st9gz", "st9"],
    ]),
    ("space", f("sp ace.tex"), [
        ["-fmt=plain", "-synctex=-1", NS, '"sp ace.tex"'],
        ["-fmt=plain", "-synctex=1", NS, "-jobname=sp2", '"sp ace.tex"'],
    ]),
    ("st11", f("st11.tex"), [
        ["-fmt=plain", "-synctex=" + o, NS, "-jobname=o" + o.replace("-", "m"), "st11"]
        for o in ["-1", "-3", "-5", "-8", "-13", "-15", "2", "9", "13"]
    ]),
    ("st12", f("st12.tex", "st2b.tex"), [
        ["-fmt=plain", "-synctex=-1", NS, "st12"],
        ["-fmt=plain", "-synctex=1", NS, "-jobname=st12gz", "st12"],
    ]),
    ("kbd", f("st2b.tex"), [
        ["-fmt=plain", "-synctex=-1", NS, "\\shipout\\hbox{x}\\input st2b \\end"],
    ]),
    ("pdf2", f("pdf2.tex", "pdf2b.tex"), [
        ["-fmt=plain", "-synctex=" + o, NS, "-jobname=p" + o.replace("-", "m"), "pdf2"]
        for o in ["-1", "-5", "-13", "1", "5"]
    ]),
    ("pdf3", f("pdf3.tex"), [
        ["-fmt=plain", "-synctex=-1", NS, "pdf3"],
        ["-fmt=plain", "-synctex=-5", NS, "-jobname=pdf3f", "pdf3"],
    ]),
    ("pdf3abs", [], [
        ["-fmt=plain", "-synctex=" + o, NS, "-jobname=a" + o.replace("-", "m"), C + "/pdf3.tex"]
        for o in ["1", "5", "13", "-13", "2"]
    ]),
    ("nonascii", f("한글.tex", "café.tex", "st2b.tex"), [
        ["-fmt=plain", "-synctex=-1", NS, "한글"],
        ["-fmt=plain", "-synctex=1", NS, "-jobname=hgz", "한글"],
        ["-fmt=plain", "-synctex=-1", NS, "café"],
    ]),
    ("nonasciiabs", [], [
        ["-fmt=plain", "-synctex=1", NS, "-jobname=cgz", C + "/café.tex"],
        ["-fmt=plain", "-synctex=-1", NS, "-jobname=cpl", C + "/café.tex"],
        ["-fmt=plain", "-synctex=1", NS, "-jobname=kgz", C + "/가나.tex"],
        ["-fmt=plain", "-synctex=-1", NS, C + "/가나.tex"],
    ]),
    ("pdf4", f("pdf4.tex"), [
        ["-fmt=plain", "-synctex=-1", NS, "pdf4"],
        ["-fmt=plain", "-synctex=1", NS, "-jobname=pdf4gz", "pdf4"],
    ]),
]


def main():
    sel = sys.argv[1:]
    gen_st8.write()  # the *abs cases \input files by absolute path
    results = []
    for name, files, steps in CASES:
        if sel and not any(s in name for s in sel):
            continue
        same, _, _ = difftest.case(name, files, [FMT] + steps)
        p = subprocess.run([sys.executable, "-I", "-X", "utf8", str(HERE / "cmpsync.py"), str(WORK / "test" / name)],
                           capture_output=True, text=True, encoding="utf-8", errors="replace")
        print(p.stdout.rstrip(), p.stderr.rstrip())
        results.append((name, same, "SYNC-OK" in p.stdout))
    print()
    for name, same, sync in results:
        print(f"{name:10s} difftest={'SAME' if same else 'DIFF'}  synctex={'OK' if sync else 'DIFF'}")


if __name__ == "__main__":
    main()
