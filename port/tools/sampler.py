"""Sampling profiler for the Rust engine on Windows (no admin rights needed).

Runs the engine (optionally several times), samples the instruction pointer of its
busiest thread about once per millisecond (SuspendThread + GetThreadContext), then
resolves the addresses with dbghelp against the engine's PDB. Samples outside the exe
are attributed to the DLL they fall in (ntdll = system calls, loader, ...).

Usage: python -I sampler.py [--top N] [--repeat N] [--fn NAME] [--groups] CWD EXE ARGS...
  --repeat N  run the job N times and merge the samples (for short jobs)
  --fn NAME   also list the hottest addresses inside function NAME
  --groups    also print self time per subsystem (crate / runtime module)
"""
import collections
import ctypes
import ctypes.wintypes as W
import os
import subprocess
import sys
import time

k32 = ctypes.WinDLL("kernel32", use_last_error=True)
winmm = ctypes.WinDLL("winmm")
dbghelp = ctypes.WinDLL("dbghelp", use_last_error=True)
psapi = ctypes.WinDLL("psapi", use_last_error=True)

TH32CS_SNAPTHREAD = 0x4
THREAD_ALL = 0x0002 | 0x0008 | 0x0010 | 0x0040  # suspend/resume, get context, query info
CONTEXT_CONTROL = 0x00100001


class THREADENTRY32(ctypes.Structure):
    _fields_ = [("dwSize", W.DWORD), ("cntUsage", W.DWORD), ("th32ThreadID", W.DWORD),
                ("th32OwnerProcessID", W.DWORD), ("tpBasePri", W.LONG), ("tpDeltaPri", W.LONG),
                ("dwFlags", W.DWORD)]


class MODULEINFO(ctypes.Structure):
    _fields_ = [("lpBaseOfDll", ctypes.c_void_p), ("SizeOfImage", W.DWORD), ("EntryPoint", ctypes.c_void_p)]


def threads_of(pid):
    snap = k32.CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
    te = THREADENTRY32()
    te.dwSize = ctypes.sizeof(te)
    out = []
    ok = k32.Thread32First(snap, ctypes.byref(te))
    while ok:
        if te.th32OwnerProcessID == pid:
            out.append(te.th32ThreadID)
        ok = k32.Thread32Next(snap, ctypes.byref(te))
    k32.CloseHandle(snap)
    return out


def modules_of(hproc):
    """[(base, size, name)] of the modules loaded in the process."""
    arr = (ctypes.c_void_p * 512)()
    need = W.DWORD()
    if not psapi.EnumProcessModules(hproc, arr, ctypes.sizeof(arr), ctypes.byref(need)):
        return []
    out = []
    name = ctypes.create_unicode_buffer(260)
    for i in range(min(need.value // ctypes.sizeof(ctypes.c_void_p), 512)):
        mi = MODULEINFO()
        if psapi.GetModuleInformation(hproc, ctypes.c_void_p(arr[i]), ctypes.byref(mi), ctypes.sizeof(mi)):
            psapi.GetModuleBaseNameW(hproc, ctypes.c_void_p(arr[i]), name, 260)
            out.append((mi.lpBaseOfDll or 0, mi.SizeOfImage, name.value))
    return out


# x64 CONTEXT is 1232 bytes, 16-byte aligned; ContextFlags at 0x30, Rip at 0xF8
CTX_SIZE = 1232
ctx_buf = ctypes.create_string_buffer(CTX_SIZE + 16)
ctx_addr = (ctypes.addressof(ctx_buf) + 15) & ~15


STACK_BYTES = 1024
stack_buf = ctypes.create_string_buffer(STACK_BYTES)


def rip_of(h, hproc):
    """(rip, top of stack as bytes) of a thread, or None."""
    ctypes.c_uint32.from_address(ctx_addr + 0x30).value = CONTEXT_CONTROL
    if k32.SuspendThread(h) == 0xFFFFFFFF:
        return None
    try:
        if not k32.GetThreadContext(h, ctypes.c_void_p(ctx_addr)):
            return None
        rip = ctypes.c_uint64.from_address(ctx_addr + 0xF8).value
        rsp = ctypes.c_uint64.from_address(ctx_addr + 0x98).value
        got = ctypes.c_size_t()
        k32.ReadProcessMemory(hproc, ctypes.c_void_p(rsp), stack_buf, STACK_BYTES, ctypes.byref(got))
        return rip, stack_buf.raw[:got.value]
    finally:
        k32.ResumeThread(h)


def exports_of(path):
    """Sorted [(rva, name)] of a DLL's exported functions (parsed from the PE file)."""
    import struct
    try:
        d = open(path, "rb").read()
    except OSError:
        return []
    pe = struct.unpack_from("<I", d, 0x3C)[0]
    nsec = struct.unpack_from("<H", d, pe + 6)[0]
    optsz = struct.unpack_from("<H", d, pe + 20)[0]
    opt = pe + 24
    exp_rva = struct.unpack_from("<I", d, opt + 112)[0]
    secs = [struct.unpack_from("<IIII", d, opt + optsz + 40 * i + 8) for i in range(nsec)]

    def off(rva):
        for vs, va, rs, raw in secs:
            if va <= rva < va + max(vs, rs):
                return rva - va + raw
        return None
    o = off(exp_rva)
    if o is None:
        return []
    nfun, nnames, afun, aname, aord = struct.unpack_from("<IIIII", d, o + 20)
    funs = [struct.unpack_from("<I", d, off(afun) + 4 * i)[0] for i in range(nfun)]
    out = []
    for i in range(nnames):
        nrva = struct.unpack_from("<I", d, off(aname) + 4 * i)[0]
        ordi = struct.unpack_from("<H", d, off(aord) + 2 * i)[0]
        no = off(nrva)
        out.append((funs[ordi], d[no:d.index(b"\0", no)].decode()))
    return sorted(out)


def thread_cpu(h):
    c, e, kt, ut = (W.FILETIME() for _ in range(4))
    k32.GetThreadTimes(h, ctypes.byref(c), ctypes.byref(e), ctypes.byref(kt), ctypes.byref(ut))
    return ((ut.dwHighDateTime << 32) | ut.dwLowDateTime) + ((kt.dwHighDateTime << 32) | kt.dwLowDateTime)


class SYMBOL_INFO(ctypes.Structure):
    _fields_ = [("SizeOfStruct", W.ULONG), ("TypeIndex", W.ULONG), ("Reserved", ctypes.c_uint64 * 2),
                ("Index", W.ULONG), ("Size", W.ULONG), ("ModBase", ctypes.c_uint64), ("Flags", W.ULONG),
                ("Value", ctypes.c_uint64), ("Address", ctypes.c_uint64), ("Register", W.ULONG),
                ("Scope", W.ULONG), ("Tag", W.ULONG), ("NameLen", W.ULONG), ("MaxNameLen", W.ULONG),
                ("Name", ctypes.c_char * 512)]


def sample_once(cwd, exe, args, exe_samples, other_samples):
    """Run the job once. exe_samples: Counter of RVAs in the exe; other_samples: Counter of
    DLL names. Returns the wall time."""
    p = subprocess.Popen([exe, *args], cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                         stderr=subprocess.DEVNULL)
    hproc = ctypes.c_void_p(int(p._handle))
    handles = {}
    mods = []
    t0 = time.perf_counter()
    next_enum = 0.0
    best = None
    raw = []
    next_threads = 0.0
    while p.poll() is None:
        now = time.perf_counter()
        # thread snapshots cover the whole system and cost milliseconds: take them until
        # the engine thread (the second one) exists, then rarely
        if now >= next_threads:
            for tid in threads_of(p.pid):
                if tid not in handles:
                    h = k32.OpenThread(THREAD_ALL, False, tid)
                    if h:
                        handles[tid] = h
            next_threads = now + (0.004 if now - t0 < 0.1 else 0.02 if now - t0 < 1.0 else 0.25)
        if now >= next_enum:
            next_enum = now + (0.003 if now - t0 < 0.05 else 0.02 if now - t0 < 0.5 else 0.25)
            m = modules_of(hproc)
            if len(m) > len(mods):
                mods = m
        # sample every thread; threads parked in a wait call (the main thread joining the
        # engine thread) are dropped below
        for h in list(handles.values()):
            r = rip_of(h, hproc)
            if r:
                raw.append(r)
        time.sleep(0.0002)
    wall = time.perf_counter() - t0
    for h in handles.values():
        k32.CloseHandle(h)
    import bisect
    import struct
    exe_name = os.path.basename(exe).lower()
    exe_mod = next(((b, sz) for b, sz, n in mods if n.lower() == exe_name), (0, 0))
    for r, stack in raw:
        for base, size, name in mods:
            if base <= r < base + size:
                if name.lower() == exe_name:
                    exe_samples[r - base] += 1
                else:
                    # nearest export below the address, and the first return address on
                    # the stack that lies in the exe (who made the system call)
                    ex = EXPORTS.setdefault(name, exports_of(os.path.join(os.environ["SystemRoot"], "System32", name)))
                    i = bisect.bisect_right(ex, (r - base, "~")) - 1
                    fn = ex[i][1] if i >= 0 else "?"
                    if fn in WAITS:
                        break
                    caller = None
                    for k in range(0, len(stack) - 7, 8):
                        v = struct.unpack_from("<Q", stack, k)[0]
                        if exe_mod[0] <= v < exe_mod[0] + exe_mod[1]:
                            caller = v - exe_mod[0]
                            break
                    other_samples[(name, fn, caller)] += 1
                break
        else:
            other_samples[("?", "?", None)] += 1
    return wall


EXPORTS = {}
WAITS = {"ZwWaitForSingleObject", "NtWaitForSingleObject", "ZwWaitForMultipleObjects",
         "NtWaitForMultipleObjects", "ZwDelayExecution", "NtDelayExecution", "ZwWaitForAlertByThreadId",
         "NtWaitForAlertByThreadId", "ZwRemoveIoCompletion", "NtRemoveIoCompletion",
         "ZwWaitForWorkViaWorkerFactory", "NtWaitForWorkViaWorkerFactory"}


def group_of(name):
    if name.startswith("["):
        return name.split("!")[0] + "]"
    parts = name.split("::")
    if parts[0] == "pdftex":
        if len(parts) > 1 and parts[1] == "gen":
            return "pdftex::gen (translated TeX)"
        return "::".join(p for p in parts[:4] if not p.startswith("impl") and not p.startswith("<"))
    return parts[0]


def main():
    argv = sys.argv[1:]
    top, repeat, focus, groups = 40, 1, None, False
    while argv[0].startswith("--"):
        if argv[0] == "--groups":
            groups = True
            argv = argv[1:]
            continue
        if argv[0] == "--top":
            top = int(argv[1])
        elif argv[0] == "--fn":
            focus = argv[1]
        elif argv[0] == "--repeat":
            repeat = int(argv[1])
        argv = argv[2:]
    cwd, exe, *args = argv
    exe = os.path.abspath(exe)
    winmm.timeBeginPeriod(1)
    exe_samples, other = collections.Counter(), collections.Counter()
    walls = [sample_once(cwd, exe, args, exe_samples, other) for _ in range(repeat)]

    # resolve offline: load the exe's symbols at its preferred base and look up RVAs
    k32.GetCurrentProcess.restype = ctypes.c_void_p
    proc = ctypes.c_void_p(k32.GetCurrentProcess())
    dbghelp.SymSetOptions(0x2 | 0x200)  # UNDNAME | FAIL_CRITICAL_ERRORS
    dbghelp.SymFromAddr.argtypes = [ctypes.c_void_p, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_void_p]
    dbghelp.SymInitializeW.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p, W.BOOL]
    dbghelp.SymInitializeW(proc, os.path.dirname(exe), False)
    dbghelp.SymLoadModuleExW.restype = ctypes.c_uint64
    dbghelp.SymLoadModuleExW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_wchar_p,
                                         ctypes.c_uint64, W.DWORD, ctypes.c_void_p, W.DWORD]
    BASE = 0x140000000
    loaded = dbghelp.SymLoadModuleExW(proc, None, exe, None, BASE, 0, None, 0)
    funcs = collections.Counter()
    hot = collections.Counter()
    sym = SYMBOL_INFO()
    sym.SizeOfStruct = ctypes.sizeof(SYMBOL_INFO) - 512
    sym.MaxNameLen = 511
    disp = ctypes.c_uint64()
    for rva, n in exe_samples.items():
        name = f"?{BASE + rva:x}"
        if loaded and dbghelp.SymFromAddr(proc, BASE + rva, ctypes.byref(disp), ctypes.byref(sym)):
            name = sym.Name.decode(errors="replace")
        funcs[name] += n
        if focus and name.endswith("::" + focus):
            hot[BASE + rva] += n
    callers = collections.Counter()
    for (dll, fn, caller), n in other.items():
        funcs[f"[{dll}!{fn}]"] += n
        cname = "?"
        if caller is not None and loaded and dbghelp.SymFromAddr(proc, BASE + caller, ctypes.byref(disp), ctypes.byref(sym)):
            cname = sym.Name.decode(errors="replace")
        callers[f"{dll}!{fn} <- {cname}"] += n
    total = sum(funcs.values()) or 1
    print(f"{repeat} run(s), wall {min(walls)*1000:.1f} ms min / {sum(walls)/len(walls)*1000:.1f} ms mean, "
          f"{total} samples, symbols {'ok' if loaded else 'MISSING'}")
    if groups:
        g = collections.Counter()
        for name, n in funcs.items():
            g[group_of(name)] += n
        print("-- by subsystem")
        for name, n in g.most_common(25):
            print(f"{100 * n / total:6.2f}%  {name}")
        print("-- by function")
    for name, n in funcs.most_common(top):
        print(f"{100 * n / total:6.2f}%  {name}")
    if callers:
        print("-- time in system DLLs by (DLL function <- first engine function on the stack)")
        for name, n in callers.most_common(top):
            print(f"{100 * n / total:6.2f}%  {name}")
    if focus:
        print(f"hottest addresses in {focus} (image base {BASE:#x}):")
        for a, n in hot.most_common(top):
            print(f"  {a:#x}  {100 * n / total:5.2f}%")


if __name__ == "__main__":
    main()
