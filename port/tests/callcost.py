"""Per-call cost of macro_call in each engine: 10^7 calls of small macros (seven nested
ten-fold repetitions), minus the same loop with an empty body; best of N runs.

Usage: python -I tests/callcost.py [runs]
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import microbench as mb  # noqa: E402  (pins to P-cores on import)

PRE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
\def\A#1{#1#1#1#1#1#1#1#1#1#1}
\def\R#1{\def\B{#1}\def\C{\A\B}\def\D{\A\C}\def\E{\A\D}\def\F{\A\E}\def\G{\A\F}\def\H{\A\G}\A\H}
"""
CALLS = 1e7
CASES = {
    "noparam": r"\def\x{}\R{\x}",
    "undelim1": r"\def\x#1{}\R{\x a}",
    "braced1": r"\def\x#1{}\R{\x{a}}",
    "delim1": r"\def\x#1.{}\R{\x a.}",
    "param-use": r"\def\x#1{#1}\R{\x a}",
}


def main():
    runs = int(sys.argv[1]) if len(sys.argv) > 1 else 3
    (mb.D / "c-empty.tex").write_text(PRE + r"\R{}" + "\n\\end\n", encoding="utf-8", newline="\n")
    empty = {e: mb.run(x, "c-empty", runs) for e, x in mb.ENGINES.items()}
    print(f"{'case':12}" + "".join(f"{e:>10}" for e in mb.ENGINES) + "   (ns per call)")
    for name, body in CASES.items():
        (mb.D / f"c-{name}.tex").write_text(PRE + body + "\n\\end\n", encoding="utf-8", newline="\n")
        t = {e: (mb.run(x, f"c-{name}", runs) - empty[e]) * 1e9 / CALLS for e, x in mb.ENGINES.items()}
        print(f"{name:12}" + "".join(f"{t[e]:10.2f}" for e in mb.ENGINES), flush=True)


if __name__ == "__main__":
    main()
