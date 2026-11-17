"""Aggregate the engine's in-process samples (TEXRS_SAMPLE, engine/src/rt/sampler.rs).

Runs the job N times with TEXRS_SAMPLE set, then prints self time per function (exe
functions by symbol, DLL time by export name and by the first engine function on the
stack), optionally inclusive time (any frame on the sampled stack) and the hottest
addresses of one function.

Usage: python -I isample.py [--repeat N] [--top N] [--fn NAME] [--incl] [--groups] CWD EXE ARGS...
"""
import bisect
import collections
import ctypes
import ctypes.wintypes as W
import os
import pathlib
import subprocess
import sys
import tempfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from sampler import SYMBOL_INFO, dbghelp, exports_of, group_of, k32  # noqa: E402

WAITS = {"ZwWaitForSingleObject", "NtWaitForSingleObject", "ZwDelayExecution", "NtDelayExecution"}


class IMAGEHLP_LINE64(ctypes.Structure):
    _fields_ = [("SizeOfStruct", W.DWORD), ("Key", ctypes.c_void_p), ("LineNumber", W.DWORD),
                ("FileName", ctypes.c_char_p), ("Address", ctypes.c_uint64)]


class Resolver:
    BASE = 0x140000000

    def __init__(self, exe):
        k32.GetCurrentProcess.restype = ctypes.c_void_p
        self.proc = ctypes.c_void_p(k32.GetCurrentProcess())
        dbghelp.SymSetOptions(0x2 | 0x200)
        dbghelp.SymFromAddr.argtypes = [ctypes.c_void_p, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_void_p]
        dbghelp.SymInitializeW.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p, W.BOOL]
        dbghelp.SymInitializeW(self.proc, os.path.dirname(exe), False)
        dbghelp.SymLoadModuleExW.restype = ctypes.c_uint64
        dbghelp.SymLoadModuleExW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_wchar_p,
                                             ctypes.c_uint64, W.DWORD, ctypes.c_void_p, W.DWORD]
        self.ok = dbghelp.SymLoadModuleExW(self.proc, None, exe, None, self.BASE, 0, None, 0)
        self.sym = SYMBOL_INFO()
        self.sym.SizeOfStruct = ctypes.sizeof(SYMBOL_INFO) - 512
        self.sym.MaxNameLen = 511
        self.disp = ctypes.c_uint64()
        self.cache = {}
        self.exports = {}

    def exe(self, rva):
        if rva not in self.cache:
            name = f"?{self.BASE + rva:x}"
            if self.ok and dbghelp.SymFromAddr(self.proc, self.BASE + rva, ctypes.byref(self.disp), ctypes.byref(self.sym)):
                name = self.sym.Name.decode(errors="replace")
            self.cache[rva] = name
        return self.cache[rva]

    def line(self, addr):
        """(file, line) of an absolute (preferred-base) exe address, or None."""
        ln = IMAGEHLP_LINE64()
        ln.SizeOfStruct = ctypes.sizeof(IMAGEHLP_LINE64)
        d = W.DWORD()
        dbghelp.SymGetLineFromAddr64.argtypes = [ctypes.c_void_p, ctypes.c_uint64, ctypes.POINTER(W.DWORD), ctypes.POINTER(IMAGEHLP_LINE64)]
        if self.ok and dbghelp.SymGetLineFromAddr64(self.proc, addr, ctypes.byref(d), ctypes.byref(ln)):
            return ln.FileName.decode(errors="replace"), ln.LineNumber
        return None

    def inline_line(self, addr):
        """(file, line) of the innermost inlined frame at addr (falls back to line())."""
        dbghelp.SymAddrIncludeInlineTrace.argtypes = [ctypes.c_void_p, ctypes.c_uint64]
        dbghelp.SymAddrIncludeInlineTrace.restype = W.DWORD
        n = dbghelp.SymAddrIncludeInlineTrace(self.proc, addr) if self.ok else 0
        if n:
            ctx = W.DWORD()
            idx = W.DWORD()
            dbghelp.SymQueryInlineTrace.argtypes = [ctypes.c_void_p, ctypes.c_uint64, W.DWORD, ctypes.c_uint64,
                                                    ctypes.c_uint64, ctypes.POINTER(W.DWORD), ctypes.POINTER(W.DWORD)]
            if dbghelp.SymQueryInlineTrace(self.proc, addr, 0, addr, addr, ctypes.byref(ctx), ctypes.byref(idx)):
                ln = IMAGEHLP_LINE64()
                ln.SizeOfStruct = ctypes.sizeof(IMAGEHLP_LINE64)
                d = W.DWORD()
                dbghelp.SymGetLineFromInlineContext.argtypes = [ctypes.c_void_p, ctypes.c_uint64, W.DWORD, ctypes.c_uint64,
                                                                ctypes.POINTER(W.DWORD), ctypes.POINTER(IMAGEHLP_LINE64)]
                if dbghelp.SymGetLineFromInlineContext(self.proc, addr, ctx.value, 0, ctypes.byref(d), ctypes.byref(ln)):
                    return ln.FileName.decode(errors="replace"), ln.LineNumber
        return self.line(addr)

    def dll(self, path, rva):
        ex = self.exports.setdefault(path, exports_of(path))
        i = bisect.bisect_right(ex, (rva, "~")) - 1
        return ex[i][1] if i >= 0 else "?"


def main():
    argv = sys.argv[1:]
    repeat, top, focus, incl, groups = 1, 40, None, False, False
    callers_of = None
    lines_mode = False
    all_addrs = collections.Counter()
    while argv[0].startswith("--"):
        a = argv.pop(0)
        if a == "--incl":
            incl = True
        elif a == "--groups":
            groups = True
        elif a == "--repeat":
            repeat = int(argv.pop(0))
        elif a == "--top":
            top = int(argv.pop(0))
        elif a == "--fn":
            focus = argv.pop(0)
        elif a == "--callers":
            callers_of = argv.pop(0)
        elif a == "--lines":
            lines_mode = True
    cwd, exe, *args = argv
    exe = os.path.abspath(exe)
    res = Resolver(exe)
    selfc, inclc, callers, hot, upc = (collections.Counter() for _ in range(5))
    total = 0
    for _ in range(repeat):
        fd, path = tempfile.mkstemp(suffix=".smp")
        os.close(fd)
        env = dict(os.environ, TEXRS_SAMPLE=path)
        subprocess.run([exe, *args], cwd=cwd, env=env, stdin=subprocess.DEVNULL, capture_output=True)
        mods = []
        for line in open(path, encoding="utf-8", errors="replace"):
            if line.startswith("M "):
                _, b, s, name = line.rstrip("\n").split(" ", 3)
                mods.append((int(b, 16), int(s, 16), name))
                continue
            if not line.startswith("S "):
                continue
            words = [int(x, 16) for x in line.split()[1:]]
            rip, stack = words[0], words[1:]
            exe_mod = next((m for m in mods if m[2].lower() == exe.lower()), mods[0] if mods else (0, 0, ""))

            def where(a):
                for b, s, n in mods:
                    if b <= a < b + s:
                        return b, n
                return None, None
            b, n = where(rip)
            if b is None:
                name = "[?]"
            elif b == exe_mod[0]:
                name = res.exe(rip - b)
                if lines_mode:
                    all_addrs[Resolver.BASE + rip - b] += 1
                if focus and name.endswith("::" + focus):
                    hot[Resolver.BASE + rip - b] += 1
            else:
                fn = res.dll(n, rip - b)
                if fn in WAITS:
                    continue
                name = f"[{os.path.basename(n)}!{fn}]"
                caller = next((w for w in stack if exe_mod[0] <= w < exe_mod[0] + exe_mod[1]), None)
                callers[f"{name} <- {res.exe(caller - exe_mod[0]) if caller else '?'}"] += 1
            total += 1
            selfc[name] += 1
            if callers_of:
                frames = [name] + [res.exe(w - exe_mod[0]) for w in stack if exe_mod[0] <= w < exe_mod[0] + exe_mod[1]]
                for i, f in enumerate(frames):
                    if f.endswith(callers_of):
                        ups = []
                        for g in frames[i + 1:]:
                            if g != f and (not ups or ups[-1] != g):
                                ups.append(g.split("::")[-1][:40])
                            if len(ups) == 4:
                                break
                        upc[" < ".join(ups) or "?"] += 1
                        break
            if incl:
                seen = {name}
                for w in stack:
                    if exe_mod[0] <= w < exe_mod[0] + exe_mod[1]:
                        f = res.exe(w - exe_mod[0])
                        if f not in seen:
                            seen.add(f)
                            inclc[f] += 1
                for f in seen:
                    if f == name:
                        inclc[f] += 1
        os.remove(path)
    total = total or 1
    print(f"{repeat} run(s), {total} samples (about {total / repeat / 10:.1f} ms sampled per run)")
    if groups:
        g = collections.Counter()
        for name, c in selfc.items():
            g[group_of(name)] += c
        print("-- self time by subsystem")
        for name, c in g.most_common(20):
            print(f"{100 * c / total:6.2f}%  {name}")
    print("-- self time by function")
    for name, c in selfc.most_common(top):
        print(f"{100 * c / total:6.2f}%  {name}")
    if incl:
        print("-- inclusive (function anywhere on the sampled stack; approximate)")
        for name, c in inclc.most_common(top):
            print(f"{100 * c / total:6.2f}%  {name}")
    if callers:
        print("-- DLL time by caller")
        for name, c in callers.most_common(min(top, 20)):
            print(f"{100 * c / total:6.2f}%  {name}")
    if callers_of:
        print(f"-- callers of {callers_of} (next distinct frame up the stack)")
        for name, c in upc.most_common(top):
            print(f"{100 * c / total:6.2f}%  {name}")
    if lines_mode:
        dbghelp.SymSetOptions(0x2 | 0x200 | 0x10)
        lc = collections.Counter()
        for a, c in all_addrs.items():
            l = res.inline_line(a)
            lc[l if l else ("?", 0)] += c
        src = {}
        print("-- hottest source lines (innermost inlined frame)")
        for (f, n), c in lc.most_common(top):
            if f not in src:
                try:
                    src[f] = open(f, encoding="utf-8", errors="replace").read().splitlines()
                except OSError:
                    src[f] = []
            text = src[f][n - 1].strip()[:90] if 0 < n <= len(src[f]) else ""
            print(f"{100 * c / total:6.2f}%  {os.path.basename(f)}:{n}  {text}")
    if focus:
        print(f"-- hottest addresses in {focus}")
        for a, c in hot.most_common(top):
            print(f"  {a:#x}  {100 * c / total:5.2f}%")
        dbghelp.SymSetOptions(0x2 | 0x200 | 0x10)  # + SYMOPT_LOAD_LINES
        lines = collections.Counter()
        for a, c in hot.items():
            l = res.line(a)
            lines[l if l else ("?", 0)] += c
        print(f"-- hottest source lines in {focus}")
        src_cache = {}
        for (f, n), c in lines.most_common(top):
            if f not in src_cache:
                try:
                    src_cache[f] = open(f, encoding="utf-8", errors="replace").read().splitlines()
                except OSError:
                    src_cache[f] = []
            text = src_cache[f][n - 1].strip()[:100] if 0 < n <= len(src_cache[f]) else ""
            print(f"{100 * c / total:6.2f}%  {os.path.basename(f)}:{n}  {text}")


if __name__ == "__main__":
    main()
