#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build the reference image (scripts/reference/Dockerfile): the C++ Gource from
# this repository plus a headless GL stack.
#
#   scripts/reference/build.sh [extra 'docker build' arguments, e.g. --no-cache]
#
# Environment:
#   GOURCE_SRC   checkout to take the C++ sources from. Default: this checkout
#                if its src/core submodule is initialised, otherwise the main
#                worktree (secondary worktrees have an empty src/core).
#   REF_IMAGE    image name:tag (default gource-reference:0.57)
#   MAKE_JOBS    parallel compile jobs inside the image build (default 8)
#
# Only an allow-list of the source tree goes to Docker (never target/,
# node_modules/, .git/), copied into a temporary directory used as the build
# context.
set -euo pipefail

# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

need docker
need tar
need sha256sum

SRC=$(find_gource_src)
MAKE_JOBS=${MAKE_JOBS:-8}

# What ./configure and make read: autotools inputs, the m4 macros, the sources
# (including the src/core submodule) and data/ (textures, shaders, font) that
# `make install` copies into the image.
CONTEXT_ITEMS=(src data m4 configure.ac Makefile.am autogen.sh)
for item in "${CONTEXT_ITEMS[@]}"; do
    [[ -e $SRC/$item ]] || die "'$SRC/$item' is missing; is '$SRC' a Gource checkout?"
done

CTX=$(mktemp -d "${TMPDIR:-/tmp}/gource-reference-ctx.XXXXXX")
trap 'rm -rf "$CTX"' EXIT

echo "build context from $SRC -> $CTX" >&2
tar -C "$SRC" \
    --exclude='.git' --exclude='*.o' --exclude='.deps' --exclude='.dirstamp' \
    -cf - "${CONTEXT_ITEMS[@]}" | tar -C "$CTX" -xf -

# Fingerprint of exactly what gets compiled, recorded as an image label.
FINGERPRINT=$(cd "$CTX" && find . -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum | sha256sum | cut -c1-16)
echo "source fingerprint $FINGERPRINT ($(du -sh "$CTX" | cut -f1) of context)" >&2

docker build \
    --file "$REF_DIR/Dockerfile" \
    --tag "$REF_IMAGE" \
    --build-arg "MAKE_JOBS=$MAKE_JOBS" \
    --build-arg "SOURCE_FINGERPRINT=$FINGERPRINT" \
    "$@" \
    "$CTX"

echo "built $REF_IMAGE" >&2
