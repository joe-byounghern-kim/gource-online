#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A/B two gource-rs builds on one log with hardware counters (perf stat in
# Docker: instructions, cycles, cache misses) plus the bench phase means,
# alternating the builds `runs` times. Instruction counts barely move with
# machine load, so they show a change even on a busy host; wall times are
# the median of the runs.
#
#   scripts/perf/ab-native.sh <base gource-rs> <cand gource-rs> <custom.log> <runs> [bench args...]
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
base="$(realpath "$1")"; cand="$(realpath "$2")"; log="$(realpath "$3")"; runs="$4"; shift 4
image=gource-perf-tools:1
docker image inspect "$image" >/dev/null 2>&1 || docker build -q -t "$image" "$root/scripts/perf/perf-tools" >&2
for i in $(seq "$runs"); do
  for which in base cand; do
    bin=$base; [[ $which == cand ]] && bin=$cand
    docker run --rm --cap-add PERFMON -v "$bin:/gource-rs:ro" -v "$log:/in.log:ro" "$image" \
      bash -c 'perf stat -x, -e instructions:u,cycles:u,cache-misses:u -o /tmp/stat \
                 /gource-rs bench /in.log --json "$@" > /tmp/bench.json
               awk -F, -v w='"$which"' '\''/instructions|cycles|cache-misses/ { printf "%s %s=%s ", w, $3, $1 }'\'' /tmp/stat
               grep -A1 "\"\(update\|extract_view\|build\|frame\)\": {" /tmp/bench.json | grep -o "\"[a-z_]*\": {\|\"total_ms\": [0-9.]*" | paste -sd" " | sed "s/\": {//g; s/\"total_ms\": / /g; s/\"//g"' _ "$@"
  done
done
