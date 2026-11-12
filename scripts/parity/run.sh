#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Render the port at every reference frame (rand-mode glibc and keyed) and
# compare with the C++ reference renders. See docs/web-mvp/PARITY.md.
#
#   scripts/parity/run.sh [render.mjs options] [scenario ...]
#
# Reference root: $PARITY_REF, default target/reference (scripts/reference/).
# Output: target/parity/port-glibc/ and target/parity/port-keyed/, each with
# frames, metrics.{json,csv} and sheet-*.jpg per scenario and summary.md.
# Needs scripts/build-web.sh to have been run (packages/viewer/src/wasm/).
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export PARITY_REF="${PARITY_REF:-$root/target/reference}"
for mode in glibc keyed; do
  node "$root/scripts/parity/render.mjs" --rand-mode "$mode" --out "$root/target/parity/port-$mode" "$@"
  python3 "$root/scripts/parity/compare.py" "$root/target/parity/port-$mode" --label "$mode"
done
