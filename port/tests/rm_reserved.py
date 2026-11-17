"""Delete files with reserved Windows names (nul, con, ...) under a directory, using the
\\\\?\\ prefix that bypasses Win32 device-name parsing. Usage: python -I rm_reserved.py DIR"""
import os
import sys

RESERVED = {"nul", "con", "aux", "prn"} | {f"com{i}" for i in range(1, 10)} | {f"lpt{i}" for i in range(1, 10)}

for root, dirs, files in os.walk(sys.argv[1]):
    for n in files:
        if n.lower().split(".")[0] in RESERVED:
            # abspath() would turn a reserved name into a device path, so join after it
            p = "\\\\?\\" + os.path.join(os.path.abspath(root), n)
            os.remove(p)
            print("removed", p)
