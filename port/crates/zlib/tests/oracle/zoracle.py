"""Replay zlib call scripts against the real C zlib 1.3.2 through ctypes.

Usage: python -I zoracle.py <script.txt> <outdir>

The script is produced by the Rust test harness (tests/common/mod.rs), which
also executes it against texrs-zlib and writes its own trace. This program
executes the same explicit calls (same avail_in / avail_out / flush values)
against C zlib and writes <outdir>/oracle.trace plus one binary blob per case
(<outdir>/<case>.c.bin) with every byte the calls produced.

The C library is loaded from $TEXRS_ZLIB_ORACLE_DLL, defaulting to Git for
Windows' zlib1.dll; the program refuses to run unless zlibVersion() is 1.3.2.
"""
import ctypes
import os
import sys
import zlib as pyzlib

DEFAULT_DLLS = [
    r"C:\Program Files\Git\mingw64\bin\zlib1.dll",
    r"C:\Program Files\Git\mingw64\libexec\git-core\zlib1.dll",
]


def load_dll():
    cands = [os.environ["TEXRS_ZLIB_ORACLE_DLL"]] if os.environ.get("TEXRS_ZLIB_ORACLE_DLL") else DEFAULT_DLLS
    for p in cands:
        if os.path.exists(p):
            d = ctypes.CDLL(p)
            d.zlibVersion.restype = ctypes.c_char_p
            v = d.zlibVersion().decode()
            if v != "1.3.2":
                sys.exit(f"oracle DLL {p} is zlib {v}, need 1.3.2")
            return d, p
    sys.exit("no zlib 1.3.2 DLL found; set TEXRS_ZLIB_ORACLE_DLL")


Z, DLL_PATH = load_dll()


class ZStream(ctypes.Structure):
    _fields_ = [
        ("next_in", ctypes.c_void_p), ("avail_in", ctypes.c_uint), ("total_in", ctypes.c_ulong),
        ("next_out", ctypes.c_void_p), ("avail_out", ctypes.c_uint), ("total_out", ctypes.c_ulong),
        ("msg", ctypes.c_char_p), ("state", ctypes.c_void_p), ("zalloc", ctypes.c_void_p),
        ("zfree", ctypes.c_void_p), ("opaque", ctypes.c_void_p), ("data_type", ctypes.c_int),
        ("adler", ctypes.c_ulong), ("reserved", ctypes.c_ulong),
    ]


class GzHeader(ctypes.Structure):
    _fields_ = [
        ("text", ctypes.c_int), ("time", ctypes.c_ulong), ("xflags", ctypes.c_int), ("os", ctypes.c_int),
        ("extra", ctypes.c_void_p), ("extra_len", ctypes.c_uint), ("extra_max", ctypes.c_uint),
        ("name", ctypes.c_void_p), ("name_max", ctypes.c_uint),
        ("comment", ctypes.c_void_p), ("comm_max", ctypes.c_uint),
        ("hcrc", ctypes.c_int), ("done", ctypes.c_int),
    ]


SZ = ctypes.sizeof(ZStream)
VER = b"1.3.2"
Z.deflateBound.restype = ctypes.c_ulong
Z.deflateBound.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
Z.compressBound.restype = ctypes.c_ulong
Z.compressBound.argtypes = [ctypes.c_ulong]
Z.adler32.restype = ctypes.c_ulong
Z.adler32.argtypes = [ctypes.c_ulong, ctypes.c_void_p, ctypes.c_uint]
Z.crc32.restype = ctypes.c_ulong
Z.crc32.argtypes = [ctypes.c_ulong, ctypes.c_void_p, ctypes.c_uint]
Z.adler32_combine64.restype = ctypes.c_ulong
Z.adler32_combine64.argtypes = [ctypes.c_ulong, ctypes.c_ulong, ctypes.c_longlong]
Z.crc32_combine64.restype = ctypes.c_ulong
Z.crc32_combine64.argtypes = [ctypes.c_ulong, ctypes.c_ulong, ctypes.c_longlong]
Z.inflateMark.restype = ctypes.c_long
Z.inflateCodesUsed.restype = ctypes.c_ulong
Z.gzopen.restype = ctypes.c_void_p
Z.gzopen.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
for fn in ("gzwrite", "gzread", "gzsetparams", "gzflush", "gzputc", "gzputs", "gzgetc", "gzungetc",
           "gzeof", "gzdirect", "gzclose", "gzclose_w", "gzclose_r", "gzbuffer", "gzprintf"):
    getattr(Z, fn).restype = ctypes.c_int
Z.gzwrite.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint]
Z.gzread.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint]
Z.gzsetparams.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int]
Z.gzflush.argtypes = [ctypes.c_void_p, ctypes.c_int]
Z.gzputc.argtypes = [ctypes.c_void_p, ctypes.c_int]
Z.gzputs.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
Z.gzprintf.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p]
Z.gzgetc.argtypes = [ctypes.c_void_p]
Z.gzungetc.argtypes = [ctypes.c_int, ctypes.c_void_p]
Z.gzgets.restype = ctypes.c_void_p
Z.gzgets.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int]
Z.gzeof.argtypes = [ctypes.c_void_p]
Z.gzdirect.argtypes = [ctypes.c_void_p]
Z.gzclose.argtypes = [ctypes.c_void_p]
Z.gzbuffer.argtypes = [ctypes.c_void_p, ctypes.c_uint]
Z.gzseek64.restype = ctypes.c_longlong
Z.gzseek64.argtypes = [ctypes.c_void_p, ctypes.c_longlong, ctypes.c_int]
Z.gztell64.restype = ctypes.c_longlong
Z.gztell64.argtypes = [ctypes.c_void_p]
Z.gzerror.restype = ctypes.c_char_p
Z.gzerror.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int)]


def msgstr(m):
    if m is None:
        return "-"
    return m.decode("latin-1").replace(" ", "_") or "''"


def crc(b):
    return pyzlib.crc32(b) & 0xFFFFFFFF


class Runner:
    def __init__(self, outdir):
        self.outdir = outdir
        self.data = {}
        self.trace = open(os.path.join(outdir, "oracle.trace"), "w", encoding="ascii", newline="\n")

    def buf(self, did):
        """ctypes buffer + python bytes for a data blob."""
        return self.data[did]

    def run_case(self, name, cmds):
        out = bytearray()
        w = self.trace.write
        w(f"case {name}\n")
        ds = None  # deflate stream
        istrm = None  # inflate stream
        cur = None  # current input data id
        keep = []  # keep ctypes buffers alive
        hdr = None
        ihdr = None
        ihdr_bufs = None
        gz = None
        gzpath = os.path.join(self.outdir, f"{name}.c.gz").encode()
        for t in cmds:
            op = t[0]
            a = t[1:]
            if op == "use":
                cur = a[0]
            elif op == "dinit":
                ds = ZStream()
                r = Z.deflateInit2_(ctypes.byref(ds), int(a[0]), int(a[1]), int(a[2]), int(a[3]), int(a[4]), VER, SZ)
                w(f"dinit {r} {msgstr(ds.msg)}\n")
                if r != 0:
                    ds = None
            elif op in ("d", "dparams"):
                off, ain, aout = int(a[0]), int(a[1]), int(a[2])
                cb, _ = self.buf(cur)
                ob = ctypes.create_string_buffer(max(aout, 1))
                ds.next_in = ctypes.addressof(cb) + off if ain or off else ctypes.addressof(cb) + off
                ds.avail_in = ain
                ds.next_out = ctypes.addressof(ob)
                ds.avail_out = aout
                if op == "d":
                    r = Z.deflate(ctypes.byref(ds), int(a[3]))
                else:
                    r = Z.deflateParams(ctypes.byref(ds), int(a[3]), int(a[4]))
                prod = ob.raw[: aout - ds.avail_out]
                out += prod
                w(f"{op} {r} {ds.avail_in} {ds.avail_out} {ds.total_in} {ds.total_out} {ds.adler} "
                  f"{ds.data_type} {msgstr(ds.msg)} {crc(prod)}\n")
            elif op == "dreset":
                r = Z.deflateReset(ctypes.byref(ds))
                w(f"dreset {r} {ds.total_in} {ds.total_out} {ds.adler} {ds.data_type} {msgstr(ds.msg)}\n")
            elif op == "dresetkeep":
                r = Z.deflateResetKeep(ctypes.byref(ds))
                w(f"dresetkeep {r} {ds.total_in} {ds.total_out} {ds.adler}\n")
            elif op == "ddict":
                cb, pb = self.buf(a[0])
                off, ln = int(a[1]), int(a[2])
                r = Z.deflateSetDictionary(ctypes.byref(ds), ctypes.c_void_p(ctypes.addressof(cb) + off), ln)
                w(f"ddict {r} {ds.adler} {ds.total_in} {ds.total_out}\n")
            elif op == "dgetdict":
                b = ctypes.create_string_buffer(32768)
                n = ctypes.c_uint(0)
                r = Z.deflateGetDictionary(ctypes.byref(ds), b, ctypes.byref(n))
                w(f"dgetdict {r} {n.value} {crc(b.raw[:n.value])}\n")
            elif op == "dprime":
                r = Z.deflatePrime(ctypes.byref(ds), int(a[0]), int(a[1]))
                w(f"dprime {r}\n")
            elif op == "dtune":
                r = Z.deflateTune(ctypes.byref(ds), int(a[0]), int(a[1]), int(a[2]), int(a[3]))
                w(f"dtune {r}\n")
            elif op == "dbound":
                v = Z.deflateBound(ctypes.byref(ds), int(a[0]))
                w(f"dbound {v}\n")
            elif op == "dbound0":
                v = Z.deflateBound(None, int(a[0]))
                w(f"dbound0 {v}\n")
            elif op == "dpending":
                p = ctypes.c_uint(0)
                b = ctypes.c_int(0)
                r = Z.deflatePending(ctypes.byref(ds), ctypes.byref(p), ctypes.byref(b))
                w(f"dpending {r} {p.value} {b.value}\n")
            elif op == "dused":
                b = ctypes.c_int(0)
                r = Z.deflateUsed(ctypes.byref(ds), ctypes.byref(b))
                w(f"dused {r} {b.value}\n")
            elif op == "dheader":
                # text time os xflags(unused) hcrc name comment extra (hex or -)
                hdr = GzHeader()
                hdr.text = int(a[0])
                hdr.time = int(a[1])
                hdr.os = int(a[2])
                hdr.hcrc = int(a[4])
                bufs = []
                for fld, val in (("name", a[5]), ("comment", a[6])):
                    if val != "-":
                        bb = ctypes.create_string_buffer(bytes.fromhex(val) + b"\0")
                        bufs.append(bb)
                        setattr(hdr, fld, ctypes.addressof(bb))
                if a[7] != "-":
                    ex = bytes.fromhex(a[7])
                    bb = ctypes.create_string_buffer(ex, max(len(ex), 1))
                    bufs.append(bb)
                    hdr.extra = ctypes.addressof(bb)
                    hdr.extra_len = len(ex)
                keep.append(bufs)
                r = Z.deflateSetHeader(ctypes.byref(ds), ctypes.byref(hdr))
                w(f"dheader {r}\n")
            elif op == "dcopy":
                ds2 = ZStream()
                r = Z.deflateCopy(ctypes.byref(ds2), ctypes.byref(ds))
                Z.deflateEnd(ctypes.byref(ds))
                keep.append(ds)
                ds = ds2
                w(f"dcopy {r}\n")
            elif op == "dend":
                r = Z.deflateEnd(ctypes.byref(ds))
                w(f"dend {r}\n")
                ds = None
            elif op == "iinit":
                istrm = ZStream()
                r = Z.inflateInit2_(ctypes.byref(istrm), int(a[0]), VER, SZ)
                w(f"iinit {r} {msgstr(istrm.msg)}\n")
            elif op == "i":
                off, ain, aout = int(a[0]), int(a[1]), int(a[2])
                cb, _ = self.buf(cur)
                ob = ctypes.create_string_buffer(max(aout, 1))
                istrm.next_in = ctypes.addressof(cb) + off
                istrm.avail_in = ain
                istrm.next_out = ctypes.addressof(ob)
                istrm.avail_out = aout
                r = Z.inflate(ctypes.byref(istrm), int(a[3]))
                prod = ob.raw[: aout - istrm.avail_out]
                out += prod
                w(f"i {r} {istrm.avail_in} {istrm.avail_out} {istrm.total_in} {istrm.total_out} "
                  f"{istrm.adler} {istrm.data_type} {msgstr(istrm.msg)} {crc(prod)}\n")
            elif op == "ireset":
                r = Z.inflateReset(ctypes.byref(istrm))
                w(f"ireset {r} {istrm.total_in} {istrm.total_out} {istrm.adler}\n")
            elif op == "iresetkeep":
                r = Z.inflateResetKeep(ctypes.byref(istrm))
                w(f"iresetkeep {r} {istrm.adler}\n")
            elif op == "ireset2":
                r = Z.inflateReset2(ctypes.byref(istrm), int(a[0]))
                w(f"ireset2 {r} {istrm.adler}\n")
            elif op == "idict":
                cb, pb = self.buf(a[0])
                off, ln = int(a[1]), int(a[2])
                r = Z.inflateSetDictionary(ctypes.byref(istrm), ctypes.c_void_p(ctypes.addressof(cb) + off), ln)
                w(f"idict {r}\n")
            elif op == "igetdict":
                b = ctypes.create_string_buffer(32768)
                n = ctypes.c_uint(0)
                r = Z.inflateGetDictionary(ctypes.byref(istrm), b, ctypes.byref(n))
                w(f"igetdict {r} {n.value} {crc(b.raw[:n.value])}\n")
            elif op == "isync":
                off, ain = int(a[0]), int(a[1])
                cb, _ = self.buf(cur)
                istrm.next_in = ctypes.addressof(cb) + off
                istrm.avail_in = ain
                r = Z.inflateSync(ctypes.byref(istrm))
                w(f"isync {r} {istrm.avail_in} {istrm.total_in} {istrm.total_out}\n")
            elif op == "isyncpoint":
                w(f"isyncpoint {Z.inflateSyncPoint(ctypes.byref(istrm))}\n")
            elif op == "iprime":
                w(f"iprime {Z.inflatePrime(ctypes.byref(istrm), int(a[0]), int(a[1]))}\n")
            elif op == "imark":
                w(f"imark {Z.inflateMark(ctypes.byref(istrm))}\n")
            elif op == "icodes":
                w(f"icodes {Z.inflateCodesUsed(ctypes.byref(istrm))}\n")
            elif op == "ivalidate":
                w(f"ivalidate {Z.inflateValidate(ctypes.byref(istrm), int(a[0]))}\n")
            elif op == "igethdr":
                ihdr = GzHeader()
                em, nm, cm = int(a[0]), int(a[1]), int(a[2])
                ihdr_bufs = [ctypes.create_string_buffer(max(em, 1)), ctypes.create_string_buffer(max(nm, 1)),
                             ctypes.create_string_buffer(max(cm, 1))]
                ihdr.extra = ctypes.addressof(ihdr_bufs[0])
                ihdr.extra_max = em
                ihdr.name = ctypes.addressof(ihdr_bufs[1])
                ihdr.name_max = nm
                ihdr.comment = ctypes.addressof(ihdr_bufs[2])
                ihdr.comm_max = cm
                keep.append(ihdr_bufs)
                r = Z.inflateGetHeader(ctypes.byref(istrm), ctypes.byref(ihdr))
                w(f"igethdr {r}\n")
            elif op == "ihdr":
                h = ihdr
                ex = ihdr_bufs[0].raw[: h.extra_max] if h.extra else b""
                nm = ihdr_bufs[1].raw[: h.name_max] if h.name else b""
                cm = ihdr_bufs[2].raw[: h.comm_max] if h.comment else b""
                w(f"ihdr {h.text} {h.time} {h.xflags} {h.os} {h.extra_len} {h.hcrc} {h.done} "
                  f"{int(bool(h.extra))} {int(bool(h.name))} {int(bool(h.comment))} {crc(ex)} {crc(nm)} {crc(cm)}\n")
            elif op == "iend":
                w(f"iend {Z.inflateEnd(ctypes.byref(istrm))}\n")
                istrm = None
            elif op == "adler":
                cb, pb = self.buf(a[0])
                off, ln, init = int(a[1]), int(a[2]), int(a[3])
                w(f"adler {Z.adler32(init, ctypes.addressof(cb) + off, ln)}\n")
            elif op == "crc":
                cb, pb = self.buf(a[0])
                off, ln, init = int(a[1]), int(a[2]), int(a[3])
                w(f"crc {Z.crc32(init, ctypes.addressof(cb) + off, ln)}\n")
            elif op == "adlercomb":
                w(f"adlercomb {Z.adler32_combine64(int(a[0]), int(a[1]), int(a[2]))}\n")
            elif op == "crccomb":
                w(f"crccomb {Z.crc32_combine64(int(a[0]), int(a[1]), int(a[2]))}\n")
            elif op == "cbound":
                w(f"cbound {Z.compressBound(int(a[0]))}\n")
            elif op == "compress2":
                cb, pb = self.buf(a[0])
                off, ln, dl, lvl = int(a[1]), int(a[2]), int(a[3]), int(a[4])
                ob = ctypes.create_string_buffer(max(dl, 1))
                dlen = ctypes.c_ulong(dl)
                r = Z.compress2(ob, ctypes.byref(dlen), ctypes.c_void_p(ctypes.addressof(cb) + off), ctypes.c_ulong(ln), lvl)
                prod = ob.raw[: dlen.value]
                out += prod
                w(f"compress2 {r} {dlen.value} {crc(prod)}\n")
            elif op == "uncompress2":
                cb, pb = self.buf(a[0])
                off, ln, dl = int(a[1]), int(a[2]), int(a[3])
                ob = ctypes.create_string_buffer(max(dl, 1))
                dlen = ctypes.c_ulong(dl)
                slen = ctypes.c_ulong(ln)
                r = Z.uncompress2(ob if dl else None, ctypes.byref(dlen),
                                  ctypes.c_void_p(ctypes.addressof(cb) + off), ctypes.byref(slen))
                prod = ob.raw[: dlen.value]
                out += prod
                w(f"uncompress2 {r} {dlen.value} {slen.value} {crc(prod)}\n")
            elif op == "gzwopen":
                gz = Z.gzopen(gzpath, a[0].encode())
                w(f"gzwopen {int(bool(gz))}\n")
            elif op == "gzropen":
                path = self.paths[a[0]].encode()
                gz = Z.gzopen(path, a[1].encode())
                w(f"gzropen {int(bool(gz))}\n")
            elif op == "gzbuffer":
                w(f"gzbuffer {Z.gzbuffer(gz, int(a[0]))}\n")
            elif op == "gzw":
                cb, pb = self.buf(a[0])
                off, ln = int(a[1]), int(a[2])
                w(f"gzw {Z.gzwrite(gz, ctypes.addressof(cb) + off, ln)}\n")
            elif op == "gzsetparams":
                w(f"gzsetparams {Z.gzsetparams(gz, int(a[0]), int(a[1]))}\n")
            elif op == "gzflush":
                w(f"gzflush {Z.gzflush(gz, int(a[0]))}\n")
            elif op == "gzputc":
                w(f"gzputc {Z.gzputc(gz, int(a[0]))}\n")
            elif op == "gzputs":
                s = bytes.fromhex(a[0]) if a[0] != "-" else b""
                w(f"gzputs {Z.gzputs(gz, s)}\n")
            elif op == "gzprintf":
                s = bytes.fromhex(a[0]) if a[0] != "-" else b""
                w(f"gzprintf {Z.gzprintf(gz, b'%s', s)}\n")
            elif op == "gzseek":
                w(f"gzseek {Z.gzseek64(gz, int(a[0]), int(a[1]))}\n")
            elif op == "gztell":
                w(f"gztell {Z.gztell64(gz)}\n")
            elif op == "gzr":
                n = int(a[0])
                b = ctypes.create_string_buffer(max(n, 1))
                r = Z.gzread(gz, b, n)
                prod = b.raw[: max(r, 0)]
                out += prod
                w(f"gzr {r} {crc(prod)}\n")
            elif op == "gzgetc":
                w(f"gzgetc {Z.gzgetc(gz)}\n")
            elif op == "gzungetc":
                w(f"gzungetc {Z.gzungetc(int(a[0]), gz)}\n")
            elif op == "gzgets":
                n = int(a[0])
                b = ctypes.create_string_buffer(max(n, 1))
                p = Z.gzgets(gz, b, n)
                if p:
                    s = b.raw[: b.raw.index(b"\0")]
                    out += s
                    w(f"gzgets {len(s)} {crc(s)}\n")
                else:
                    w("gzgets -1 0\n")
            elif op == "gzeof":
                w(f"gzeof {Z.gzeof(gz)}\n")
            elif op == "gzdirect":
                w(f"gzdirect {Z.gzdirect(gz)}\n")
            elif op == "gzerr":
                e = ctypes.c_int(0)
                m = Z.gzerror(gz, ctypes.byref(e))
                ms = m.decode("latin-1") if m else ""
                if ": " in ms:
                    ms = ms.split(": ", 1)[1]
                w(f"gzerr {e.value} {ms.replace(' ', '_') or '-'}\n")
            elif op == "gzclose":
                r = Z.gzclose(gz)
                gz = None
                w(f"gzclose {r}\n")
            elif op == "gzfile":
                with open(gzpath, "rb") as f:
                    content = f.read()
                out += content
                w(f"gzfile {len(content)} {crc(content)}\n")
            else:
                sys.exit(f"unknown op {op}")
        if ds is not None:
            Z.deflateEnd(ctypes.byref(ds))
        if istrm is not None:
            Z.inflateEnd(ctypes.byref(istrm))
        w("end\n")
        with open(os.path.join(self.outdir, f"{name}.c.bin"), "wb") as f:
            f.write(out)

    def run(self, script):
        self.paths = {}
        cmds = None
        name = None
        with open(script, encoding="ascii") as f:
            for line in f:
                t = line.split()
                if not t:
                    continue
                if t[0] == "data":
                    with open(t[2], "rb") as df:
                        b = df.read()
                    self.data[t[1]] = (ctypes.create_string_buffer(b, max(len(b), 1)), b)
                    self.paths[t[1]] = t[2]
                elif t[0] == "case":
                    name = t[1]
                    cmds = []
                elif t[0] == "end":
                    self.run_case(name, cmds)
                    cmds = None
                else:
                    cmds.append(t)
        self.trace.close()


if __name__ == "__main__":
    Runner(sys.argv[2]).run(sys.argv[1])
