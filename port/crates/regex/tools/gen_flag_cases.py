#!/usr/bin/env python3
"""Random cases for the paths pdfTeX never uses (BRE, REG_NEWLINE, REG_NOSUB,
REG_NOTBOL, REG_NOTEOL), checked against the C sources compiled with MSVC
(tools/c_oracle).  Deterministic.

Usage: python -I tools/gen_flag_cases.py <out.tsv>
Output lines: id \\t cflags \\t eflags \\t nmatch \\t pattern_hex \\t subject_hex
"""
import random
import sys

E_ATOMS = [b"a", b"b", b"x", b".", b"[ab]", b"[^a]", b"[^\n]", b"\\w", b"\\W", b"\n", b"A", b"(a|b)", b"(a*)", b"()"]
B_ATOMS = [b"a", b"b", b"x", b".", b"[ab]", b"[^a]", b"\\w", b"\n", b"A", b"*", b"\\(a\\)", b"\\(a*\\)", b"\\(\\)",
           b"a\\{2\\}", b"\\.", b"\\+", b"\\?", b"\\|", b"+", b"?", b"{", b"|"]
ANCHORS = [b"^", b"$", b"\\<", b"\\>", b"\\b", b"\\B", b"\\`", b"\\'"]
E_QUANTS = [b"", b"", b"*", b"+", b"?", b"{2}", b"{0,1}", b"{1,}"]
B_QUANTS = [b"", b"", b"*", b"\\{2\\}", b"\\{0,1\\}", b"\\{1,\\}", b"\\+", b"\\?"]


def rnd(rng, depth, ngroups, ere, allow_bk):
    parts = []
    for _ in range(rng.randint(1, 4)):
        r = rng.random()
        if depth < 2 and r < 0.2:
            ngroups[0] += 1
            inner = rnd(rng, depth + 1, ngroups, ere, allow_bk)
            atom = (b"(" + inner + b")") if ere else (b"\\(" + inner + b"\\)")
        elif allow_bk and ngroups[0] > 0 and r < 0.3:
            atom = b"\\%d" % rng.randint(1, min(ngroups[0], 3))
        elif r < 0.42:
            parts.append(rng.choice(ANCHORS))
            continue
        else:
            atom = rng.choice(E_ATOMS if ere else B_ATOMS)
        parts.append(atom + rng.choice(E_QUANTS if ere else B_QUANTS))
    out = b"".join(parts)
    if ere and rng.random() < 0.2:
        out += b"|" + rnd(rng, depth + 1, ngroups, ere, allow_bk)
    if not ere and rng.random() < 0.15:
        out += b"\\|" + rnd(rng, depth + 1, ngroups, ere, allow_bk)
    return out


def subject(rng):
    n = rng.choice([0, 1, 2, 3, 4, 6, 8])
    return bytes(rng.choice(b"aabbx\nA _") for _ in range(n))


def main():
    rng = random.Random(42)
    out = []
    for i in range(3500):
        ere = rng.random() < 0.5
        bk = rng.random() < 0.25
        while True:
            g = [0]
            pat = rnd(rng, 0, g, ere, bk)
            if len(pat) <= 26:
                break
        cflags = (1 if ere else 0)
        for bit in (2, 4, 8):          # REG_ICASE, REG_NEWLINE, REG_NOSUB
            if rng.random() < (0.15 if bit == 8 else 0.35):
                cflags |= bit
        for j in range(3):
            eflags = rng.choice([0, 0, 1, 2, 3])
            nmatch = rng.choice([0, 1, 2, 4, 10])
            out.append("fl:%d:%d\t%d\t%d\t%d\t%s\t%s" % (i, j, cflags, eflags, nmatch, pat.hex(), subject(rng).hex()))
    with open(sys.argv[1], "w", newline="\n") as f:
        f.write("\n".join(out) + "\n")
    print(len(out))


if __name__ == "__main__":
    main()
