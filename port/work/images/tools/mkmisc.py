"""Generate JPEG, JBIG2 and PDF test inputs (stdlib only).

pdfTeX never decodes JPEG or JBIG2 data, it only parses headers, so the files only need
correct headers/segment structure; the PDFs are hand-written with all the structures
pdftoepdf.cc copies.

usage: python -I mkmisc.py OUTDIR MWE_DIR
"""
import os
import struct
import sys
import zlib

OUT = sys.argv[1]
MWE = sys.argv[2]
os.makedirs(OUT, exist_ok=True)


def w(name, data):
    with open(os.path.join(OUT, name), "wb") as f:
        f.write(data)


# ----------------------------------------------------------------- JPEG
def seg(marker, payload):
    return b"\xff" + bytes([marker]) + struct.pack(">H", len(payload) + 2) + payload


def sof(marker, bits, h, wd, ncomp):
    comps = b"".join(bytes([i + 1, 0x11, 0]) for i in range(ncomp))
    return seg(marker, bytes([bits]) + struct.pack(">HH", h, wd) + bytes([ncomp]) + comps)


SCAN = seg(0xDA, b"\x01\x01\x00\x00\x3f\x00") + bytes(range(0, 250, 7)) * 3 + b"\xff\xd9"


def jfif(units, xd, yd):
    return seg(0xE0, b"JFIF\x00\x01\x02" + bytes([units]) + struct.pack(">HH", xd, yd) + b"\x00\x00")


def exif(be, xres, yres, unit, extra_zero=0):
    e = ">" if be else "<"
    tiff = (b"MM" if be else b"II") + struct.pack(e + "H", 42) + struct.pack(e + "I", 8)
    entries = [(282, 5, 1, None), (283, 5, 1, None), (296, 3, 1, unit)]
    ifd = struct.pack(e + "H", len(entries))
    data_off = 8 + 2 + 12 * len(entries) + 4
    extra = b""
    for tag, typ, cnt, val in entries:
        if typ == 5:
            num, den = (xres if tag == 282 else yres)
            ifd += struct.pack(e + "HHII", tag, typ, cnt, data_off + len(extra))
            extra += struct.pack(e + "II", num, den)
        else:
            ifd += struct.pack(e + "HHI", tag, typ, cnt) + struct.pack(e + "H", val) + b"\x00\x00"
    ifd += struct.pack(e + "I", 0)
    return seg(0xE1, b"Exif\x00" + b"\x00" * extra_zero + tiff + ifd + extra)


def jpeg(name, *segs):
    w(name, b"\xff\xd8" + b"".join(segs) + SCAN)


DQT = seg(0xDB, b"\x00" + bytes(range(1, 65)))
DHT = seg(0xC4, b"\x00" + bytes([0, 1, 5, 1, 1, 1, 1, 1, 1] + [0] * 7) + bytes(range(12)))
jpeg("j_rgb_jfif72.jpg", jfif(1, 72, 72), DQT, sof(0xC0, 8, 30, 40, 3), DHT)
jpeg("j_gray_jfif_cm.jpg", jfif(2, 100, 50), DQT, sof(0xC0, 8, 30, 40, 1), DHT)
jpeg("j_cmyk.jpg", jfif(0, 300, 300), DQT, sof(0xC0, 8, 20, 10, 4), DHT)
jpeg("j_prog.jpg", jfif(1, 150, 0), DQT, sof(0xC2, 8, 30, 40, 3), DHT)
jpeg("j_sof1_12bit.jpg", jfif(1, 0, 96), DQT, sof(0xC1, 12, 7, 9, 1), DHT)
jpeg("j_exif_be.jpg", exif(True, (300, 1), (600, 2), 2), DQT, sof(0xC0, 8, 30, 40, 3), DHT)
jpeg("j_exif_le_cm.jpg", exif(False, (118, 1), (59, 1), 3), DQT, sof(0xC0, 8, 30, 40, 3), DHT)
jpeg("j_exif_pad.jpg", exif(True, (72, 1), (72, 1), 2, extra_zero=3), DQT, sof(0xC0, 8, 30, 40, 3), DHT)
jpeg("j_noapp.jpg", DQT, sof(0xC0, 8, 33, 44, 3), DHT)
jpeg("j_app1other.jpg", seg(0xE1, b"http://ns.adobe.com/xap/1.0/\x00<x/>"), DQT, sof(0xC0, 8, 33, 44, 3), DHT)
jpeg("j_com_app14.jpg", seg(0xFE, b"a comment"), seg(0xEE, b"Adobe\x00\x64\x00\x00\x00\x00\x01"), DQT, sof(0xC0, 8, 5, 6, 3), DHT)
jpeg("j_bad_sof5.jpg", jfif(1, 72, 72), sof(0xC5, 8, 5, 6, 3))
jpeg("j_bad_cs2.jpg", jfif(1, 72, 72), sof(0xC0, 8, 5, 6, 2))
w("j_bad_header.jpg", b"\xff\xd9\xff\xd8" + b"\x00" * 20)
w("j_bad_nomarker.jpg", b"\xff\xd8" + b"\x00" * 30)
w("j_bad_eof.jpg", b"\xff\xd8" + jfif(1, 72, 72) + DQT)
# extension trick: a JPEG named .png is detected by its header
jpeg("j_named_png.png", jfif(1, 72, 72), DQT, sof(0xC0, 8, 30, 40, 3), DHT)

# ----------------------------------------------------------------- JBIG2
JB2_ID = b"\x97JB2\r\n\x1a\n"


def jseg(num, typ, page, data, refs=(), page4=False, retain=0):
    flags = typ | (0x40 if page4 else 0)
    hdr = struct.pack(">I", num) + bytes([flags])
    hdr += bytes([(len(refs) << 5) | retain])
    for r in refs:
        hdr += bytes([r]) if num <= 256 else struct.pack(">H", r) if num <= 65536 else struct.pack(">I", r)
    hdr += struct.pack(">I", page) if page4 else bytes([page])
    hdr += struct.pack(">I", len(data))
    return hdr, data


def pageinfo(wd, ht, xres=0, yres=0, striped=False, sh=0):
    return struct.pack(">IIII", wd, ht, xres, yres) + bytes([0]) + struct.pack(">H", (0x8000 | sh) if striped else 0)


def jbig2(name, segs, sequential=True, npages=None):
    flags = 0x01 if sequential else 0x00
    if npages is None:
        flags |= 0x02
    out = JB2_ID + bytes([flags]) + (struct.pack(">I", npages) if npages is not None else b"")
    if sequential:
        for h, d in segs:
            out += h + d
    else:
        out += b"".join(h for h, d in segs) + b"".join(d for h, d in segs)
    w(name, out)


GEN = bytes(range(20, 90))
jbig2("jb_seq.jb2", [
    jseg(0, 48, 1, pageinfo(64, 32, 2835, 2835)),
    jseg(1, 38, 1, GEN),
    jseg(2, 49, 1, b""),
    jseg(3, 51, 0, b""),
], npages=1)
jbig2("jb_globals.jb2", [
    jseg(0, 0, 0, b"SYMDICT" * 3),
    jseg(1, 48, 1, pageinfo(100, 40, 11811, 11811)),
    jseg(2, 6, 1, b"TEXTREGION1", refs=(0,)),
    jseg(3, 49, 1, b""),
    jseg(4, 48, 2, pageinfo(50, 80)),
    jseg(5, 6, 2, b"TEXTREGION2", refs=(0,)),
    jseg(6, 49, 2, b""),
    jseg(7, 51, 0, b""),
], npages=2)
jbig2("jb_random.jb2", [
    jseg(0, 0, 0, b"GLOBAL"),
    jseg(1, 48, 1, pageinfo(33, 44, 3937, 3937)),
    jseg(2, 38, 1, GEN[:30]),
    jseg(3, 49, 1, b""),
    jseg(4, 48, 2, pageinfo(10, 0, striped=True, sh=8)),
    jseg(5, 38, 2, GEN[:7]),
    jseg(6, 50, 2, struct.pack(">I", 21)),
    jseg(7, 49, 2, b""),
    jseg(8, 51, 0, b""),
], sequential=False, npages=2)
jbig2("jb_noeof.jb2", [
    jseg(0, 48, 1, pageinfo(8, 8)),
    jseg(1, 38, 1, GEN[:5]),
    jseg(2, 49, 1, b""),
], npages=None)
jbig2("jb_page4.jb2", [
    jseg(0, 48, 1, pageinfo(16, 16), page4=True),
    jseg(1, 38, 1, GEN[:9], page4=True),
    jseg(2, 49, 1, b"", page4=True),
], npages=1)
jbig2("jb_badtype.jb2", [jseg(0, 60, 1, b"")], npages=1)


# ----------------------------------------------------------------- PDF
class Pdf:
    def __init__(self, version="1.4"):
        self.objs = {}
        self.version = version

    def add(self, num, body):
        self.objs[num] = body

    def stream(self, num, dict_extra, data):
        self.objs[num] = b"<< " + dict_extra + b" /Length %d >>\nstream\n" % len(data) + data + b"\nendstream"

    def write(self, name, root=1, info=None, extra_trailer=b""):
        out = b"%PDF-" + self.version.encode() + b"\n%\xe2\xe3\xcf\xd3\n"
        offs = {}
        for n in sorted(self.objs):
            offs[n] = len(out)
            out += b"%d 0 obj\n" % n + self.objs[n] + b"\nendobj\n"
        size = max(self.objs) + 1
        x = len(out)
        out += b"xref\n0 %d\n0000000000 65535 f \n" % size
        for n in range(1, size):
            out += (b"%010d 00000 n \n" % offs[n]) if n in offs else b"0000000000 65535 f \n"
        out += b"trailer\n<< /Size %d /Root %d 0 R" % (size, root)
        if info:
            out += b" /Info %d 0 R" % info
        out += extra_trailer + b" >>\nstartxref\n%d\n%%%%EOF\n" % x
        w(name, out)


# a feature-rich 3 page document
p = Pdf("1.4")
p.add(1, b"<< /Type /Catalog /Pages 2 0 R /Dests << /second [4 0 R /XYZ 0 0 null] /third [5 0 R /Fit] >> >>")
p.add(2, b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 200 100] /Rotate 0 /Resources << /ProcSet [/PDF /ImageB] >> >>")
p.add(3, b"<< /Type /Page /Parent 2 0 R /Contents 6 0 R /CropBox [10 5 190.5 95.25] /BleedBox [5 2 195 98] "
         b"/TrimBox [12 7 188 93] /ArtBox [20 10 180 90] /Resources 7 0 R /LastModified (D:20200101000000Z) "
         b"/PieceInfo << /App << /LastModified (D:2020) /Private 1.5 >> >> /Metadata 12 0 R "
         b"/Group << /S /Transparency /CS /DeviceRGB /I true >> >>")
p.add(4, b"<< /Type /Page /Parent 2 0 R /Contents [8 0 R 9 0 R] /Rotate 90 /Resources << /ExtGState << /G0 10 0 R >> /ProcSet [/PDF] >> >>")
p.add(5, b"<< /Type /Page /Parent 2 0 R /Rotate -90 /MediaBox [0 0 300 150.123456789] /Resources << /XObject << /X0 11 0 R >> >> /Contents 13 0 R >>")
p.stream(6, b"/Filter /FlateDecode", zlib.compress(b"q 1 0 0 RG 0 0 1 rg 20 20 100 50 re B Q\n/G0 gs\n"))
p.add(7, b"<< /ProcSet [/PDF /Text] /ExtGState << /G0 10 0 R /G1 << /CA 0.5 /ca .25 /BM /Multiply >> >> "
         b"/ColorSpace << /CS0 [/Indexed /DeviceRGB 1 <ff0000 00ff00>] /CS1 [/CalRGB << /WhitePoint [0.9505 1 1.089] >>] >> "
         b"/Pattern << /P0 14 0 R >> /Shading << /Sh0 << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 1 0] /Function 15 0 R >> >> "
         b"/Properties << /MC0 << /Name (L\\(a\\)y\\\\er\\001\\377) /Weird#20Key true /Hex <00ff10> /Neg -3 /Real -0.000001 /Big 12345.6789012 >> >> "
         b"/Subtype /NotADict /Scalar 42 >>")
p.stream(8, b"", b"0 g 0 0 10 10 re f")
p.stream(9, b"/Filter /FlateDecode", zlib.compress(b"1 0 0 rg 50 50 20 20 re f"))
p.add(10, b"<< /Type /ExtGState /LW 2 /D [[3 2] 0] /SA false /Font null >>")
p.stream(11, b"/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << >>", b"0 0 m 10 10 l S")
p.stream(12, b"/Type /Metadata /Subtype /XML", b"<?xpacket begin='' id='W5M0MpCehiHzreSzNTczkc9d'?><x:xmpmeta/>")
p.stream(13, b"", b"0 0 1 rg 10 10 50 50 re f")
p.stream(14, b"/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 4 4] /XStep 4 /YStep 4 /Resources << >>", b"0 0 2 2 re f")
p.add(15, b"<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>")
p.add(16, b"<< /Title (Test \\(PDF\\)) /Producer (mkmisc.py) /CreationDate (D:20200101) >>")
p.write("p_rich.pdf", info=16)

# PDF version 1.6 header, no Resources on the page, empty contents, /Group as a reference
p = Pdf("1.6")
p.add(1, b"<< /Type /Catalog /Pages 2 0 R >>")
p.add(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
p.add(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 50 60] /Group 4 0 R >>")
p.add(4, b"<< /S /Transparency /CS /DeviceGray >>")
p.write("p_v16.pdf")

# page tree inheritance of Resources and an indirect Length and Filter array
p = Pdf("1.3")
p.add(1, b"<< /Type /Catalog /Pages 2 0 R >>")
p.add(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /ProcSet 6 0 R /XObject << /Im1 7 0 R >> >> /MediaBox [0 0 30 30] >>")
p.add(3, b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>")
data = zlib.compress(b"q 20 0 0 20 5 5 cm /Im1 Do Q")
import base64
a85 = base64.a85encode(data) + b"~>"
p.add(4, b"<< /Length 5 0 R /Filter [/ASCII85Decode /FlateDecode] >>\nstream\n" + a85 + b"\nendstream")
p.add(5, b"%d" % len(a85))
p.add(6, b"[/PDF /ImageC]")
p.stream(7, b"/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8", bytes(range(12)))
p.write("p_inherit.pdf")

# copy the mwe example PDFs (TikZ/LaTeX output with embedded Type 1 fonts)
for f in ("example-image.pdf", "example-image-a.pdf", "example-grid-100x100bp.pdf"):
    src = os.path.join(MWE, f)
    if os.path.exists(src):
        with open(src, "rb") as fi:
            w("mwe-" + f, fi.read())
print("ok")


# ----------------------------------------------------------------- %.8f rounding ties
p = Pdf("1.4")
p.add(1, b"<< /Type /Catalog /Pages 2 0 R >>")
p.add(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
p.add(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0.001953125 -0.005859375 100.013671875 50.123046875] /Rotate 90 "
         b"/Contents 4 0 R /Resources << /Properties << /P [0.0000005 0.0000015 -0.0000005 1.9999995 123456.0000005 0.5 0.25 0.125] >> >> >>")
p.stream(4, b"", b"0 0 m 10 10 l S")
p.write("p_ties.pdf")


# ----------------------------------------------------------------- RC4-40 (V1 R2) encrypted PDF
import hashlib


def rc4(key, data):
    S = list(range(256))
    j = 0
    for i in range(256):
        j = (j + S[i] + key[i % len(key)]) % 256
        S[i], S[j] = S[j], S[i]
    out = bytearray()
    i = j = 0
    for c in data:
        i = (i + 1) % 256
        j = (j + S[i]) % 256
        S[i], S[j] = S[j], S[i]
        out.append(c ^ S[(S[i] + S[j]) % 256])
    return bytes(out)


PAD = bytes.fromhex("28BF4E5E4E758A4164004E56FFFA01082E2E00B6D0683E802F0CA9FE6453697A")
ID0 = b"0123456789abcdef"
P = -44  # print allowed
okey = hashlib.md5(PAD).digest()[:5]           # empty owner password
O = rc4(okey, PAD)
key = hashlib.md5(PAD + O + struct.pack("<i", P) + ID0).digest()[:5]
U = rc4(key, PAD)


def objkey(num, gen):
    return hashlib.md5(key + struct.pack("<I", num)[:3] + struct.pack("<I", gen)[:2]).digest()[:10]


def pstr(num, s):
    return b"<" + rc4(objkey(num, 0), s).hex().encode() + b">"


p = Pdf("1.3")
p.add(1, b"<< /Type /Catalog /Pages 2 0 R >>")
p.add(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
p.add(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 80 40] /Contents 4 0 R /Resources << /Properties << /T "
         + pstr(3, b"secret (text)") + b" >> /XObject << /X 5 0 R >> >> >>")
content = zlib.compress(b"1 0 0 rg 5 5 30 30 re f /X Do")
p.add(4, b"<< /Length %d /Filter /FlateDecode >>\nstream\n" % len(content) + rc4(objkey(4, 0), content) + b"\nendstream")
form = b"0 0 1 rg 40 5 30 30 re f"
p.add(5, b"<< /Type /XObject /Subtype /Form /BBox [0 0 80 40] /Length %d >>\nstream\n" % len(form) + rc4(objkey(5, 0), form) + b"\nendstream")
p.add(6, b"<< /Filter /Standard /V 1 /R 2 /O <" + O.hex().encode() + b"> /U <" + U.hex().encode() + b"> /P %d >>" % P)
p.add(7, b"<< /Title " + pstr(7, b"encrypted title") + b" >>")
p.write("p_rc4.pdf", info=7, extra_trailer=b" /Encrypt 6 0 R /ID [<" + ID0.hex().encode() + b"> <" + ID0.hex().encode() + b">]")
print("ok2")
