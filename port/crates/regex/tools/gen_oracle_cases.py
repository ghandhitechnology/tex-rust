#!/usr/bin/env python3
"""Generate the inputs for the pdfTeX oracle (tools/oracle.py).

Usage: python -I tools/gen_oracle_cases.py <suite_cases.tsv> <out_main.tsv> <out_backref.tsv>

All cases are ERE (pdfTeX always passes REG_EXTENDED), optionally with
REG_ICASE, eflags 0, and a \\pdfmatch subcount (nmatch).  NUL bytes are never
generated (pdfTeX passes C strings).  The generator is deterministic.

Groups (id prefixes):
  s:<suite id>      every suite pattern/subject (BRE ones too, as ERE)
  s1:/s2:/s0:       the same with subcount 1 / 2 / 0
  si:               suite cases with REG_ICASE toggled
  cls:              each [:class:], its negation, \\w \\W \\s \\S . \\< against every byte
  ic:               REG_ICASE case folding for every byte
  err:              patterns aimed at every regcomp error path
  fz:               random EREs without back-references
  bk:               random EREs with back-references (some hang the C engine;
                    written to a separate file so the oracle runs them in small chunks)
  long:             long subjects (buffer growth paths, icase + back-references)
"""
import random
import sys

main_cases = []
bk_cases = []
seen = set()


def add(lst, cid, icase, sub, pat, subj):
    if b"\0" in pat or b"\0" in subj:
        return
    key = (icase, sub, pat, subj)
    if key in seen:
        return
    seen.add(key)
    lst.append("%s\t%d\t%d\t%s\t%s" % (cid, icase, sub, pat.hex(), subj.hex()))


def suite(path):
    for line in open(path):
        if line.startswith("#") or not line.strip():
            continue
        f = line.rstrip("\n").split("\t")
        sid, cflags = f[0], int(f[1])
        pat, subj = bytes.fromhex(f[4]), bytes.fromhex(f[5])
        pat = pat.split(b"\0")[0]
        subj = subj.split(b"\0")[0]
        icase = 1 if cflags & 2 else 0
        has_bk = any(b"\\%d" % d in pat for d in range(1, 10))
        lst = bk_cases if has_bk else main_cases
        add(lst, "s:" + sid, icase, -1, pat, subj)
        if b"(" in pat:
            add(lst, "s1:" + sid, icase, 1, pat, subj)
            add(lst, "s2:" + sid, icase, 2, pat, subj)
        add(lst, "s0:" + sid, icase, 0, pat, subj)
        add(lst, "si:" + sid, 1 - icase, -1, pat, subj)


CLASSES = [b"alpha", b"upper", b"lower", b"digit", b"xdigit", b"space", b"print", b"punct",
           b"graph", b"cntrl", b"blank", b"alnum"]


def classes():
    for c in CLASSES:
        for neg in (b"", b"^"):
            for icase in (0, 1):
                if icase and c not in (b"upper", b"lower", b"alpha"):
                    continue
                pat = b"[" + neg + b"[:" + c + b":]]"
                for b in range(1, 256):
                    add(main_cases, "cls:%s%s%s:%d" % (neg.decode(), c.decode(), "/i" if icase else "", b), icase, -1, pat, bytes([b]))
    for name, pat in ((b"w", b"\\w"), (b"W", b"\\W"), (b"s", b"\\s"), (b"S", b"\\S"), (b"dot", b".")):
        for b in range(1, 256):
            add(main_cases, "cls:%s:%d" % (name.decode(), b), 0, -1, pat, bytes([b]))
    # word context of each byte: \<a matches "Xa" iff X is not a word char
    for b in range(1, 256):
        add(main_cases, "cls:wordctx:%d" % b, 0, -1, b"\\<a", bytes([b]) + b"a")
        add(main_cases, "cls:wordend:%d" % b, 0, -1, b"a\\>", b"a" + bytes([b]))


def icase_fold():
    for b in range(1, 256):
        if b in b"\\[]().*+?{}|^$":
            lit = b"\\" + bytes([b])
        else:
            lit = bytes([b])
        for other in sorted({b, b ^ 0x20}):
            if other == 0:
                continue
            add(main_cases, "ic:lit:%d:%d" % (b, other), 1, -1, lit, bytes([other]))
            add(main_cases, "ic:br:%d:%d" % (b, other), 1, -1, b"[" + bytes([b]) + b"]" if b not in b"]^-[\\" else b"[\\" + bytes([b]) + b"]", bytes([other]))
            add(main_cases, "ic:nc:%d:%d" % (b, other), 0, -1, lit, bytes([other]))
    # escaped letters under REG_ICASE never match (raw case vs upper-cased input)
    for b in b"acdefghijklmnopqrtuvxyzACDEFGHIJKLMNOPQRTUVXYZ0":
        for s in (bytes([b]), bytes([b]).lower(), bytes([b]).upper()):
            add(main_cases, "ic:esc:%d:%s" % (b, s.hex()), 1, -1, b"\\" + bytes([b]), s)
            add(main_cases, "ic:escnc:%d:%s" % (b, s.hex()), 0, -1, b"\\" + bytes([b]), s)
    for pat in (b"[a-z]", b"[A-Z]", b"[Z-a]", b"[_-a]", b"[a-_]", b"[[.a.]]", b"[[=a=]]", b"[[.A.]-c]",
                b"[[:upper:]]+", b"[[:lower:]]+", b"[[:UPPER:]]", b"\\W+", b"[^a]+", b"[^[:lower:]]+"):
        for s in (b"aZ_", b"AbC", b"m", b"M", b"_", b"[\\]^_`"):
            add(main_cases, "ic:misc", 1, -1, pat, s)
            add(main_cases, "ic:miscnc", 0, -1, pat, s)


ERR_PATTERNS = [
    b"(", b"a(", b"(a", b"((a)", b"a)", b")", b"())", b"\\", b"a\\", b"[", b"[a", b"[]", b"[]a", b"[^", b"[^]",
    b"[^]a", b"[a-", b"[a-]", b"[-a]", b"[a--]", b"[--a]", b"[a-b-c]", b"[b-a]", b"[a-a]", b"[[:alpha:]",
    b"[[:alpha:", b"[[:alpha", b"[[:", b"[[:foo:]]", b"[[:ALPHA:]]", b"[[:alpha:]-z]", b"[a-[:alpha:]]",
    b"[[.a.]]", b"[[.ab.]]", b"[[..]]", b"[[.a.]-z]", b"[a-[.z.]]", b"[[.ab.]-z]", b"[[=a=]]", b"[[=ab=]]",
    b"[[=a=]-z]", b"[[.", b"[[=", b"[[.a", b"[[.a.", b"[[=a=", b"[[:a:]", b"[[:" + b"a" * 31 + b":]]",
    b"[[:" + b"a" * 32 + b":]]", b"[[." + b"a" * 31 + b".]]", b"[\\]", b"[\\]]", b"[]]", b"[]-a]", b"[a-]]",
    b"a{", b"a{1", b"a{1,", b"a{1,2", b"a{,", b"a{,2}", b"a{}", b"a{,}", b"a{x}", b"a{1x}", b"a{1,x}", b"a{2,1}",
    b"a{1,1}", b"a{32767}", b"a{32768}", b"a{0,32768}", b"a{99999999999}", b"a{1}{2}", b"a{1}*", b"a*{1}",
    b"{", b"{1}", b"a|{1}", b"(){1}", b"}", b"a}", b"a{1}}", b"*", b"*a", b"a|*", b"(*)", b"(|*)", b"^*",
    b"$*", b"a**", b"a*+?", b"+", b"?", b"a|+", b"(+)", b"\\1", b"(a)\\2", b"(a)\\1", b"(a\\1)", b"\\0",
    b"(a)(b)(c)(d)(e)(f)(g)(h)(i)\\9", b"(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)\\10", b"((a)\\2)", b"(a)|\\1",
    b"a\\", b"\\(", b"\\)", b"\\{", b"\\}", b"\\{1\\}", b"a\\{1\\}", b"\\|", b"\\.", b"\\w*\\", b"[a]\\",
    b"\\<\\>", b"\\b*", b"\\`*", b"\\'*", b"(^*)", b"x|", b"|x", b"|", b"||", b"()", b"(|)", b"(()|)", b"",
]


def errors():
    for i, p in enumerate(ERR_PATTERNS):
        for s in (b"", b"a", b"aa{1}", b"x(a)y"):
            add(bk_cases if b"\\1" in p or b"\\2" in p or b"\\9" in p else main_cases, "err:%d" % i, 0, -1, p, s)
        add(main_cases, "erri:%d" % i, 1, -1, p, b"A")


ATOMS = [b"a", b"b", b"c", b"x", b".", b"[ab]", b"[^a]", b"[a-c]", b"[[:alpha:]]", b"[[:digit:]]", b"\\w",
         b"\\W", b"\\s", b"\\S", b"()", b"A", b"1", b" ", b"a", b"b", b"(a|b)", b"(a*)", b"(ab|a)"]
# Anchors cannot take a repetition operator in ERE (REG_BADRPT); they get one
# only rarely so that most generated patterns compile.
ANCHORS = [b"\\b", b"\\B", b"\\<", b"\\>", b"\\`", b"\\'", b"^", b"$"]
QUANTS = [b"", b"", b"", b"*", b"+", b"?", b"{0}", b"{1}", b"{2}", b"{0,1}", b"{1,2}", b"{2,}", b"{,2}", b"{0,}",
          b"*?", b"+*", b"{1}?"]


def rnd_re(rng, depth, ngroups, allow_bk):
    """Random ERE; ngroups is a 1-element list counting opened groups."""
    nbranch = 1 if rng.random() < 0.7 else rng.randint(2, 3)
    branches = []
    for _ in range(nbranch):
        parts = []
        for _ in range(rng.randint(1 if depth > 0 else 1, 4)):
            r = rng.random()
            if depth < 2 and r < 0.25:
                ngroups[0] += 1
                inner = rnd_re(rng, depth + 1, ngroups, allow_bk)
                atom = b"(" + inner + b")"
            elif allow_bk and ngroups[0] > 0 and r < 0.35:
                atom = b"\\%d" % rng.randint(1, min(ngroups[0], 3))
            elif r < 0.45:
                parts.append(rng.choice(ANCHORS) + (rng.choice(QUANTS) if rng.random() < 0.03 else b""))
                continue
            else:
                atom = rng.choice(ATOMS)
            parts.append(atom + rng.choice(QUANTS))
        branches.append(b"".join(parts))
    return b"|".join(branches)


def rnd_subject(rng):
    alphabet = b"aaabbbccx A1\n_"
    n = rng.choice([0, 1, 2, 3, 4, 5, 6, 8, 10])
    return bytes(rng.choice(alphabet) for _ in range(n))


def fuzz(rng, count, allow_bk, prefix, lst):
    for i in range(count):
        # Keep patterns small: nested repetitions around anchors make the C
        # engine's compile time explode (both engines; see README).
        while True:
            g = [0]
            pat = rnd_re(rng, 0, g, allow_bk)
            if len(pat) <= 28:
                break
        icase = 1 if rng.random() < 0.2 else 0
        sub = rng.choice([-1, -1, -1, -1, 1, 2, 3, 20])
        for j in range(3):
            add(lst, "%s:%d:%d" % (prefix, i, j), icase, sub, pat, rnd_subject(rng))


def long_cases(rng):
    pats = [b"(a+)(b*)\\1", b"(ab|a)(bc|c)*", b"([a-c]*)\\1", b"x(.*)y", b"(a|ab)(c|bcd)(d*)", b"\\<(\\w+)\\>",
            b"((a)|b)+", b"(a*)*", b"(a|b)*c", b"([[:alpha:]]+)[[:space:]]+\\1"]
    for i, p in enumerate(pats):
        for j in range(4):
            n = rng.choice([40, 100, 300, 700])
            s = bytes(rng.choice(b"aabbcxy \n") for _ in range(n))
            lst = bk_cases if b"\\1" in p else main_cases
            for icase in (0, 1):
                add(lst, "long:%d:%d" % (i, j), icase, -1, p, s)
                add(lst, "longU:%d:%d" % (i, j), icase, -1, p, s.upper())


def main():
    rng = random.Random(20261117)
    suite(sys.argv[1])
    classes()
    icase_fold()
    errors()
    fuzz(rng, 2500, False, "fz", main_cases)
    fuzz(rng, 900, True, "bk", bk_cases)
    long_cases(rng)
    # Make ids unique.
    for lst, path in ((main_cases, sys.argv[2]), (bk_cases, sys.argv[3])):
        with open(path, "w", newline="\n") as f:
            for k, line in enumerate(lst):
                cid, rest = line.split("\t", 1)
                f.write("%s#%d\t%s\n" % (cid, k, rest))
    print("main %d, backref %d" % (len(main_cases), len(bk_cases)))


if __name__ == "__main__":
    main()
