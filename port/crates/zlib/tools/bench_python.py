"""CPython zlib (C zlib 1.3.2) counterpart of examples/bench.rs.

Usage: python -I tools/bench_python.py <file> [reps]
"""
import sys
import time
import zlib

path = sys.argv[1]
reps = int(sys.argv[2]) if len(sys.argv) > 2 else 5
data = open(path, "rb").read()
print(f"input {len(data)} bytes, crc {zlib.crc32(data):08x}, zlib {zlib.ZLIB_RUNTIME_VERSION}")
for level in (1, 6, 9):
    best = 1e9
    for _ in range(reps):
        t = time.perf_counter()
        comp = zlib.compress(data, level)
        best = min(best, time.perf_counter() - t)
    ibest = 1e9
    for _ in range(reps):
        t = time.perf_counter()
        back = zlib.decompress(comp, 15, len(data))
        ibest = min(ibest, time.perf_counter() - t)
    assert back == data
    print(f"level {level}: {len(comp)} bytes (crc {zlib.crc32(comp):08x})  "
          f"deflate {best * 1e3:.1f} ms ({len(data) / best / 1e6:.1f} MB/s)  "
          f"inflate {ibest * 1e3:.1f} ms ({len(data) / ibest / 1e6:.1f} MB/s)")

# Direct C zlib 1.3.2 (Git for Windows' zlib1.dll, or $TEXRS_ZLIB_ORACLE_DLL)
# through ctypes: one compress2()/uncompress() call, no Python buffering.
import ctypes
import os

dll = os.environ.get("TEXRS_ZLIB_ORACLE_DLL", r"C:\Program Files\Git\mingw64\bin\zlib1.dll")
if os.path.exists(dll):
    z = ctypes.CDLL(dll)
    z.zlibVersion.restype = ctypes.c_char_p
    print(f"direct C {dll} ({z.zlibVersion().decode()}):")
    src = ctypes.create_string_buffer(data, len(data))
    cap = len(data) + len(data) // 100 + 1024
    dst = ctypes.create_string_buffer(cap)
    back = ctypes.create_string_buffer(len(data))
    for level in (1, 6, 9):
        best = 1e9
        for _ in range(reps):
            n = ctypes.c_ulong(cap)
            t = time.perf_counter()
            r = z.compress2(dst, ctypes.byref(n), src, ctypes.c_ulong(len(data)), level)
            best = min(best, time.perf_counter() - t)
            assert r == 0
        clen = n.value
        ibest = 1e9
        for _ in range(reps):
            m = ctypes.c_ulong(len(data))
            t = time.perf_counter()
            r = z.uncompress(back, ctypes.byref(m), dst, ctypes.c_ulong(clen))
            ibest = min(ibest, time.perf_counter() - t)
            assert r == 0
        print(f"level {level}: {clen} bytes (crc {zlib.crc32(dst.raw[:clen]):08x})  "
              f"deflate {best * 1e3:.1f} ms ({len(data) / best / 1e6:.1f} MB/s)  "
              f"inflate {ibest * 1e3:.1f} ms ({len(data) / ibest / 1e6:.1f} MB/s)")
