"""Compare the outputs of a difftest case that difftest itself cannot judge:
* every .synctex / .synctex.gz file (gz decompressed), after replacing the c run directory
  by the r one (exact spelling, so a drive-letter case difference would still show), and
  reporting whether the files are also byte-identical;
* every file in subdirectories (e.g. -output-directory), which difftest skips.
Usage: python -I cmpsync.py TESTDIR   (TESTDIR = .../test/NAME)"""
import gzip
import pathlib
import sys

base = pathlib.Path(sys.argv[1])
c, r = base / "c", base / "r"
ok = True
ca = str(c).replace("\\", "/").encode()
cr = str(r).replace("\\", "/").encode()


def rel_files(d):
    return {p.relative_to(d).as_posix() for p in d.rglob("*") if p.is_file()}


names = sorted(rel_files(c) | rel_files(r))
for n in names:
    in_sub = "/" in n
    if ".synctex" not in n and not in_sub:
        continue
    pc, pr = c / n, r / n
    if not pc.exists() or not pr.exists():
        print(f"  {n}: only in {'c' if pc.exists() else 'r'}")
        ok = False
        continue
    a, b = pc.read_bytes(), pr.read_bytes()
    if a == b:
        print(f"  {n}: identical bytes ({len(a)})")
        continue
    if a[:2] == b"\x1f\x8b" and b[:2] == b"\x1f\x8b":
        a, b = gzip.decompress(a), gzip.decompress(b)
    a = a.replace(ca, cr)
    if a == b:
        print(f"  {n}: identical after normalizing c->r ({len(a)} bytes content)")
    else:
        ok = False
        i = next((k for k in range(min(len(a), len(b))) if a[k] != b[k]), min(len(a), len(b)))
        print(f"  {n}: DIFF at {i}: {a[max(0, i - 60):i + 60]!r}\n     vs {b[max(0, i - 60):i + 60]!r}")
print("SYNC-OK" if ok else "SYNC-DIFF")
