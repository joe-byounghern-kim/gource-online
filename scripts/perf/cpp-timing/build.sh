#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build gource-timing:0.57: the reference C++ Gource plus per-frame timing.
# The sources are copied to a temporary directory and instrumented there;
# the repository's C++ tree is never modified.
#
#   scripts/perf/cpp-timing/build.sh
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../../reference/common.sh
source "$here/../../reference/common.sh"
need docker
ensure_image
SRC=$(find_gource_src)
CTX=$(mktemp -d "${TMPDIR:-/tmp}/gource-timing-ctx.XXXXXX")
trap 'rm -rf "$CTX"' EXIT
tar -C "$SRC" --exclude='.git' --exclude='*.o' --exclude='.deps' --exclude='.dirstamp' \
    -cf - src data m4 configure.ac Makefile.am autogen.sh | tar -C "$CTX" -xf -
python3 "$here/instrument.py" "$CTX/src/gource.cpp"
docker build --file "$here/Dockerfile" --tag gource-timing:0.57 \
    --build-arg "MAKE_JOBS=${MAKE_JOBS:-8}" "$CTX"
