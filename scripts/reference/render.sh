#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Render one Gource log with the C++ reference build, headless, to PNG frames.
#
#   scripts/reference/render.sh <log-file> <out-dir> [extra gource args...]
#
# Runs, inside the gource-reference image (build it with build.sh; render.sh
# builds it on first use), under xvfb-run with LIBGL_ALWAYS_SOFTWARE=1:
#
#   gource --viewport 1280x720 --output-framerate 30 --output-ppm-stream - \
#          --stop-at-end --hide mouse --disable-input --no-vsync \
#          <extra gource args> <log-file>
#
# and converts the PPM stream to <out-dir>/frame-%05d.png (every Nth frame and
# the last one). <out-dir> also receives input.log (a copy of the log),
# command.txt (the exact gource command), environment.txt (Gource version, GL
# renderer, image id), gource.stderr.log, manifest.json (frame size, timing,
# per-frame data), frames.sha256 (hash of every frame) and contact-sheet.png.
#
# Environment:
#   REF_WIDTH, REF_HEIGHT  viewport (default 1280 x 720)
#   REF_FPS                --output-framerate: 25, 30 or 60 (default 30)
#   REF_EVERY              save every Nth frame (default 15); the last frame is
#                          always saved. 1 saves all of them.
#   REF_TIMEOUT            seconds before the run is killed (default 1800)
#   REF_CPUS               docker --cpus limit, 0 for none (default 8)
#   REF_GL_THREADS         llvmpipe rasteriser threads (default 4)
#   REF_IMAGE              image to run (default gource-reference:0.57)
set -euo pipefail

# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# Print the comment block at the top of this file (from line 4 to the first
# non-comment line).
usage() {
    awk 'NR >= 4 && /^#/ { sub(/^# ?/, ""); print; next } NR >= 4 { exit }' "${BASH_SOURCE[0]}"
}

if [[ $# -lt 2 || $1 == -h || $1 == --help ]]; then
    usage >&2
    exit 2
fi

LOG=$1
OUT=$2
shift 2

[[ -f $LOG ]] || die "log file '$LOG' not found"
[[ -s $LOG ]] || die "log file '$LOG' is empty"

REF_WIDTH=${REF_WIDTH:-1280}
REF_HEIGHT=${REF_HEIGHT:-720}
REF_FPS=${REF_FPS:-30}
REF_EVERY=${REF_EVERY:-15}
REF_TIMEOUT=${REF_TIMEOUT:-1800}
REF_CPUS=${REF_CPUS:-8}
REF_GL_THREADS=${REF_GL_THREADS:-4}

case $REF_FPS in 25 | 30 | 60) ;; *) die "REF_FPS must be 25, 30 or 60 (Gource's supported output rates)" ;; esac
[[ $REF_EVERY =~ ^[1-9][0-9]*$ ]] || die "REF_EVERY must be a positive integer"

ensure_image

mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
SCENARIO=$(basename "$OUT")

# Remove what a previous run left behind so frames of different runs never mix.
rm -f "$OUT"/frame-*.png "$OUT"/frame-*.png.tmp "$OUT"/contact-sheet.png \
      "$OUT"/manifest.json "$OUT"/frames.sha256 "$OUT"/command.txt "$OUT"/environment.txt \
      "$OUT"/gource.stderr.log

LOG_ABS=$(cd "$(dirname "$LOG")" && pwd)/$(basename "$LOG")
if [[ $LOG_ABS != "$OUT/input.log" ]]; then
    cp -f -- "$LOG" "$OUT/input.log"
fi

CPU_ARGS=()
if [[ $REF_CPUS != 0 ]]; then
    CPU_ARGS=(--cpus "$REF_CPUS")
fi

docker run --rm --init --user "$(id -u):$(id -g)" ${CPU_ARGS[@]+"${CPU_ARGS[@]}"} \
    -e HOME=/tmp \
    -e REF_WIDTH="$REF_WIDTH" -e REF_HEIGHT="$REF_HEIGHT" -e REF_FPS="$REF_FPS" \
    -e REF_EVERY="$REF_EVERY" -e REF_TIMEOUT="$REF_TIMEOUT" -e REF_SCENARIO="$SCENARIO" \
    -e LP_NUM_THREADS="$REF_GL_THREADS" \
    -v "$REF_DIR/container:/ref:ro" \
    -v "$OUT:/out" \
    "$REF_IMAGE" /ref/render-inner.sh "$@"

{
    echo "image: $REF_IMAGE $(docker image inspect --format '{{.Id}}' "$REF_IMAGE")"
    echo "source-fingerprint: $(docker image inspect --format '{{index .Config.Labels "gource.source-fingerprint"}}' "$REF_IMAGE")"
} >> "$OUT/environment.txt"

echo "frames in $OUT" >&2
