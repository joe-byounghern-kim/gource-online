#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The browser half of PERFORMANCE.md: fixed-dt ticks and real-time playback
# for each benchmark log on ANGLE's GL and Vulkan backends (NVIDIA GPU).
#
#   scripts/perf/browser-suite.sh <base-url> <out-dir> [label]
#
# <base-url> serves a site staged by stage-site.sh with stress.log,
# stress50k.log, stress75k.log and git.log under /perf/. Writes one JSON per
# run (<label>-<log>-<mode>-<gpu>.json) and the console output to
# <label>.txt.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
base="$1"; out="$(realpath -m "$2")"; label="${3:-run}"
mkdir -p "$out"
bench() { node "$here/browser-bench.mjs" --base "$base" "$@"; }
{
  for gpu in gl nvidia; do
    echo "### fixed stress (100k files) $gpu"
    bench --log /perf/stress.log --mode fixed --frames 3000 --gpu "$gpu" --json "$out/$label-stress-fixed-$gpu.json"
    echo "### fixed git/git (seconds-per-day=0.01) $gpu"
    bench --log /perf/git.log --mode fixed --frames 5000 --options seconds-per-day=0.01 --gpu "$gpu" --json "$out/$label-git-fixed-$gpu.json"
    for log in stress50k stress75k stress; do
      echo "### realtime $log $gpu"
      bench --log "/perf/$log.log" --mode realtime --seconds 60 --gpu "$gpu" --json "$out/$label-$log-realtime-$gpu.json"
    done
    echo "### realtime git/git (seconds-per-day=0.01) $gpu"
    bench --log /perf/git.log --mode realtime --seconds 60 --options seconds-per-day=0.01 --gpu "$gpu" --json "$out/$label-git-realtime-$gpu.json"
  done
} 2>&1 | tee "$out/$label.txt"
