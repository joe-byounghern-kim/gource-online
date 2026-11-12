#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Runs INSIDE the reference image (this directory is mounted read-only at /ref;
# the output directory is mounted at /out and holds input.log). render.sh calls
# it; you normally do not run it by hand.
#
#   render-inner.sh [extra gource args...]
#
# Environment (all set by render.sh): REF_WIDTH REF_HEIGHT REF_FPS REF_EVERY
# REF_TIMEOUT REF_SCENARIO.
set -uo pipefail

W=${REF_WIDTH:-1280}
H=${REF_HEIGHT:-720}
FPS=${REF_FPS:-30}
EVERY=${REF_EVERY:-15}
TIMEOUT=${REF_TIMEOUT:-1800}
NAME=${REF_SCENARIO:-}
OUT=/out
LOG=$OUT/input.log

[[ -s $LOG ]] || { echo "render-inner: $LOG missing or empty" >&2; exit 2; }

# Larger than the viewport so the GL window is never clipped by the X screen
# (pixels outside the screen are undefined for glReadPixels).
SCREEN_W=$(( W > 1920 ? W : 1920 ))
SCREEN_H=$(( H > 1080 ? H : 1080 ))

# The reference command. --disable-input and --no-vsync keep the run
# independent of (absent) devices and display sync; the simulation itself runs
# on a fixed tick in export mode, so wall-clock speed does not matter.
GOURCE_ARGS=(
    --viewport "${W}x${H}"
    --output-framerate "$FPS"
    --output-ppm-stream -
    --stop-at-end
    --hide mouse
    --disable-input
    --no-vsync
    "$@"
    "$LOG"
)
{ printf '%q ' gource "${GOURCE_ARGS[@]}"; echo; } > "$OUT/command.txt"

# The software stack that produced the frames: the GL implementation decides
# the pixels, so it belongs next to them.
{
    gource --help | head -n 1
    xvfb-run -a -s "-screen 0 640x480x24" glxinfo -B 2>/dev/null | grep -E '^OpenGL .*string'
} > "$OUT/environment.txt" 2>/dev/null

# gource writes the PPM stream to stdout and diagnostics to stderr, which
# gource-run.sh keeps in a file next to the frames (xvfb-run itself would merge
# stderr into the stream, see gource-run.sh).
: > "$OUT/gource.stderr.log"
xvfb-run -a -s "-screen 0 ${SCREEN_W}x${SCREEN_H}x24" \
    /ref/gource-run.sh "$TIMEOUT" "$OUT/gource.stderr.log" "${GOURCE_ARGS[@]}" \
  | python3 /ref/ppm2png.py --out-dir "$OUT" --every "$EVERY" --fps "$FPS" \
        --scenario "$NAME" --require-ink
status=("${PIPESTATUS[@]}")

if [[ ${status[0]} -ne 0 ]]; then
    if [[ ${status[0]} -eq 124 ]]; then
        echo "render-inner: gource timed out after ${TIMEOUT}s" >&2
    else
        echo "render-inner: gource exited with ${status[0]}" >&2
    fi
    echo "--- gource.stderr.log (last lines)" >&2
    tail -n 15 "$OUT/gource.stderr.log" >&2
    exit "${status[0]}"
fi
exit "${status[1]}"
