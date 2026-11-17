"""Run one engine job repeatedly (stdin = NUL, output discarded, like hyperfine -N) until it
exits non-zero; then save the job's log and print its error lines.

Usage: python -I repeat_job.py RUNS DIR EXE ARGS...   (the log is DIR/<last arg>.log)
"""
import os
import pathlib
import shutil
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import difftest  # noqa: E402

runs, cwd, exe, *args = sys.argv[1:]
cwd = pathlib.Path(cwd)
env = dict(os.environ)
env.update({"SOURCE_DATE_EPOCH": "1700000000", "FORCE_SOURCE_DATE": "1",
            "TEXRS_SELFAUTOLOC": difftest.selfautoloc()})
log = cwd / (args[-1] + ".log")
for i in range(int(runs)):
    p = subprocess.run([exe, *args], cwd=cwd, env=env, stdin=subprocess.DEVNULL, capture_output=True)
    if p.returncode != 0:
        print(f"run {i}: exit code {p.returncode}")
        keep = cwd.parent / f"fail-{args[-1]}-{i}.log"
        shutil.copy2(log, keep)
        lines = log.read_text(encoding="latin-1").splitlines()
        for k, l in enumerate(lines):
            if l.startswith("!"):
                print("\n".join(lines[max(0, k - 3):k + 8]))
                break
        print("stderr:", p.stderr[-500:])
        print("saved", keep)
        sys.exit(1)
print(f"{runs} runs, all exit code 0")
