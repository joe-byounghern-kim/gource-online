#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build the release wasm + a test build of the player (its test hooks) and stage
# it with benchmark logs in a directory any static server can serve (for example
# `npx vite preview --outDir <site-dir>`), so `#src=/perf/<name>` loads them.
#
#   scripts/perf/stage-site.sh <site-dir> <log>... [--profiling]
#
# --profiling keeps the wasm name section (function names in Chrome CPU
# profiles) by building with strip = false into target/perf-prof.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
site="$(realpath -m "$1")"; shift
logs=(); profiling=0
for a in "$@"; do
  if [[ $a == --profiling ]]; then profiling=1; else logs+=("$(realpath "$a")"); fi
done
export PATH="$HOME/.cargo/bin:$PATH"
cd "$root"
if [[ $profiling == 1 ]]; then
  CARGO_PROFILE_RELEASE_STRIP=false CARGO_TARGET_DIR="$root/target/perf-prof" scripts/build-web.sh --release
else
  scripts/build-web.sh --release
fi
GOURCE_PLAYER_TEST_BUILD=1 VITE_E2E_HOOKS=1 pnpm -C apps/player build >/dev/null
rm -rf "$site"
cp -r apps/player/dist "$site"
mkdir -p "$site/perf"
for l in "${logs[@]}"; do cp "$l" "$site/perf/"; done
echo "staged $site ($(ls "$site/perf" | wc -l) logs)"
