#!/usr/bin/env python3
r"""Run regex cases through TeX Live's real pdfTeX (\pdfmatch) and record what
the bundled C regex engine does.

Usage: python -I tools/oracle.py <pdftex.exe> <cases.tsv> <out.tsv> [workdir] [chunk]

cases.tsv lines:  id \t icase(0/1) \t subcount(-1 = default) \t pattern_hex \t subject_hex
out.tsv lines:    id \t icase \t subcount \t pattern_hex \t subject_hex \t result
result:
  E:<regerror text>   regcomp failed (\pdfmatch returned -1; message from the
                      "\pdfmatch: ..." warning in the log)
  0                   regexec found no match
  1:<r0>;<r1>;...     match; one entry per \pdflastmatch i, i < nmatch
                      (nmatch = subcount, or 10 when subcount < 0), each
                      "so,eo" (eo = so + length of the reported text) or "-"
                      when pdfTeX reports "-1->"
  CRASH / TIMEOUT     pdfTeX died / hung on this case

Patterns and subjects are passed as \pdfunescapehex{...}, so every byte
except NUL reaches regcomp/regexec unchanged (pdfTeX hands C strings to the
engine).  pdfTeX always uses REG_EXTENDED (+ REG_ICASE for "icase") and
eflags = 0.
"""
import os
import re
import subprocess
import sys

CHUNK = 1500


def tex_for(cases, resname):
    out = [r"\catcode`\{=1 \catcode`\}=2 \catcode`\#=6",
           r"\immediate\openout1=" + resname]
    for k, (cid, icase, sub, pat, subj) in enumerate(cases):
        n = 10 if sub < 0 else sub
        kw = ("icase " if icase else "") + ("subcount %d " % sub if sub >= 0 else "")
        lm = "".join(r":\pdfescapehex{\pdflastmatch%d}" % i for i in range(n))
        out.append(r"\edef\r{\pdfmatch %s{\pdfunescapehex{%s}}{\pdfunescapehex{%s}}}"
                   r"\immediate\write1{@%d:\r%s}" % (kw, pat, subj, k, lm))
    out.append(r"\immediate\closeout1")
    out.append(r"\end")
    return "\n".join(out) + "\n"


def run_chunk(pdftex, cases, workdir, tag):
    """Returns dict id -> result, or None if pdfTeX failed for this chunk."""
    texname = "orc%s.tex" % tag
    resname = "orc%s.res" % tag
    logname = "orc%s.log" % tag
    for fn in (resname, logname):
        try:
            os.remove(os.path.join(workdir, fn))
        except FileNotFoundError:
            pass
    with open(os.path.join(workdir, texname), "w", newline="\n") as f:
        f.write(tex_for(cases, resname))
    env = dict(os.environ)
    env["max_print_line"] = "1000000"
    env["error_line"] = "254"
    env["half_error_line"] = "238"
    try:
        p = subprocess.Popen([pdftex, "-ini", "-interaction=batchmode", texname], cwd=workdir, env=env,
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        p.wait(timeout=20 + len(cases) * 0.05)
    except subprocess.TimeoutExpired:
        # A failed assert() in the C engine makes the MSVC runtime of
        # pdftex.dll show a modal "Microsoft Visual C++ Runtime Library"
        # dialog, so the process blocks instead of exiting.  Tell that apart
        # from a genuine hang by the window title.
        kind = "TIMEOUT"
        try:
            out = subprocess.run(["tasklist", "/v", "/fo", "csv", "/fi", "PID eq %d" % p.pid],
                                 capture_output=True, timeout=30).stdout.decode("utf-8", "replace")
            if "Runtime Library" in out:
                kind = "ASSERT"
        except Exception:
            pass
        p.kill()
        p.wait()
        return kind
    if p.returncode != 0:
        return "CRASH"
    try:
        res = open(os.path.join(workdir, resname), "rb").read().decode("latin-1").split("\n")
        log = open(os.path.join(workdir, logname), "rb").read().decode("latin-1")
    except FileNotFoundError:
        return "CRASH"
    try:
        return parse_results(cases, res, log)
    except (ValueError, AttributeError, IndexError):
        # A truncated output file: pdfTeX died while writing.
        return "CRASH"


def parse_results(cases, res, log):
    warnings = re.findall(r"\\pdfmatch: ([^\r\n]*)", log)
    results = {}
    wi = 0
    for line in res:
        if not line.startswith("@"):
            continue
        k, rest = line[1:].split(":", 1)
        cid = cases[int(k)][0]
        parts = rest.split(":")
        r = parts[0]
        if r == "-1":
            results[cid] = "E:" + (warnings[wi] if wi < len(warnings) else "?")
            wi += 1
        elif r == "0":
            results[cid] = "0"
        elif r == "1":
            regs = []
            for h in parts[1:]:
                s = bytes.fromhex(h).decode("latin-1")
                m = re.match(r"(-?\d+)->", s)
                so = int(m.group(1))
                if so < 0:
                    regs.append("-")
                else:
                    regs.append("%d,%d" % (so, so + len(s) - len(m.group(0))))
            results[cid] = "1:" + ";".join(regs)
        else:
            results[cid] = "?" + r
    if len(results) != len(cases):
        return "CRASH"
    return results


def run_all(pdftex, cases, workdir, tag="0"):
    r = run_chunk(pdftex, cases, workdir, tag)
    if isinstance(r, dict):
        return r
    if len(cases) == 1:
        return {cases[0][0]: r}
    mid = len(cases) // 2
    out = run_all(pdftex, cases[:mid], workdir, tag + "a")
    out.update(run_all(pdftex, cases[mid:], workdir, tag + "b"))
    return out


def main():
    pdftex, inp, outp = sys.argv[1], sys.argv[2], sys.argv[3]
    workdir = sys.argv[4] if len(sys.argv) > 4 else os.path.join(os.path.dirname(os.path.abspath(outp)), "oracle-work")
    os.makedirs(workdir, exist_ok=True)
    global CHUNK
    if len(sys.argv) > 5:
        CHUNK = int(sys.argv[5])
    cases = []
    for line in open(inp):
        if line.startswith("#") or not line.strip():
            continue
        f = line.rstrip("\n").split("\t")
        cases.append((f[0], int(f[1]), int(f[2]), f[3], f[4]))
    results = {}
    for k in range(0, len(cases), CHUNK):
        chunk = cases[k:k + CHUNK]
        results.update(run_all(pdftex, chunk, workdir, "c%d" % (k // CHUNK)))
        print("%d/%d" % (min(k + CHUNK, len(cases)), len(cases)), file=sys.stderr)
    with open(outp, "w", newline="\n") as f:
        f.write("# id\ticase\tsubcount\tpattern_hex\tsubject_hex\tresult\n")
        f.write("# generated by tools/oracle.py with TeX Live 2026 pdftex.exe (pdfTeX 1.40.29, \\pdfmatch)\n")
        for (cid, icase, sub, pat, subj) in cases:
            f.write("%s\t%d\t%d\t%s\t%s\t%s\n" % (cid, icase, sub, pat, subj, results.get(cid, "?")))


if __name__ == "__main__":
    main()
