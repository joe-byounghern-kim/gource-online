#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Behaviour check for performance work: run `gource-rs simulate` (every 60
# frames, 3000 frames) with two builds on every fixture and scenario log
# (plus any extra logs given) and diff the view hashes.
#
#   scripts/perf/compare-hashes.sh <baseline gource-rs> <candidate gource-rs> [extra.log...]
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
base="$1"; cand="$2"; shift 2
status=0
for log in "$root"/tests/fixtures/custom-log/*.log "$root"/tests/logs/*.log "$@"; do
  [[ -f $log ]] || continue
  a=$("$base" simulate "$log" --frames 3000 --every 60 2>&1 || true)
  b=$("$cand" simulate "$log" --frames 3000 --every 60 2>&1 || true)
  if [[ "$a" == "$b" ]]; then
    echo "same  $(basename "$log")  $(tail -1 <<<"$b" | grep -o 'hash=[0-9a-f]*')"
  else
    echo "DIFF  $(basename "$log")"; diff <(echo "$a") <(echo "$b") | head -6; status=1
  fi
done
exit $status
