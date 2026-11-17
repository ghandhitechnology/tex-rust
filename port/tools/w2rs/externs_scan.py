"""List identifiers the tangled Pascal uses but never declares (web2c externals), with counts
and one sample call each."""
import collections
import sys

import os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pas

prog = pas.parse_file(sys.argv[1], {"INITEX", "STAT"})
declared = set()
for n, _ in prog["consts"]:
    declared.add(n)
for n, _ in prog["types"]:
    declared.add(n)
for n, _ in prog["vars"]:
    declared.add(n)
procs = {p["name"]: p for p in prog["procs"]}
declared |= set(procs)

uses = collections.Counter()
sample = {}


def walk(n, local):
    if isinstance(n, tuple) and n:
        k = n[0]
        if k == "id" and n[1] not in local and n[1] not in declared:
            uses[n[1]] += 1
            sample.setdefault(n[1], n)
        elif k in ("call", "pcall"):
            if n[1] not in local and n[1] not in declared:
                uses[n[1]] += 1
                sample.setdefault(n[1], n)
            for a in n[2]:
                walk(a, local)
            return
        elif k == "for":
            if n[1] not in local and n[1] not in declared:
                uses[n[1]] += 1
        for x in n[1:]:
            walk(x, local)
    elif isinstance(n, list):
        for x in n:
            walk(x, local)


for p in prog["procs"]:
    if p["forward"]:
        continue
    local = {v for v, _ in p["vars"]} | {a for a, _, _ in p["params"]} | {p["name"]} | {c for c, _ in p["consts"]}
    walk(p["body"], local)
for name, ty in prog["vars"]:
    pass

for k, v in sorted(uses.items(), key=lambda kv: -kv[1]):
    s = repr(sample.get(k))
    print(f"{v:5} {k:28} {s[:150]}")
