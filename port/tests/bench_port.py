"""Speed comparison: TeX Live 2026 pdftex vs the Rust port, same inputs.

Each engine uses formats it built itself, except the "TL format" jobs, where both engines
load TeX Live's installed pdflatex.fmt. The Rust engine keeps a cache of decompressed
formats (rt/dump.rs, fmt_cache); the "no fmt cache" jobs turn it off (TEXRS_FMT_CACHE=0)
to show the engine without it. Every job is checked for byte-identical output before it
is timed (format builds: the .log; the .fmt itself is compared, with TeX Live's own
run-to-run noise masked, by tests/difftest.py). Results: bench/results/port.md, port.json.

Usage: python -I port/tests/bench_port.py [runs]   (needs an idle machine; uses hyperfine)
       TEXRS_BENCH_BIN=path/to/pdftex.exe selects the Rust binary (e.g. the PGO build).
"""
import json
import os
import pathlib
import shutil
import statistics
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import difftest  # noqa: E402
import pcore  # noqa: E402

# timings on a hybrid CPU depend on P- vs E-core placement; pin both engines to P-cores
pcore.pin_to_pcores()

ROOT = difftest.ROOT
HYPERFINE = ROOT / "tools" / "bin" / "hyperfine.exe"
CORPUS = ROOT / "bench" / "corpus"
OUT = ROOT / "bench" / "results"
W = difftest.WORK / "_bench"
TL = difftest.TL_BIN / "pdftex.exe"
RUST = pathlib.Path(os.environ.get("TEXRS_BENCH_BIN", difftest.ENGINE_DIR / "target" / "release" / "pdftex.exe"))

ENV = dict(os.environ)
ENV.update({"SOURCE_DATE_EPOCH": "1700000000", "FORCE_SOURCE_DATE": "1",
            "TEXRS_SELFAUTOLOC": difftest.selfautoloc()})
ENV["PATH"] = str(difftest.TL_BIN) + os.pathsep + ENV["PATH"]
NOCACHE = {"TEXRS_FMT_CACHE": "0"}


def sh(exe, args, cwd, extra=None):
    env = dict(ENV, **(extra or {}))
    return subprocess.run([str(exe), *args], cwd=cwd, env=env, capture_output=True, stdin=subprocess.DEVNULL)


PLAIN_INI = ["-ini", "-interaction=batchmode", "-translate-file=cp227.tcx", "plain", "\\dump"]
LATEX_INI = ["-ini", "-jobname=pdflatex", "-progname=pdflatex", "-translate-file=cp227.tcx",
             "-interaction=batchmode", "*pdflatex.ini"]


def prepare():
    if W.exists():
        shutil.rmtree(W)
    big = (HERE / "cases" / "bigplain.tex").read_text(encoding="utf-8")
    body = big.rsplit("\\bye", 1)[0]
    for side, exe in (("c", TL), ("r", RUST)):
        d = W / side
        d.mkdir(parents=True)
        for f in CORPUS.glob("*.tex"):
            shutil.copy2(f, d / f.name)
        for f in ("graphics.tex", "hyperref.tex", "microtype.tex"):
            shutil.copy2(HERE / "cases" / "latex" / f, d / f)
        shutil.copy2(HERE / "cases" / "bigplain.tex", d / "bigplain.tex")
        (d / "big10.tex").write_text(body * 10 + "\\bye\n", encoding="utf-8", newline="\n")
        sh(exe, PLAIN_INI, d)
        sh(exe, LATEX_INI, d)


# TeX Live's installed format (absolute path, so the format in the run directory is not used)
TLFMT = str(difftest.TL_BIN.parents[1] / "texmf-var" / "web2c" / "pdftex" / "pdflatex.fmt").replace("\\", "/")


def latex(n, fmt="pdflatex"):
    return ["-progname=pdflatex", f"-fmt={fmt}", "-interaction=batchmode", n]


# name, args, files whose identity is checked, extra environment for the Rust engine
JOBS = [
    ("plain format build (-ini)", PLAIN_INI[:-2] + ["-jobname=plainb", "plain", "\\dump"], ["plainb.log"], {}),
    ("pdflatex format build (-ini)", ["-ini", "-jobname=latexb", "-progname=pdflatex", "-translate-file=cp227.tcx",
                                      "-interaction=batchmode", "*pdflatex.ini"], ["latexb.log"], {}),
    ("plain 70 pages, DVI", ["-fmt=plain", "-interaction=batchmode", "-output-format=dvi", "bigplain"], ["bigplain.dvi"], {}),
    ("plain 70 pages, PDF", ["-fmt=plain", "-interaction=batchmode", "-output-format=pdf", "bigplain"], ["bigplain.pdf"], {}),
    ("plain 700 pages, PDF", ["-fmt=plain", "-interaction=batchmode", "-output-format=pdf", "big10"], ["big10.pdf"], {}),
] + [
    (f"LaTeX {n}", latex(n), [f"{n}.pdf"], {})
    for n in ("01-latex-hello", "02-article", "03-math", "04-book", "05-tikz", "06-preamble",
              "graphics", "hyperref", "microtype")
] + [
    ("LaTeX 01-latex-hello, no fmt cache", latex("01-latex-hello"), ["01-latex-hello.pdf"], NOCACHE),
    ("LaTeX 04-book, no fmt cache", latex("04-book"), ["04-book.pdf"], NOCACHE),
    ("LaTeX 04-book, TL's installed format", latex("04-book", TLFMT), ["04-book.pdf"], {}),
]


def hyperfine(cmd, cwd, runs, extra, warmup):
    """Wall-clock times (s) of `runs` runs of cmd."""
    out = W / "hf.json"
    args = [str(HYPERFINE), "-N", "-w", str(warmup), "-r", str(runs), "--export-json", str(out), cmd]
    if out.exists():
        out.unlink()
    p = subprocess.run(args, cwd=cwd, env=dict(ENV, **extra), capture_output=True, text=True, errors="replace")
    if p.returncode != 0:
        raise SystemExit(f"hyperfine failed on {cmd}:\n{p.stderr[-2000:]}")
    r = json.loads(out.read_text())["results"][0]
    if "measurements" in r:  # hyperfine 2.x
        return [m["time_wall_clock"]["value"] for m in r["measurements"]]
    return list(r["times"])  # hyperfine 1.x


def stats(ts):
    return {"mean": statistics.mean(ts), "median": statistics.median(ts), "min": min(ts),
            "stddev": statistics.stdev(ts) if len(ts) > 1 else 0.0, "n": len(ts)}


def measure(cmd_tl, cmd_rs, extra, runs, rounds=5):
    """Time both engines in alternating rounds (TL first in even rounds, Rust first in odd
    ones), so slow drift in machine load affects both equally; samples are pooled."""
    per = max(1, runs // rounds)
    tl, rs = [], []
    for k in range(rounds):
        order = [("c", cmd_tl, {}, tl), ("r", cmd_rs, extra, rs)]
        for side, cmd, env, acc in (order if k % 2 == 0 else order[::-1]):
            acc += hyperfine(cmd, W / side, per, env, 2 if k == 0 else 1)
    return stats(tl), stats(rs)


def main():
    runs = int(sys.argv[1]) if len(sys.argv) > 1 else 10
    flt = sys.argv[2] if len(sys.argv) > 2 else ""
    prepare()
    rows = []
    for name, args, outputs, extra in JOBS:
        if flt not in name:
            continue
        for side, exe in (("c", TL), ("r", RUST)):
            sh(exe, args, W / side, extra if side == "r" else None)
            sh(exe, args, W / side, extra if side == "r" else None)  # second pass: .aux etc. settled
        same = all((W / "c" / o).exists() and (W / "c" / o).read_bytes() == (W / "r" / o).read_bytes()
                   for o in outputs)
        # hyperfine -N splits the command with shell-word rules: a bare \dump would lose its
        # backslash, so arguments with backslashes or spaces are single-quoted
        quoted = " ".join(f"'{a}'" if (" " in a or "\\" in a) else a for a in args)
        tl, rs = measure(f'"{TL}" {quoted}', f'"{RUST}" {quoted}', extra, runs)
        rows.append({"job": name, "identical": same, "tl": tl, "rust": rs})
        print(f"{name:40} identical={same}  TL {tl['median']*1000:7.1f} ms  Rust {rs['median']*1000:7.1f} ms"
              f"  speed-up {tl['median']/rs['median']:.2f}x (min {tl['min']/rs['min']:.2f}x)", flush=True)
    OUT.mkdir(exist_ok=True)
    (OUT / "port.json").write_text(json.dumps(rows, indent=1))
    n = rows[0]["tl"]["n"] if rows else runs
    lines = [f"# TeX Live pdftex vs Rust port (ms; {n} runs per engine and job, in alternating rounds)", "",
             f"Rust binary: `{RUST.relative_to(ROOT) if RUST.is_relative_to(ROOT) else RUST}`", "",
             "| job | identical output | TeX Live median (min) | Rust median (min) | speed-up (median) | speed-up (min) |",
             "|---|---|---:|---:|---:|---:|"]
    for r in rows:
        lines.append(f"| {r['job']} | {'yes' if r['identical'] else 'NO'} | "
                     f"{r['tl']['median']*1000:.1f} ({r['tl']['min']*1000:.1f}) | "
                     f"{r['rust']['median']*1000:.1f} ({r['rust']['min']*1000:.1f}) | "
                     f"{r['tl']['median']/r['rust']['median']:.2f}x | {r['tl']['min']/r['rust']['min']:.2f}x |")
    (OUT / "port.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
