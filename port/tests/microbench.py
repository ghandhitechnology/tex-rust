"""Per-operation cost of TeX's core in each engine: plain TeX documents that repeat one
kind of operation 10^6 times (by nesting 10-fold repetition macros), timed against an
empty run of the same shape. Prints ns per operation for TeX Live, the baseline Rust
binary and the current one.

Usage: python -I tests/microbench.py [runs]
"""
import os
import pathlib
import statistics
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import difftest  # noqa: E402
import pcore  # noqa: E402

pcore.pin_to_pcores()
ROOT = HERE.parents[1]
D = ROOT / "port" / "test" / "_micro"
D.mkdir(parents=True, exist_ok=True)
ENGINES = {
    "TL": difftest.TL_BIN / "pdftex.exe",
    "base": ROOT / "port" / "bench" / "baseline" / "pdftex.exe",
    "now": ROOT / "port" / "engine" / "target" / "release" / "pdftex.exe",
}
ENV = dict(os.environ, SOURCE_DATE_EPOCH="1700000000", FORCE_SOURCE_DATE="1",
           TEXRS_SELFAUTOLOC=difftest.selfautoloc())

# \R{body} runs body 10^6 times: six nested levels of ten-fold repetition
PRE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
\def\A#1{#1#1#1#1#1#1#1#1#1#1}
\def\R#1{\def\B{#1}\def\C{\A\B}\def\D{\A\C}\def\E{\A\D}\def\F{\A\E}\def\G{\A\F}\A\G}
"""
TESTS = {
    "empty": r"\R{}",
    "relax": r"\R{\relax}",
    "macro0": r"\def\x{}\R{\x}",
    "macro1-braced10": r"\def\x#1{}\R{\x{abcdefghij}}",
    "macro2-N": r"\def\x#1#2{}\R{\x ab}",
    "macro-delim": r"\def\x#1\stop{}\R{\x abcde\stop}",
    "param-use": r"\def\x#1{#1#1}\R{\x{\relax}}",
    "expandafter": r"\def\x{}\def\y{}\R{\expandafter\x\y}",
    "ifx": r"\def\x{}\def\y{}\R{\ifx\x\y\fi}",
    "ifx-false-skip": r"\def\x{}\def\y{a}\R{\ifx\x\y abcdefghij\fi}",
    "csname": r"\R{\csname relax\endcsname}",
    "edef": r"\R{\edef\z{abc}}",
    "count-advance": r"\count255=0 \R{\advance\count255 by 1 }",
    "ifnum": r"\R{\ifnum1<2 \fi}",
}


def run(exe, f, runs):
    args = [str(exe), "-ini", "-interaction=batchmode", f]
    for _ in range(2):
        subprocess.run(args, cwd=D, env=ENV, capture_output=True)
    ts = []
    for _ in range(runs):
        a = time.perf_counter()
        subprocess.run(args, cwd=D, env=ENV, capture_output=True)
        ts.append(time.perf_counter() - a)
    return min(ts)


def main():
    runs = int(sys.argv[1]) if len(sys.argv) > 1 else 3
    for name, body in TESTS.items():
        (D / f"{name}.tex").write_text(PRE + body + "\n\\end\n", encoding="utf-8", newline="\n")
    base = {e: run(x, "empty", runs) for e, x in ENGINES.items()}
    print(f"{'operation':18}" + "".join(f"{e:>10}" for e in ENGINES) + "   (ns per operation; now vs base, now vs TL)")
    for name in TESTS:
        if name == "empty":
            continue
        t = {e: (run(x, name, runs) - base[e]) * 1e9 / 1e6 for e, x in ENGINES.items()}
        print(f"{name:18}" + "".join(f"{t[e]:10.1f}" for e in ENGINES)
              + f"   {t['base'] / t['now']:.2f}x  {t['TL'] / t['now']:.2f}x", flush=True)


if __name__ == "__main__":
    main()
