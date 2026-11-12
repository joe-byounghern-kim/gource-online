#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Time the C++ Gource frame by frame on a custom log, headless at a fixed
# 1/60 s tick (the PPM exporter forces Gource's fixed tick rate; frames go to
# /dev/null, glReadPixels is outside the timed region).
#
#   scripts/perf/cpp-timing/run.sh <custom.log> <out-dir> <frames> [gource args...]
#
# Environment: GL=llvmpipe (default; Xvfb + Mesa software GL) or GL=nvidia
# (EGL on the NVIDIA GPU through Docker CDI, SDL's offscreen video driver).
# FINISH=1 adds glFinish() after draw(). Writes <out-dir>/timing.txt
# (frame logic_us draw_us finish_us files users dirs) and prints a summary.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
log="$(realpath "$1")"; out="$(realpath -m "$2")"; frames="$3"; shift 3
mkdir -p "$out"
seconds=$(python3 -c "print($frames / 60)")
args=(--viewport 1280x720 --output-framerate 60 --output-ppm-stream /dev/null
      --hide mouse --disable-input --no-vsync --stop-at-time "$seconds" "$@" /in.log)
case "${GL:-llvmpipe}" in
  llvmpipe)
    docker run --rm --init --user "$(id -u):$(id -g)" -v "$log:/in.log:ro" -v "$out:/out" \
      -e GOURCE_TIMING_FILE=/out/timing.txt -e GOURCE_TIMING_FINISH="${FINISH:-0}" \
      gource-timing:0.57 \
      xvfb-run -a -s "-screen 0 1920x1080x24" gource "${args[@]}" 2> "$out/stderr.txt" ;;
  nvidia)
    docker run --rm --init --user "$(id -u):$(id -g)" --device nvidia.com/gpu=all \
      -e NVIDIA_DRIVER_CAPABILITIES=all \
      -v "$log:/in.log:ro" -v "$out:/out" \
      -e GOURCE_TIMING_FILE=/out/timing.txt -e GOURCE_TIMING_FINISH="${FINISH:-0}" \
      -e LIBGL_ALWAYS_SOFTWARE= -e GALLIUM_DRIVER= -e SDL_VIDEODRIVER=offscreen \
      -e __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/10_nvidia.json \
      gource-timing:0.57 gource "${args[@]}" 2> "$out/stderr.txt" ;;
  *) echo "GL must be llvmpipe or nvidia" >&2; exit 2 ;;
esac
python3 "$here/summary.py" "$out/timing.txt" "$frames" "$out/summary.json"
