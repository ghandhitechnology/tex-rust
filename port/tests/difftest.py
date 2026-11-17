"""Differential test: run TeX Live's pdftex and the Rust port on the same job and compare
every output file (stdout+stderr, .log, .dvi, .pdf, .fmt, .aux, ...) byte for byte.

Usage:
  python -I port/tests/difftest.py NAME [--files f1 f2 ...] -- ARGS...
      runs `pdftex ARGS...` in port/test/NAME/{c,r}, after copying the given input files.
  python -I port/tests/difftest.py --suite FILE.json   (list of {"name","files","args","env"})

Format files are compared after gunzip (TeX Live writes them gzip-compressed).
font_check[null_font] is masked in formats: TeX never initializes it, so it holds C
malloc garbage that differs even between two TeX Live runs. Its offset comes from the
Rust run's TEXRS_DUMPTRACE output. Both runs get SOURCE_DATE_EPOCH and
FORCE_SOURCE_DATE=1 so dates and /ID are deterministic.
"""
import gzip
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
# TEXRS_TEXLIVE: the TeX Live 2026 installation to compare against (default tools/texlive/2026)
TL_ROOT = pathlib.Path(os.environ.get("TEXRS_TEXLIVE", ROOT / "tools" / "texlive" / "2026"))
TL_BIN = TL_ROOT / "bin" / "windows"
# TEXRS_ENGINE_DIR: an engine crate other than port/engine (e.g. a delegate's working copy)
ENGINE_DIR = pathlib.Path(os.environ.get("TEXRS_ENGINE_DIR", ROOT / "port" / "engine"))
RUST_BIN = ENGINE_DIR / "target" / os.environ.get("TEXRS_PROFILE", "debug") / "pdftex.exe"
# TEXRS_WORK: where test directories are created (default port/test)
WORK = pathlib.Path(os.environ.get("TEXRS_WORK", ROOT / "port" / "test"))
SEP = b"\n--stderr--\n"


def selfautoloc():
    p = str(TL_BIN).replace("\\", "/")
    return p[0].lower() + p[1:]  # kpathsea reports the drive letter in lower case


def run(exe, args, cwd, env_extra, timeout):
    env = dict(os.environ)
    env.update({"SOURCE_DATE_EPOCH": "1700000000", "FORCE_SOURCE_DATE": "1", "TEXRS_SELFAUTOLOC": selfautoloc()})
    env["PATH"] = str(TL_BIN) + os.pathsep + env["PATH"]
    env.update(env_extra or {})
    # a leading "<FILE" argument feeds FILE (relative to the run directory) to stdin
    stdin_data = b""
    if args and args[0].startswith("<"):
        stdin_data = (pathlib.Path(cwd) / args[0][1:]).read_bytes()
        args = args[1:]
    t0 = time.perf_counter()
    try:
        p = subprocess.run([str(exe), *args], cwd=cwd, env=env, capture_output=True, timeout=timeout,
                           input=stdin_data)
        rc, out, err = p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired as e:
        rc, out, err = "timeout", e.stdout or b"", e.stderr or b""
    return rc, out, err, time.perf_counter() - t0


def content(path):
    data = path.read_bytes()
    if path.suffix == ".fmt" and data[:2] == b"\x1f\x8b":
        return gzip.decompress(data)
    return data


def first_diff(a, b):
    n = min(len(a), len(b))
    for i in range(n):
        if a[i] != b[i]:
            return i
    return n if len(a) != len(b) else -1


_noise_cache = {}


def tl_noise(name, files, steps, env, timeout, fname):
    """Content of output `fname` from a second, independent TeX Live run of the same case."""
    key = (name, fname)
    if key not in _noise_cache:
        d = WORK / name / "c2"
        if d.exists():
            shutil.rmtree(d)
        d.mkdir(parents=True)
        for f in files:
            src = pathlib.Path(f)
            (shutil.copytree if src.is_dir() else shutil.copy2)(src, d / src.name)
        for st in steps:
            run(TL_BIN / "pdftex.exe", [x.replace("{SIDE}", "c") for x in st], d, env, timeout)
        p = d / fname
        _noise_cache[key] = content(p) if p.exists() else None
    return _noise_cache[key]


def case(name, files, args, env=None, timeout=600, quiet=False):
    """args: one argument list, or a list of argument lists run in sequence (steps)."""
    steps = args if args and isinstance(args[0], list) else [args]
    base = WORK / name
    if base.exists():
        shutil.rmtree(base)
    dirs = {"c": base / "c", "r": base / "r"}
    for d in dirs.values():
        d.mkdir(parents=True)
        for f in files:
            src = pathlib.Path(f)
            if src.is_dir():
                shutil.copytree(src, d / src.name)
            else:
                shutil.copy2(src, d / src.name)
    env_r = dict(env or {})
    env_r["TEXRS_DUMPTRACE"] = "1"
    rc_c, out_c, err_c, t_c = [], b"", b"", 0.0
    rc_r, out_r, err_r, t_r = [], b"", b"", 0.0
    for st in steps:
        # "{SIDE}" in an argument becomes "c" (TeX Live run) or "r" (Rust run), e.g. to use
        # a format each engine built for itself: -fmt=.../_fmt/{SIDE}/pdflatex
        a1, b1, c1, d1 = run(TL_BIN / "pdftex.exe", [x.replace("{SIDE}", "c") for x in st], dirs["c"], env, timeout)
        a2, b2, c2, d2 = run(RUST_BIN, [x.replace("{SIDE}", "r") for x in st], dirs["r"], env_r, timeout)
        rc_c.append(a1); out_c += b1; err_c += c1; t_c += d1
        rc_r.append(a2); out_r += b2; err_r += c2; t_r += d2
    mask = []
    kept = []
    dumps = []  # (offset, nitems, size) of every format dump call, from the Rust trace
    for line in err_r.splitlines(keepends=True):
        if line.startswith(b"DUMP "):
            f = line.split()
            dumps.append((int(f[1].split(b"=")[1]), int(f[2].split(b"=")[1]), int(f[3].split(b"=")[1])))
            if line.rstrip().endswith(b"fontcheck"):
                mask.append((int(line.split(b"off=")[1].split()[0]), 4))
        else:
            kept.append(line)
    err_r = b"".join(kept)
    (dirs["c"] / "_stdout.txt").write_bytes(out_c + SEP + err_c)
    (dirs["r"] / "_stdout.txt").write_bytes(out_r + SEP + err_r)
    problems = []
    if rc_c != rc_r:
        problems.append(f"exit code {rc_c} vs {rc_r}")
    def tree(d):
        return {p.relative_to(d).as_posix() for p in d.rglob("*") if p.is_file()}

    names = sorted(tree(dirs["c"]) | tree(dirs["r"]))
    for n in names:
        pc, pr = dirs["c"] / n, dirs["r"] / n
        if pc.is_dir() or pr.is_dir():
            continue
        if not pc.exists() or not pr.exists():
            problems.append(f"{n}: only in {'c' if pc.exists() else 'r'}")
            continue
        a, b = content(pc), content(pr)
        # the two runs happen in different directories (.../c and .../r): normalise the
        # run directory where it legitimately appears (recorder PWD, absolute paths)
        side_paths = {"c": [str(dirs["c"])], "r": [str(dirs["r"])]}
        # paths given per engine with {SIDE} (e.g. each engine's own format) also differ
        for st in steps:
            for x in st:
                if "{SIDE}" in x:
                    v = x.split("=", 1)[-1]
                    side_paths["c"].append(v.replace("{SIDE}", "c"))
                    side_paths["r"].append(v.replace("{SIDE}", "r"))
        for side in ("c", "r"):
            for d in side_paths[side]:
                fwd = d.replace("\\", "/").encode()
                for form in (fwd, fwd[:1].lower() + fwd[1:], d.encode()):
                    if side == "c":
                        a = a.replace(form, b"<RUNDIR>")
                    else:
                        b = b.replace(form, b"<RUNDIR>")
        if pc.suffix == ".fmt" and len(a) == len(b):
            a, b = bytearray(a), bytearray(b)
            for off, ln in mask:
                a[off:off + ln] = bytes(ln)
                b[off:off + ln] = bytes(ln)
            if a != b:
                # Formats can contain memory TeX never initializes (C malloc garbage that
                # differs between two TeX Live runs). Rerun TeX Live and ignore exactly
                # the bytes where TeX Live disagrees with itself.
                noise = tl_noise(name, files, steps, env, timeout, n)
                if noise is not None and len(noise) == len(a):
                    k = 0
                    for i, (x, y) in enumerate(zip(a, noise)):
                        if x != y:
                            # mask the whole dumped item (memory word) containing the byte
                            lo, hi = i, i + 1
                            for off, cnt, size in dumps:
                                if off <= i < off + cnt * size and size <= 8:
                                    lo = off + (i - off) // size * size
                                    hi = lo + size
                                    break
                            for j in range(lo, hi):
                                a[j] = b[j] = 0
                            k += 1
                    if k:
                        print(f"    ({n}: {k} bytes masked where two TeX Live runs differ)")
            a, b = bytes(a), bytes(b)
        if a != b:
            i = first_diff(a, b)
            nd = sum(1 for x, y in zip(a, b) if x != y) + abs(len(a) - len(b))
            problems.append(f"{n}: {nd} bytes differ, first at {i} (sizes {len(a)} vs {len(b)}): "
                            f"{a[max(0, i - 30):i + 40]!r} vs {b[max(0, i - 30):i + 40]!r}")
    status = "SAME" if not problems else "DIFF"
    if not quiet or problems:
        print(f"[{status}] {name}  (TL {t_c * 1000:.0f} ms, Rust {t_r * 1000:.0f} ms)")
        for p in problems:
            print("   ", p[:500])
    return not problems, t_c, t_r


def main():
    argv = sys.argv[1:]
    if argv and argv[0] == "--suite":
        suite = json.loads(pathlib.Path(argv[1]).read_text())
        ok = 0
        for c in suite:
            r, _, _ = case(c["name"], [ROOT / f for f in c.get("files", [])], c["args"], c.get("env"))
            ok += r
        print(f"{ok}/{len(suite)} identical")
        return
    name = argv.pop(0)
    files = []
    if argv and argv[0] == "--files":
        argv.pop(0)
        while argv and argv[0] != "--":
            files.append(pathlib.Path(argv.pop(0)))
    steps = []
    while argv and argv[0] == "--":
        argv.pop(0)
        cur = []
        while argv and argv[0] != "--":
            cur.append(argv.pop(0))
        steps.append(cur)
    case(name, files, steps)


if __name__ == "__main__":
    main()
