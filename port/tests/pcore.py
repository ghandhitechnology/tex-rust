"""Pin the current process (and so every child it starts) to the CPU's performance cores.

Hybrid Intel CPUs (here an i5-14400F: 6 P-cores with 2 threads each, 4 E-cores) give a job
very different speeds depending on where Windows schedules it, which makes timings noisy.
Both engines are pinned the same way, so comparisons stay fair.
"""
import ctypes
import ctypes.wintypes as W

k32 = ctypes.WinDLL("kernel32", use_last_error=True)


class SYSTEM_CPU_SET_INFORMATION(ctypes.Structure):
    _fields_ = [("Size", W.DWORD), ("Type", W.DWORD), ("Id", W.DWORD), ("Group", W.WORD),
                ("LogicalProcessorIndex", ctypes.c_ubyte), ("CoreIndex", ctypes.c_ubyte),
                ("LastLevelCacheIndex", ctypes.c_ubyte), ("NumaNodeIndex", ctypes.c_ubyte),
                ("EfficiencyClass", ctypes.c_ubyte), ("AllFlags", ctypes.c_ubyte),
                ("SchedulingClass", ctypes.c_ubyte), ("Reserved", ctypes.c_ubyte * 3),
                ("AllocationTag", ctypes.c_uint64)]


def pcore_mask():
    """Affinity mask of the logical processors with the highest efficiency class."""
    need = W.ULONG()
    k32.GetSystemCpuSetInformation(None, 0, ctypes.byref(need), None, 0)
    buf = ctypes.create_string_buffer(need.value)
    if not k32.GetSystemCpuSetInformation(buf, need, ctypes.byref(need), None, 0):
        return 0
    off, cpus = 0, []
    while off < need.value:
        e = SYSTEM_CPU_SET_INFORMATION.from_buffer_copy(buf.raw[off:off + ctypes.sizeof(SYSTEM_CPU_SET_INFORMATION)])
        cpus.append((e.LogicalProcessorIndex, e.EfficiencyClass))
        off += e.Size or ctypes.sizeof(SYSTEM_CPU_SET_INFORMATION)
    top = max(c for _, c in cpus)
    mask = 0
    for i, c in cpus:
        if c == top:
            mask |= 1 << i
    return mask


def pin_to_pcores():
    mask = pcore_mask()
    if mask:
        k32.GetCurrentProcess.restype = ctypes.c_void_p
        k32.SetProcessAffinityMask(ctypes.c_void_p(k32.GetCurrentProcess()), ctypes.c_size_t(mask))
    return mask


if __name__ == "__main__":
    print(hex(pcore_mask()))
