"""Interleaved A/B timing of two engines on one job (alternating runs cancel out drift in
machine load). Each engine runs in its own copy of the job directory.

Usage: python -I ab_time.py RUNS DIR_A EXE_A DIR_B EXE_B -- ARGS...
"""
import os
import pathlib
import statistics
import subprocess
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import difftest  # noqa: E402
import pcore  # noqa: E402

pcore.pin_to_pcores()

argv = sys.argv[1:]
sep = argv.index("--")
runs, da, ea, db, eb = argv[:sep]
args = argv[sep + 1:]
env = dict(os.environ)
env.update({"SOURCE_DATE_EPOCH": "1700000000", "FORCE_SOURCE_DATE": "1",
            "TEXRS_SELFAUTOLOC": difftest.selfautoloc()})
times = {"A": [], "B": []}
for i in range(int(runs)):
    for side, d, e in (("A", da, ea), ("B", db, eb)) if i % 2 == 0 else (("B", db, eb), ("A", da, ea)):
        t = time.perf_counter()
        p = subprocess.run([e, *args], cwd=d, env=env, stdin=subprocess.DEVNULL, capture_output=True)
        times[side].append(time.perf_counter() - t)
        if p.returncode != 0:
            print(f"{side} exit code {p.returncode}")
for side in "AB":
    ts = sorted(times[side])
    print(f"{side}: min {ts[0]*1000:.0f} ms  median {statistics.median(ts)*1000:.0f} ms  max {ts[-1]*1000:.0f} ms")
a, b = statistics.median(times["A"]), statistics.median(times["B"])
print(f"median A/B = {a / b:.3f}  (min A/B = {min(times['A']) / min(times['B']):.3f})")
