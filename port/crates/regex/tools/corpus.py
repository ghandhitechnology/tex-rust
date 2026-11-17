#!/usr/bin/env python3
"""Extract test cases (with the suites' own expectations) from glibc's and
AT&T's regex test data into tests/data/suite_cases.tsv.

Usage: python -I tools/corpus.py <glibc-posix-dir> <fowler-dat-dir> <out.tsv>

Inputs (fetched from the web, see README "Test sources"):
  glibc posix/: PCRE.tests, BOOST.tests, rxspencer/tests (saved as
                rxspencer_tests), PTESTS, TESTS
  AT&T testregex (via rust-lang/regex testdata/fowler/dat): basic.dat,
                nullsubexpr.dat, repetition.dat

Output columns (tab separated):
  source  cflags  eflags  nmatch  pattern_hex  subject_hex  expect
cflags/eflags use the POSIX values (REG_EXTENDED=1, REG_ICASE=2,
REG_NEWLINE=4, REG_NOSUB=8; REG_NOTBOL=1, REG_NOTEOL=2).
expect is one of
  ERR:<NAME>   regcomp must fail with REG_<NAME> (ERR:* = any error)
  NOMATCH      regexec must fail
  MATCH[:spec;spec;...]  regexec must succeed; spec i describes register i:
      so,eo    exact offsets          -       unset (-1,-1)
      ?        not checked            T=<hex> matched text equals <hex>
      @<hex>   empty match followed by <hex> (rxspencer convention)
"""
import os
import re
import sys

REG_EXTENDED, REG_ICASE, REG_NEWLINE, REG_NOSUB = 1, 2, 4, 8
REG_NOTBOL, REG_NOTEOL = 1, 2

ERRNAMES = ["NOERROR", "NOMATCH", "BADPAT", "ECOLLATE", "ECTYPE", "EESCAPE", "ESUBREG",
            "EBRACK", "EPAREN", "EBRACE", "BADBR", "ERANGE", "ESPACE", "BADRPT",
            "EEND", "ESIZE", "ERPAREN"]

out = []


def hx(b):
    return b.hex()


def emit(source, cflags, eflags, nmatch, pattern, subject, expect):
    out.append("\t".join([source, str(cflags), str(eflags), str(nmatch), hx(pattern), hx(subject), expect]))


def lat(s):
    return s.encode("latin-1")


# --------------------------------------------------------------------------
def pcre(path):
    """posix/PCRE.tests, interpreted exactly like posix/tst-pcre.c."""
    pattern = None
    icase = False
    subject = None
    regs = {}
    nomatch = False
    n = 0

    def flush():
        nonlocal subject, regs, nomatch, n
        # tst-pcre.c only checks a subject when result lines follow it.
        if pattern is not None and subject is not None and (nomatch or regs):
            if nomatch:
                exp = "NOMATCH"
            else:
                specs = []
                for i in range(max(regs) + 1 if regs else 1):
                    specs.append(regs.get(i, "?"))
                exp = "MATCH:" + ";".join(specs)
            emit("pcre:%d" % n, REG_EXTENDED | (REG_ICASE if icase else 0), 0, 20,
                 pattern, subject, exp)
        subject = None
        regs = {}
        nomatch = False

    with open(path, "rb") as f:
        lines = f.read().split(b"\n")
    for ln, line in enumerate(lines, 1):
        line = line.decode("latin-1")
        if line.startswith("#"):
            continue
        if line == "":
            flush()
            pattern = None
            icase = False
            continue
        if line.startswith("/"):
            flush()
            p = line.rfind("/")
            if p == 0:
                pattern = None
                continue
            flags = line[p + 1:]
            if flags not in ("", "i"):
                pattern = None
                continue
            icase = flags == "i"
            pattern = lat(line[1:p])
            continue
        if line.startswith("    "):
            flush()
            if pattern is None:
                continue
            subject = lat(line[4:])
            n = ln
            continue
        if subject is None:
            continue
        if line == "No match":
            nomatch = True
            continue
        m = re.match(r" ?(\d+): (.*)$", line)
        if m:
            idx = int(m.group(1))
            txt = m.group(2)
            regs[idx] = "-" if txt == "<unset>" else "T=" + hx(lat(txt))
    flush()


# --------------------------------------------------------------------------
def frob_escapes(s, is_pattern):
    """tst-boost.c frob_escapes."""
    outb = []
    i = 0
    while i < len(s):
        c = s[i]
        if c == "\\" and i + 1 < len(s):
            d = s[i + 1]
            if d == "t":
                outb.append("\t"); i += 2; continue
            if d == "n":
                outb.append("\n"); i += 2; continue
            if d == "r":
                outb.append("\r"); i += 2; continue
            if d in "\\^{|}" and not is_pattern:
                outb.append(d); i += 2; continue
        outb.append(c)
        i += 1
    return "".join(outb)


def boost(path):
    """posix/BOOST.tests, interpreted like posix/tst-boost.c."""
    flags = REG_EXTENDED
    eflags = 0
    with open(path, "rb") as f:
        lines = f.read().split(b"\n")
    for ln, raw in enumerate(lines, 1):
        line = raw.decode("latin-1")
        if line.startswith(";") or line == "":
            continue
        if line.startswith("-"):
            flags = 0 if "REG_BASIC" in line else REG_EXTENDED
            if "REG_ICASE" in line:
                flags |= REG_ICASE
            if "REG_NEWLINE" in line:
                flags |= REG_NEWLINE
            eflags = 0
            if "REG_NOTBOL" in line:
                eflags |= REG_NOTBOL
            if "REG_NOTEOL" in line:
                eflags |= REG_NOTEOL
            continue
        s = line.lstrip(" \t")
        if s == "":
            continue
        k = len(re.match(r"[^ \t]*", s).group(0))
        if k == len(s):
            continue
        pattern, rest = s[:k], s[k + 1:]
        rest = rest.lstrip(" \t")
        if rest == "":
            continue
        if rest.startswith('"'):
            q = rest.find('"', 1)
            if q < 0:
                continue
            string = rest[1:q]
            p = rest[q + 1:]
        else:
            k2 = len(re.match(r"[^ \t]*", rest).group(0))
            if rest.startswith("!"):
                string = None
                p = rest[k2:]
            elif k2 == len(rest):
                continue
            else:
                string = rest[:k2]
                p = rest[k2 + 1:]
        pat = frob_escapes(pattern, True)
        if string is None:
            emit("boost:%d" % ln, flags, eflags, 20, lat(pat), b"", "ERR:*")
            continue
        subj = frob_escapes(string, False)
        nums = [int(x) for x in re.findall(r"-?\d+", p)]
        pairs = [(nums[i], nums[i + 1]) for i in range(0, len(nums) - 1, 2)][:20]
        if pairs and pairs[0] == (-1, -1):
            exp = "NOMATCH"
        else:
            exp = "MATCH:" + ";".join("%d,%d" % pr if pr != (-1, -1) else "-" for pr in pairs)
        emit("boost:%d" % ln, flags, eflags, 20, lat(pat), lat(subj), exp)


# --------------------------------------------------------------------------
def rxspencer(path):
    """posix/rxspencer/tests, interpreted like posix/tst-rxspencer.c (C
    locale run only)."""

    def special(s):
        return s.replace("N", "\n").replace("T", "\t").replace("S", " ").replace("Z", "\0")

    def glibc_re_syntax(s):
        return s.replace("[[:<:]]", "\\<").replace("[[:>:]]", "\\>")

    with open(path, "rb") as f:
        lines = f.read().split(b"\n")
    for ln, raw in enumerate(lines, 1):
        line = raw.decode("latin-1")
        if line.startswith("#") or line == "":
            continue
        fields = [x for x in line.split("\t") if x != ""]
        if len(fields) < 3:
            continue
        pattern, flagstr, string = fields[0], fields[1], fields[2]
        if pattern == '""':
            pattern = ""
        if string == '""':
            string = ""
        cflags, eflags, both, cerr = REG_EXTENDED, 0, False, False
        skip = False
        for ch in flagstr:
            if ch == "b":
                cflags &= ~REG_EXTENDED
            elif ch == "&":
                both = True
            elif ch == "C":
                cerr = True
            elif ch == "i":
                cflags |= REG_ICASE
            elif ch == "s":
                cflags |= REG_NOSUB
            elif ch == "n":
                cflags |= REG_NEWLINE
            elif ch == "^":
                eflags |= REG_NOTBOL
            elif ch == "$":
                eflags |= REG_NOTEOL
            elif ch in "mp#":
                skip = True
        if skip:
            continue
        pattern = glibc_re_syntax(special(pattern))
        if not cerr:
            string = special(string)
        expect = special(fields[3]) if len(fields) > 3 else None
        matches = special(fields[4]) if len(fields) > 4 else None
        if cerr:
            # The suite expects REG_EMPTY for empty REs; glibc ignores that.
            if string == "EMPTY":
                continue
            exp = "ERR:" + string
            subj = b""
        else:
            subj = lat(string)
            if expect is None:
                exp = "NOMATCH"
            elif cflags & REG_NOSUB:
                exp = "MATCH"
            else:
                def spec(m):
                    if m == "-":
                        return "-"
                    if m.startswith("@"):
                        return "@" + hx(lat(m[1:]))
                    return "T=" + hx(lat(m))
                specs = [spec(expect)]
                if matches is not None:
                    parts = matches.split(",")
                    for k in range(1, 10):
                        specs.append(spec(parts[k - 1]) if k - 1 < len(parts) else "-")
                exp = "MATCH:" + ";".join(specs)
        emit("rxspencer:%d" % ln, cflags, eflags, 10, lat(pattern), subj, exp)
        if both:
            emit("rxspencer:%d:bre" % ln, cflags & ~REG_EXTENDED, eflags, 10, lat(pattern), subj, exp)


# --------------------------------------------------------------------------
def ptests(path):
    """posix/PTESTS (POSIX.2 BRE conformance cases), like posix/runptests.c."""
    with open(path, "rb") as f:
        lines = f.read().split(b"\n")
    for ln, raw in enumerate(lines, 1):
        line = raw.decode("latin-1")
        if line.startswith("#") or line == "":
            continue
        m = re.match(r"([^|]*)\|([^|]*)\|([^|]*)\|([^|]*)\|(.*)$", line)
        if not m:
            continue
        start, end = int(m.group(1)), int(m.group(2))
        pattern, string = m.group(3), m.group(4)
        opts = m.group(5).strip()
        cflags = 0
        if "REG_EXTENDED" in opts:
            cflags |= REG_EXTENDED
        if start == -2:
            exp = "ERR:*"
        elif start == -1:
            exp = "NOMATCH"
        elif start == 0 and end == 0:
            exp = "MATCH:0,0"
        else:
            exp = "MATCH:%d,%d" % (start - 1, end)
        emit("ptests:%d" % ln, cflags, 0, 20, lat(pattern), lat(string), exp)


def tests_file(path):
    """posix/TESTS (Henry Spencer's original set), like posix/runtests.c."""
    with open(path, "rb") as f:
        lines = f.read().split(b"\n")
    for ln, raw in enumerate(lines, 1):
        line = raw.decode("latin-1")
        if line.startswith("#") or line == "":
            continue
        m = re.match(r"([0-9]*):(.*):(.*)$", line)
        if not m:
            continue
        e = int(m.group(1))
        exp = {0: "MATCH", 1: "NOMATCH", 2: "ERR:*"}[e]
        emit("tests:%d" % ln, REG_EXTENDED, 0, 10, lat(m.group(2)), lat(m.group(3)), exp)


# --------------------------------------------------------------------------
def unescape_c(s):
    """testregex '$' flag: expand C escapes."""
    outb = bytearray()
    i = 0
    while i < len(s):
        c = s[i]
        if c == "\\" and i + 1 < len(s):
            d = s[i + 1]
            simple = {"n": 10, "t": 9, "r": 13, "f": 12, "v": 11, "a": 7, "b": 8, "e": 27, "\\": 92}
            if d in simple:
                outb.append(simple[d]); i += 2; continue
            if d == "x":
                m = re.match(r"[0-9a-fA-F]{1,2}", s[i + 2:])
                if m:
                    outb.append(int(m.group(0), 16)); i += 2 + len(m.group(0)); continue
            if d in "01234567":
                m = re.match(r"[0-7]{1,3}", s[i + 1:])
                outb.append(int(m.group(0), 8) & 255); i += 1 + len(m.group(0)); continue
        outb += c.encode("latin-1")
        i += 1
    return bytes(outb)


def fowler(path, name):
    """AT&T testregex .dat files (Go/Rust copies).  Only B/E lines whose
    flags we understand are kept; lines annotated 'Rust'/'RE2' (modified for
    other engines) are skipped, '#'-commented originals are restored."""
    with open(path, "rb") as f:
        lines = f.read().split(b"\n")
    prev_pattern = None
    for ln, raw in enumerate(lines, 1):
        line = raw.decode("latin-1")
        if line.strip() == "":
            continue
        fields = [x for x in re.split(r"\t+", line)]
        if len(fields) < 4:
            continue
        flags, pattern, string, expect = fields[0], fields[1], fields[2], fields[3]
        # testregex: "SAME" repeats the previous line's pattern.
        if pattern == "SAME":
            pattern = prev_pattern
        prev_pattern = pattern
        note = fields[4] if len(fields) > 4 else ""
        if "Rust" in note or "RE2" in note or "Go" in note:
            continue
        if flags.startswith("#"):
            flags = flags[1:]
        flags = flags.rstrip("0123456789")
        if not re.fullmatch(r"[BE]+[i$n]*|[BE]+\$|[BE]+n\$", flags):
            continue
        dollar = "$" in flags
        icase = "i" in flags
        newline = "n" in flags
        if string == "NULL":
            string = ""
        pat = unescape_c(pattern) if dollar else lat(pattern)
        subj = unescape_c(string) if dollar else lat(string)
        if expect == "NOMATCH":
            exp = "NOMATCH"
        elif re.fullmatch(r"[A-Z]+", expect):
            exp = "ERR:" + expect
        elif expect.startswith("("):
            pairs = re.findall(r"\(([^,]*),([^)]*)\)", expect)
            specs = []
            for a, b in pairs:
                if a == "?" or b == "?":
                    specs.append("-")
                else:
                    specs.append("%d,%d" % (int(a), int(b)))
            exp = "MATCH:" + ";".join(specs)
        else:
            continue
        for kind in "BE":
            if kind not in flags:
                continue
            cflags = (REG_EXTENDED if kind == "E" else 0) | (REG_ICASE if icase else 0) | (REG_NEWLINE if newline else 0)
            emit("%s:%d%s" % (name, ln, "" if kind == "E" else ":bre"), cflags, 0, 20, pat, subj, exp)


def main():
    gdir, fdir, outpath = sys.argv[1], sys.argv[2], sys.argv[3]
    pcre(os.path.join(gdir, "PCRE.tests"))
    boost(os.path.join(gdir, "BOOST.tests"))
    rxspencer(os.path.join(gdir, "rxspencer_tests"))
    ptests(os.path.join(gdir, "PTESTS"))
    tests_file(os.path.join(gdir, "TESTS"))
    for fn in ("basic.dat", "nullsubexpr.dat", "repetition.dat"):
        fowler(os.path.join(fdir, fn), "att-" + fn.split(".")[0])
    with open(outpath, "w", newline="\n") as f:
        f.write("# source\tcflags\teflags\tnmatch\tpattern_hex\tsubject_hex\texpect\n")
        f.write("# generated by tools/corpus.py from glibc posix/ and AT&T testregex data\n")
        for l in out:
            f.write(l + "\n")
    print("wrote %d cases to %s" % (len(out), outpath))


if __name__ == "__main__":
    main()
