#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Summarise a timing.txt written by the instrumented C++ Gource.

    summary.py timing.txt [frames] [out.json]

Only the first `frames` frames count (Gource's --stop-at-time overshoots).
"""
import json
import math
import sys

rows = [list(map(int, l.split())) for l in open(sys.argv[1]) if l.strip()]
if len(sys.argv) > 2 and int(sys.argv[2]) > 0:
    rows = rows[: int(sys.argv[2])]
def summ(v):
    s = sorted(v)
    rank = lambda p: s[max(1, min(len(s), math.ceil(len(s) * p / 100))) - 1]
    return {"mean": sum(s) / len(s) / 1000, "p50": rank(50) / 1000, "p95": rank(95) / 1000, "max": s[-1] / 1000}
out = {"frames": len(rows)}
for name, i in (("logic", 1), ("draw", 2), ("finish", 3)):
    out[name] = summ([r[i] for r in rows])
out["frame"] = summ([r[1] + r[2] + r[3] for r in rows])
out["last"] = {"files": rows[-1][4], "users": rows[-1][5], "dirs": rows[-1][6]}
out["peak"] = {"files": max(r[4] for r in rows), "users": max(r[5] for r in rows), "dirs": max(r[6] for r in rows)}
print(f"frames      {out['frames']}")
print(f"{'phase':<10}{'mean':>10}{'p50':>10}{'p95':>10}{'max':>10}   (ms)")
for k in ("logic", "draw", "finish", "frame"):
    s = out[k]
    print(f"{k:<10}{s['mean']:>10.4f}{s['p50']:>10.4f}{s['p95']:>10.4f}{s['max']:>10.4f}")
print(f"last        {out['last']}")
print(f"peak        {out['peak']}")
if len(sys.argv) > 3:
    json.dump(out, open(sys.argv[3], "w"), indent=2)
