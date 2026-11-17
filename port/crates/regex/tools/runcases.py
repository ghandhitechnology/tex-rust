#!/usr/bin/env python3
"""Run cases through a line-oriented regex driver with a per-case timeout.

Usage: python -I tools/runcases.py <driver.exe> <cases.tsv> <out.tsv> [timeout_s]

The driver is examples/runcase.rs (the port) or tools/c_oracle/runc.c (the C
engine built with MSVC).  cases.tsv lines:
    id \\t cflags \\t eflags \\t nmatch \\t pattern_hex \\t subject_hex
out.tsv lines: id \\t result, where result is the driver's output line, or
TIMEOUT (no answer within the timeout) / CRASH (driver died on this case).
"""
import queue
import subprocess
import sys
import threading


def main():
    exe, inp, outp = sys.argv[1], sys.argv[2], sys.argv[3]
    timeout = float(sys.argv[4]) if len(sys.argv) > 4 else 10.0
    cases = [l.rstrip("\n").split("\t") for l in open(inp) if l.strip() and not l.startswith("#")]
    results = {}
    i = 0
    while i < len(cases):
        p = subprocess.Popen([exe], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        q = queue.Queue()

        def reader(stream=p.stdout):
            for line in stream:
                q.put(line.decode("latin-1").rstrip("\r\n"))
            q.put(None)

        def writer(start=i, stream=p.stdin):
            try:
                for c in cases[start:]:
                    stream.write(("\t".join(c[1:6]) + "\n").encode())
                    stream.flush()
                stream.close()
            except OSError:
                pass

        tr = threading.Thread(target=reader, daemon=True)
        tw = threading.Thread(target=writer, daemon=True)
        tr.start()
        tw.start()
        while i < len(cases):
            try:
                r = q.get(timeout=timeout)
            except queue.Empty:
                results[cases[i][0]] = "TIMEOUT"
                i += 1
                break
            if r is None:
                results[cases[i][0]] = "CRASH"
                i += 1
                break
            results[cases[i][0]] = r
            i += 1
        p.kill()
        p.wait()
        for s in (p.stdin, p.stdout):
            try:
                s.close()
            except OSError:
                pass
        tr.join(timeout=5)
        tw.join(timeout=5)
    with open(outp, "w", newline="\n") as f:
        for c in cases:
            f.write(c[0] + "\t" + results.get(c[0], "?") + "\n")


if __name__ == "__main__":
    main()
