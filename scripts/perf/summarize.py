#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Markdown tables from browser-suite.sh output.

    scripts/perf/summarize.py <suite-dir> <label> [<label>...]

One fixed-dt table (mean / p95 ms per phase) and one real-time table (fps,
dropped frames, CPU per frame, peak counts, memory) per label.
"""
import json
import os
import sys

suite, labels = sys.argv[1], sys.argv[2:]


def load(label, log, mode, gpu):
    p = os.path.join(suite, f"{label}-{log}-{mode}-{gpu}.json")
    return json.load(open(p)) if os.path.exists(p) else None


GPUS = (("gl", "ANGLE GL"), ("nvidia", "ANGLE Vulkan"))
print("### Fixed-dt ticks (ms, mean / p95)\n")
print("| build | log | backend | sim | build | gpu submit | frame | peak files | uploads MiB/frame |")
print("|---|---|---|---|---|---|---|---|---|")
for label in labels:
    for log, name in (("stress", "stress 100k"), ("git", "git/git")):
        for gpu, gname in GPUS:
            r = load(label, log, "fixed", gpu)
            if not r:
                continue
            f = r["fixed"]
            cell = lambda k: f"{f[k]['mean']:.2f} / {f[k]['p95']:.1f}"
            up = f.get("uploaded_bytes", {}).get("mean", 0) / 1048576
            print(f"| {label} | {name} | {gname} | {cell('sim')} | {cell('build')} | "
                  f"{cell('gpu_submit')} | {cell('frame')} | {f['peak']['files']} | {up:.2f} |")
print("\n### Real-time playback, 60 s\n")
print("| build | log | backend | fps | dropped (>20.7 ms) | CPU frame mean / p95 | CPU >16.7 ms | peak files | wasm growth MiB |")
print("|---|---|---|---|---|---|---|---|---|")
for label in labels:
    for log, name in (("stress50k", "stress 50k"), ("stress75k", "stress 75k"), ("stress", "stress 100k"), ("git", "git/git")):
        for gpu, gname in GPUS:
            r = load(label, log, "realtime", gpu)
            if not r:
                continue
            t = r["realtime"]
            print(f"| {label} | {name} | {gname} | {t['fps']:.1f} | {t['frames_over_20_7ms']} | "
                  f"{t['cpu_frame']['mean']:.2f} / {t['cpu_frame']['p95']:.1f} | {t['cpu_frames_over_16_7ms']} | "
                  f"{t['peak']['files']} | {t['wasm_memory_growth_bytes'] / 1048576:.1f} |")
