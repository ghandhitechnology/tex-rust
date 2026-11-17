"""Speed of the current Rust engine against a saved baseline Rust binary (bench/baseline),
on the bench_port.py jobs: both run in their own copy of the job directory, interleaved
(alternating order). Prints median times and the speed-up for each job.

Usage: python -I tests/bench_vs_base.py [runs] [name-filter]
"""
import os
import pathlib
import shutil
import statistics
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import bench_port as bp  # noqa: E402  (pins to P-cores on import)

BASE = bp.ROOT / "port" / "bench" / "baseline" / "pdftex.exe"
NEW = bp.RUST
W = bp.difftest.WORK / "_bvb"


def prepare():
    if W.exists():
        shutil.rmtree(W)
    big = (HERE / "cases" / "bigplain.tex").read_text(encoding="utf-8")
    body = big.rsplit("\\bye", 1)[0]
    for side, exe in (("b", BASE), ("n", NEW)):
        d = W / side
        d.mkdir(parents=True)
        for f in bp.CORPUS.glob("*.tex"):
            shutil.copy2(f, d / f.name)
        for f in ("graphics.tex", "hyperref.tex", "microtype.tex"):
            shutil.copy2(HERE / "cases" / "latex" / f, d / f)
        shutil.copy2(HERE / "cases" / "bigplain.tex", d / "bigplain.tex")
        (d / "big10.tex").write_text(body * 10 + "\\bye\n", encoding="utf-8", newline="\n")
        bp.sh(exe, bp.PLAIN_INI, d)
        bp.sh(exe, bp.LATEX_INI, d)


def run(exe, args, cwd, extra):
    env = dict(bp.ENV, **extra)
    t = time.perf_counter()
    subprocess.run([str(exe), *args], cwd=cwd, env=env, capture_output=True, stdin=subprocess.DEVNULL)
    return time.perf_counter() - t


def main():
    runs = int(sys.argv[1]) if len(sys.argv) > 1 else 5
    flt = sys.argv[2] if len(sys.argv) > 2 else ""
    prepare()
    for name, args, outputs, extra in bp.JOBS:
        if flt not in name:
            continue
        for side, exe in (("b", BASE), ("n", NEW)):
            run(exe, args, W / side, extra)
            run(exe, args, W / side, extra)
        same = all((W / "b" / o).read_bytes() == (W / "n" / o).read_bytes() for o in outputs
                   if (W / "b" / o).exists())
        tb, tn = [], []
        for i in range(runs):
            order = [("b", BASE, tb), ("n", NEW, tn)]
            for side, exe, acc in (order if i % 2 == 0 else order[::-1]):
                acc.append(run(exe, args, W / side, extra))
        mb, mn = statistics.median(tb), statistics.median(tn)
        print(f"{name:40} same={same}  base {mb*1000:7.1f} ms  new {mn*1000:7.1f} ms  speed-up {mb/mn:.2f}x", flush=True)


if __name__ == "__main__":
    main()
