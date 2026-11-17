"""Per-call vs per-token cost of macro calls: \\x{<n tokens>} for several n, 10^5 calls
each (five nested ten-fold repetitions), in TeX Live, the baseline and the current engine.
A least-squares line through (n, ns per call) gives the fixed cost and the cost per token.

Usage: python -I tests/slope.py [runs]
"""
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import microbench as mb  # noqa: E402  (pins to P-cores on import)

PRE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
\def\A#1{#1#1#1#1#1#1#1#1#1#1}
\def\R#1{\def\B{#1}\def\C{\A\B}\def\D{\A\C}\def\E{\A\D}\def\F{\A\E}\def\G{\A\F}\A\G}
"""
CALLS = 1e6
CASES = {
    "braced": lambda n: r"\def\x#1{}\R{\x{" + "a" * n + "}}",
    "delim": lambda n: r"\def\x#1\stop{}\R{\x " + "a" * n + r"\stop}",
    "body": lambda n: r"\def\x{" + "a" * 0 + r"\relax" * n + r"}\R{\x}",
    "param-use": lambda n: r"\def\x#1{" + "#1" * n + r"}\R{\x{\relax}}",
}
NS = [1, 10, 50, 200]


def fit(xs, ys):
    n = len(xs)
    mx, my = sum(xs) / n, sum(ys) / n
    b = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sum((x - mx) ** 2 for x in xs)
    return my - b * mx, b


def main():
    runs = int(sys.argv[1]) if len(sys.argv) > 1 else 5
    (mb.D / "s-empty.tex").write_text(PRE + r"\R{}" + "\n\\end\n", encoding="utf-8", newline="\n")
    empty = {e: mb.run(x, "s-empty", runs) for e, x in mb.ENGINES.items()}
    for case, gen in CASES.items():
        res = {e: [] for e in mb.ENGINES}
        for n in NS:
            name = f"s-{case}-{n}"
            (mb.D / f"{name}.tex").write_text(PRE + gen(n) + "\n\\end\n", encoding="utf-8", newline="\n")
            for e, x in mb.ENGINES.items():
                res[e].append((mb.run(x, name, runs) - empty[e]) * 1e9 / CALLS)
        line = f"{case:10}"
        for e in mb.ENGINES:
            a, b = fit(NS, res[e])
            line += f"  {e}: {a:6.1f} ns/call + {b:5.2f} ns/token"
        print(line, flush=True)
        print("            " + "  ".join(f"{e}: " + " ".join(f"{v:7.1f}" for v in res[e]) for e in mb.ENGINES))


if __name__ == "__main__":
    main()
