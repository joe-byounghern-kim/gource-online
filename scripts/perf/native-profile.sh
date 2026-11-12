#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Sample `gource-rs bench` with Linux perf (inside Docker, so the host's
# perf_event_paranoid does not matter; perf runs as root with CAP_PERFMON
# in the container, outputs are chowned back) and write a flame graph and a flat
# profile.
#
#   scripts/perf/native-profile.sh <custom.log> <out-dir> [bench args...]
#
# Output: <out-dir>/flame.svg, <out-dir>/top.txt (self time per symbol),
# <out-dir>/bench.txt. Builds gource-rs with line tables into
# target/perf-prof (the release profile otherwise strips symbols).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
log="$(realpath "$1")"; out="$(realpath -m "$2")"; shift 2
mkdir -p "$out"
export PATH="$HOME/.cargo/bin:$PATH"

CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=line-tables-only \
  CARGO_TARGET_DIR="$root/target/perf-prof" \
  cargo build --release -p gource-cli --manifest-path "$root/Cargo.toml" >&2

image=gource-perf-tools:1
docker image inspect "$image" >/dev/null 2>&1 \
  || docker build -q -t "$image" "$root/scripts/perf/perf-tools" >&2

docker run --rm --cap-add PERFMON -e HOST_UID="$(id -u):$(id -g)" \
  -v "$root/target/perf-prof/release/gource-rs:/gource-rs:ro" \
  -v "$log:/in.log:ro" -v "$out:/out" "$image" bash -c '
    set -e
    perf record -F 2000 -g --call-graph dwarf,32768 -o /out/perf.data \
      /gource-rs bench /in.log "$@" > /out/bench.txt 2>/out/perf-record.txt
    perf report -i /out/perf.data --no-children --sort symbol --stdio \
      --percent-limit 0.5 -g none 2>/dev/null > /out/top.txt
    perf script -i /out/perf.data 2>/dev/null \
      | /opt/FlameGraph/stackcollapse-perf.pl \
      | /opt/FlameGraph/flamegraph.pl --width 1600 > /out/flame.svg
    rm -f /out/perf.data
    chown -R "$HOST_UID" /out' _ "$@"
cat "$out/bench.txt"
sed -n '1,60p' "$out/top.txt" | grep -v '^#' | grep -v '^$'
