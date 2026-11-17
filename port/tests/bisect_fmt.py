"""Find the first line of a .ini/.tex file after which the Rust engine's dumped format
differs from TeX Live's: dumps after the first N lines (binary search on N).

Usage: python -I port/tests/bisect_fmt.py path/to/plain.tex [extra pdftex args...]
"""
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import difftest  # noqa: E402


def same_after(lines, n, extra):
    src = difftest.WORK / "_bisect_src.tex"
    src.parent.mkdir(parents=True, exist_ok=True)
    src.write_text("".join(lines[:n]) + "\n\\dump\n", encoding="latin-1")
    ok, _, _ = difftest.case("bisect", [src], ["-ini", "-interaction=batchmode", *extra, "_bisect_src"], quiet=True)
    return ok


def main():
    path = pathlib.Path(sys.argv[1])
    extra = sys.argv[2:]
    lines = path.read_text(encoding="latin-1").splitlines(keepends=True)
    lo, hi = 0, len(lines)
    if not same_after(lines, lo, extra):
        print("differs with zero lines")
        return
    if same_after(lines, hi, extra):
        print("no difference")
        return
    while hi - lo > 1:
        mid = (lo + hi) // 2
        if same_after(lines, mid, extra):
            lo = mid
        else:
            hi = mid
    print(f"first differing prefix: {hi} lines; line {hi}: {lines[hi - 1]!r}")


if __name__ == "__main__":
    main()
