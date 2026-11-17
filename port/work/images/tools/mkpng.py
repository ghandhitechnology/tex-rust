"""Generate PNG test images (stdlib only): every color type / bit depth, tRNS, alpha,
interlacing, gamma and the ancillary chunks that block pdfTeX's PNG copy path, all
row filter types, several IDAT layouts, plus a few damaged files.

usage: python -I mkpng.py OUTDIR
"""
import os
import struct
import sys
import zlib

OUT = sys.argv[1]
os.makedirs(OUT, exist_ok=True)


def chunk(t, data, crc_ok=True):
    c = zlib.crc32(t + data) & 0xFFFFFFFF
    if not crc_ok:
        c ^= 0x5A5A5A5A
    return struct.pack(">I", len(data)) + t + data + struct.pack(">I", c)


class Rng:
    def __init__(self, seed):
        self.s = seed & 0xFFFFFFFF

    def next(self):
        self.s = (self.s * 1103515245 + 12345) & 0x7FFFFFFF
        return self.s >> 8


def sample(rng, x, y, c, depth, mode):
    maxv = (1 << depth) - 1
    if mode == "grad":
        v = (x * 7 + y * 13 + c * 29) % (maxv + 1)
    elif mode == "rand":
        v = rng.next() % (maxv + 1)
    else:  # smooth: gradient scaled to full range
        v = ((x + y + c * 3) * maxv) // max(1, (x + y + c * 3) + 5)
    return v


def pack_row(vals, depth):
    if depth >= 8:
        out = bytearray()
        for v in vals:
            out += struct.pack(">H", v) if depth == 16 else bytes([v])
        return bytes(out)
    out = bytearray()
    acc = 0
    n = 0
    for v in vals:
        acc = (acc << depth) | v
        n += depth
        if n == 8:
            out.append(acc)
            acc = 0
            n = 0
    if n:
        out.append(acc << (8 - n))
    return bytes(out)


def paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    if pb <= pc:
        return b
    return c


def filt(raw, prev, ftype, bpp):
    out = bytearray([ftype])
    for i, x in enumerate(raw):
        a = raw[i - bpp] if i >= bpp else 0
        b = prev[i] if prev else 0
        c = prev[i - bpp] if (prev and i >= bpp) else 0
        if ftype == 0:
            p = 0
        elif ftype == 1:
            p = a
        elif ftype == 2:
            p = b
        elif ftype == 3:
            p = (a + b) // 2
        else:
            p = paeth(a, b, c)
        out.append((x - p) & 0xFF)
    return bytes(out)


CHANNELS = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}
ADAM7 = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]


def image_data(w, h, ct, depth, interlace, mode, seed, filters):
    rng = Rng(seed)
    ch = CHANNELS[ct]
    pix = [[[sample(rng, x, y, c, depth, mode) for c in range(ch)] for x in range(w)] for y in range(h)]
    if ct == 3:
        for y in range(h):
            for x in range(w):
                pix[y][x] = [pix[y][x][0] % (1 << depth)]
    bpp = max(1, (ch * depth) // 8)
    raw = bytearray()
    fi = 0

    def rows(coords_rows):
        nonlocal fi
        prev = None
        for coords in coords_rows:
            vals = []
            for (x, y) in coords:
                vals += pix[y][x]
            r = pack_row(vals, depth)
            ftype = filters[fi % len(filters)]
            fi += 1
            raw.extend(filt(r, prev, ftype, bpp))
            prev = r

    if not interlace:
        rows([[(x, y) for x in range(w)] for y in range(h)])
    else:
        for (xs, ys, dx, dy) in ADAM7:
            prow = []
            for y in range(ys, h, dy):
                coords = [(x, y) for x in range(xs, w, dx)]
                if coords:
                    prow.append(coords)
            if prow and prow[0]:
                rows(prow)
    return bytes(raw)


def png(name, w, h, ct, depth, interlace=0, mode="grad", seed=1, filters=(0, 1, 2, 3, 4),
        before_plte=(), after_plte=(), palette=None, trns=None, idat_split=None, level=9,
        bad_idat_crc=False, after_idat=(), trailing=b"", truncate=None):
    data = b"\x89PNG\r\n\x1a\n"
    data += chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, depth, ct, 0, 0, interlace))
    for c in before_plte:
        data += c
    if palette is not None:
        data += chunk(b"PLTE", bytes(sum(([r, g, b] for (r, g, b) in palette), [])))
    for c in after_plte:
        data += c
    if trns is not None:
        data += chunk(b"tRNS", trns)
    z = zlib.compress(image_data(w, h, ct, depth, interlace, mode, seed, filters), level)
    parts = [z] if not idat_split else [z[i:i + idat_split] for i in range(0, len(z), idat_split)]
    for k, p in enumerate(parts):
        data += chunk(b"IDAT", p, crc_ok=not (bad_idat_crc and k == 0))
    for c in after_idat:
        data += c
    data += chunk(b"IEND", b"")
    data += trailing
    if truncate:
        data = data[:truncate]
    with open(os.path.join(OUT, name + ".png"), "wb") as f:
        f.write(data)


def pal(n):
    return [((i * 37) % 256, (i * 91 + 17) % 256, (255 - i * 13) % 256) for i in range(n)]


def gama(g):
    return chunk(b"gAMA", struct.pack(">I", g))


def icc_profile(cs=b"RGB ", good=True):
    tags = 1
    hdr = bytearray(128)
    body_len = 128 + 4 + 12 * tags + 20
    struct.pack_into(">I", hdr, 0, body_len)
    hdr[8] = 2  # version 2
    hdr[12:16] = b"mntr"
    hdr[16:20] = cs
    hdr[20:24] = b"XYZ "
    hdr[36:40] = b"acsp" if good else b"xxxx"
    hdr[68:80] = bytes([0x00, 0x00, 0xf6, 0xd6, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0xd3, 0x2d])
    tagtab = struct.pack(">I", tags) + b"wtpt" + struct.pack(">II", 128 + 4 + 12 * tags, 20)
    data = bytes(hdr) + tagtab + b"XYZ " + bytes(16)
    assert len(data) == body_len
    return data


def iccp(cs=b"RGB ", good=True):
    return chunk(b"iCCP", b"test profile\x00\x00" + zlib.compress(icc_profile(cs, good)))


# --- plain images (PNG copy candidates)
for d in (1, 2, 4, 8, 16):
    png("gray%d" % d, 40, 23, 0, d, mode="rand", seed=d)
png("rgb8", 41, 29, 2, 8, mode="rand", seed=3)
png("rgb16", 19, 13, 2, 16, mode="rand", seed=4)
for d in (1, 2, 4, 8):
    png("pal%d" % d, 32, 17, 3, d, palette=pal(1 << d), mode="rand", seed=10 + d)
png("pal8short", 20, 10, 3, 8, palette=pal(200), mode="grad", seed=7)

# --- tRNS
png("gray8trns", 25, 15, 0, 8, trns=struct.pack(">H", 7), mode="grad")
png("gray2trns", 25, 15, 0, 2, trns=struct.pack(">H", 2), mode="grad")
png("gray16trns", 25, 15, 0, 16, trns=struct.pack(">H", 20), mode="grad")
png("rgb8trns", 25, 15, 2, 8, trns=struct.pack(">HHH", 7, 14, 29), mode="grad")
png("rgb16trns", 9, 7, 2, 16, trns=struct.pack(">HHH", 7, 14, 29), mode="grad")
png("pal4trns", 31, 9, 3, 4, palette=pal(16), trns=bytes([0, 128, 255, 64, 32]), mode="rand", seed=5)
png("pal8trns", 31, 9, 3, 8, palette=pal(256), trns=bytes(range(0, 256, 3)), mode="rand", seed=6)

# --- alpha channels
png("ga8", 27, 19, 4, 8, mode="rand", seed=21)
png("ga16", 27, 19, 4, 16, mode="rand", seed=22)
png("rgba8", 27, 19, 6, 8, mode="rand", seed=23)
png("rgba16", 13, 11, 6, 16, mode="rand", seed=24)

# --- interlaced
for (n, w, h, ct, d, extra) in [
    ("igray8", 37, 23, 0, 8, {}), ("igray1", 40, 23, 0, 1, {}), ("igray4", 14, 9, 0, 4, {}),
    ("irgb8", 41, 29, 2, 8, {}), ("irgb16", 19, 13, 2, 16, {}),
    ("ipal8", 33, 17, 3, 8, {"palette": pal(256)}), ("ipal2", 32, 17, 3, 2, {"palette": pal(4)}),
    ("irgba8", 27, 19, 6, 8, {}), ("iga16", 27, 19, 4, 16, {}),
    ("ipal4trns", 31, 9, 3, 4, {"palette": pal(16), "trns": bytes([0, 128, 255])}),
    ("i1x1", 1, 1, 2, 8, {}), ("i2x5", 2, 5, 0, 8, {}), ("i3x3", 3, 3, 2, 8, {}), ("i5x2", 5, 2, 6, 8, {}),
    ("i8x9", 8, 9, 0, 2, {}), ("i7x9pad", 7, 9, 0, 2, {}),
]:
    png(n, w, h, ct, d, interlace=1, mode="rand", seed=w * h + d, **extra)

# --- gamma
png("gamma45455", 21, 11, 2, 8, before_plte=[gama(45455)], mode="rand", seed=31)
png("gamma100000", 21, 11, 2, 8, before_plte=[gama(100000)], mode="rand", seed=32)
png("gamma16", 21, 11, 0, 16, before_plte=[gama(45455)], mode="rand", seed=33)
png("gammapal", 21, 11, 3, 8, palette=pal(256), before_plte=[gama(50000)], mode="rand", seed=34)
png("gammaga8", 21, 11, 4, 8, before_plte=[gama(45455)], mode="rand", seed=35)
png("gammagray4", 22, 11, 0, 4, before_plte=[gama(45455)], mode="rand", seed=36)
png("gammagray2", 20, 11, 0, 2, before_plte=[gama(45455)], mode="rand", seed=37)
png("gammargba16", 21, 11, 6, 16, before_plte=[gama(70000)], mode="rand", seed=38)

# --- chunks that block the copy (or not)
png("srgb", 17, 9, 2, 8, before_plte=[chunk(b"sRGB", b"\x00")], mode="rand", seed=41)
png("chrm", 17, 9, 2, 8, before_plte=[chunk(b"cHRM", struct.pack(">8I", 31270, 32900, 64000, 33000, 30000, 60000, 15000, 6000))], mode="rand", seed=42)
png("iccp", 17, 9, 2, 8, before_plte=[iccp()], mode="rand", seed=43)
png("iccpbad", 17, 9, 2, 8, before_plte=[iccp(good=False)], mode="rand", seed=44)
png("iccpgray", 17, 9, 2, 8, before_plte=[iccp(cs=b"GRAY")], mode="rand", seed=45)
png("sbit", 17, 9, 2, 8, before_plte=[chunk(b"sBIT", b"\x05\x06\x05")], mode="rand", seed=46)
png("sbit16", 17, 9, 0, 16, before_plte=[chunk(b"sBIT", b"\x0c"), gama(45455)], mode="rand", seed=47)
png("bkgd", 17, 9, 2, 8, after_plte=[chunk(b"bKGD", struct.pack(">HHH", 1, 2, 3))], mode="rand", seed=48)
png("hist", 16, 9, 3, 2, palette=pal(4), after_plte=[chunk(b"hIST", struct.pack(">4H", 1, 2, 3, 4))], mode="rand", seed=49)
png("histbefore", 16, 9, 3, 2, before_plte=[chunk(b"hIST", b"")], palette=pal(4), mode="rand", seed=49)
png("splt", 17, 9, 2, 8, after_plte=[chunk(b"sPLT", b"pal\x00\x08" + bytes([1, 2, 3, 4, 0, 5]))], mode="rand", seed=50)
png("phys", 17, 9, 2, 8, before_plte=[chunk(b"pHYs", struct.pack(">IIB", 2835, 5669, 1))], mode="rand", seed=51)
png("physunk", 17, 9, 2, 8, before_plte=[chunk(b"pHYs", struct.pack(">IIB", 2835, 5669, 0))], mode="rand", seed=52)
png("text", 17, 9, 0, 8, before_plte=[chunk(b"tEXt", b"Title\x00hello"), chunk(b"zTXt", b"C\x00\x00" + zlib.compress(b"comment")),
                                       chunk(b"tIME", struct.pack(">HBBBBB", 2020, 1, 2, 3, 4, 5))],
    after_idat=[chunk(b"tEXt", b"After\x00idat")], mode="rand", seed=53)
png("badcrcanc", 17, 9, 2, 8, before_plte=[chunk(b"sRGB", b"\x00", crc_ok=False)], mode="rand", seed=54)
png("unknownanc", 17, 9, 2, 8, before_plte=[chunk(b"prVt", b"private data")], mode="rand", seed=55)
png("dupgama", 17, 9, 2, 8, before_plte=[gama(100000), gama(45455)], mode="rand", seed=56)
png("gamaafterplte", 17, 9, 3, 8, palette=pal(256), after_plte=[gama(45455)], mode="rand", seed=57)
png("trnsbadlen", 17, 9, 2, 8, trns=b"\x00\x01", mode="rand", seed=58)

# --- IDAT layouts, compression levels, filters
png("idat1", 64, 48, 2, 8, idat_split=1, mode="rand", seed=61)
png("idat100", 64, 48, 2, 8, idat_split=100, mode="rand", seed=62)
png("level0", 64, 48, 2, 8, level=0, mode="rand", seed=63)
png("filt0", 30, 30, 2, 8, filters=(0,), mode="grad")
png("filt4", 30, 30, 0, 16, filters=(4,), mode="grad")
png("big", 400, 300, 2, 8, mode="grad", filters=(1, 2, 3, 4, 0))
png("bigrgba", 300, 200, 6, 8, mode="rand", seed=64)
png("wide", 9000, 3, 2, 8, mode="grad")

# --- rows whose last byte is partly padding (libpng keeps those bits from pdfTeX's
# uninitialized row buffer): known differences, tested separately
png("pad_pal1", 33, 17, 3, 1, palette=pal(2), mode="rand", seed=81)
png("pad_gray4gamma", 21, 11, 0, 4, before_plte=[gama(45455)], mode="rand", seed=82)

# --- damaged
png("badidatcrc", 17, 9, 2, 8, bad_idat_crc=True, mode="rand", seed=71)
png("truncated", 64, 48, 2, 8, mode="rand", seed=72, truncate=300)
png("trailing", 17, 9, 2, 8, mode="rand", seed=73, trailing=b"garbage after IEND")
print("ok")
