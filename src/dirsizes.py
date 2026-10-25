"""Aggregate sizes per directory prefix from a `7z l -slt` listing. usage: dirsizes.py LISTING [N]"""
import collections, sys

d = collections.Counter()
cur = None
for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
    line = line.rstrip("\n")
    if line.startswith("Path = "):
        cur = line[7:].replace("\\", "/")
    elif line.startswith("Size = ") and cur:
        s = int(line[7:] or 0)
        parts = cur.split("/")
        for depth in (2, 3, 4):
            if len(parts) > depth:
                d["/".join(parts[:depth])] += s
for k, v in sorted(d.items(), key=lambda x: -x[1])[: int(sys.argv[2]) if len(sys.argv) > 2 else 45]:
    print(f"{v / 2**20:9.1f} MB  {k}")
