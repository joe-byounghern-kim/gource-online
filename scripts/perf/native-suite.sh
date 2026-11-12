#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The native half of PERFORMANCE.md: `gource-rs bench` (release) on each
# benchmark log, `runs` times, alternating builds when two are given, and
# the per-phase mean/p95/max of the median run (by frame mean).
#
#   scripts/perf/native-suite.sh <logs-dir> <runs> <gource-rs> [<other gource-rs>...]
#
# <logs-dir> holds stress.log (100k files, gen-stress-log.py), stress50k.log,
# stress75k.log and git.log (git/git, see PERFORMANCE.md).
set -euo pipefail
logs="$(realpath "$1")"; runs="$2"; shift 2
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
declare -A ARGS=(
  [stress]="--frames 3000"
  [stress75k]="--frames 3000"
  [stress50k]="--frames 3000"
  [git]="--frames 5000 --options seconds-per-day=0.01"
)
for log in stress stress75k stress50k git; do
  [[ -f $logs/$log.log ]] || continue
  for i in $(seq "$runs"); do
    n=0
    for bin in "$@"; do
      n=$((n + 1))
      # shellcheck disable=SC2086
      "$bin" bench "$logs/$log.log" ${ARGS[$log]} --json > "$tmp/$log-$n-$i.json"
    done
  done
  n=0
  for bin in "$@"; do
    n=$((n + 1))
    python3 - "$log" "$bin" "$tmp"/"$log-$n"-*.json <<'PY'
import json, sys
log, binary, files = sys.argv[1], sys.argv[2], sys.argv[3:]
runs = sorted((json.load(open(f)) for f in files), key=lambda r: r["phases"]["frame"]["mean_ms"])
r = runs[len(runs) // 2]
p = r["phases"]
print(f"{log:<10} {binary}")
print(f"  {'phase':<13}{'mean':>9}{'p95':>9}{'max':>9}   (ms, median of {len(runs)} runs)")
for k in ("update", "extract_view", "build", "frame"):
    print(f"  {k:<13}{p[k]['mean_ms']:>9.3f}{p[k]['p95_ms']:>9.3f}{p[k]['max_ms']:>9.3f}")
pk = r.get("peak")
peak = (f"peak {pk['files']} files, {pk['users']} users, {pk['dirs']} dirs, {pk['quads']} quads, "
        f"{pk['text_quads']} text quads; " if pk else "")
print(f"  {peak}hash {r['final']['hash']}")
PY
  done
done
