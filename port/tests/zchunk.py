"""Is zlib's deflate output independent of how the input is split across Z_NO_FLUSH
calls? (C zlib 1.3.2 via CPython.) Compresses real PDF stream contents whole and in
pdfTeX-like 16 KB pieces, and in random pieces, at the levels pdfTeX uses.

Usage: python -I tests/zchunk.py FILE
"""
import random
import sys
import zlib

data = open(sys.argv[1], "rb").read()
print("zlib", zlib.ZLIB_RUNTIME_VERSION, "input", len(data))
rng = random.Random(1)
bad = 0
for level in (1, 6, 9):
    whole = zlib.compressobj(level)
    ref = whole.compress(data) + whole.flush()
    for trial in range(40):
        c = zlib.compressobj(level)
        out = b""
        i = 0
        while i < len(data):
            n = 16384 if trial == 0 else rng.choice([1, 7, 100, 4096, 16384, 65536, rng.randint(1, 300000)])
            out += c.compress(data[i:i + n])
            i += n
        out += c.flush()
        if out != ref:
            bad += 1
            print("level", level, "trial", trial, "DIFFERENT")
    # also: many small streams like a PDF has (each its own compressobj)
print("chunkings that changed the output:", bad)
