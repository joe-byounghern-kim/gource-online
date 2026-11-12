# SPDX-License-Identifier: GPL-3.0-or-later
# shellcheck shell=bash
#
# Helpers shared by build.sh, render.sh and render-all.sh. Sourced, not run.

REF_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$REF_DIR/../.." && pwd)

# Name of the image build.sh produces and render.sh runs.
REF_IMAGE=${REF_IMAGE:-gource-reference:0.57}

die() {
    echo "error: $*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "'$1' is required but not installed"
}

# Print the checkout that holds the C++ sources *and* an initialised src/core
# submodule. Order: $GOURCE_SRC, this checkout (when its submodule is
# initialised), the main worktree (first entry of `git worktree list`, which is
# where the submodule lives when this is a secondary worktree).
find_gource_src() {
    local cand=${GOURCE_SRC:-}
    if [[ -z $cand && -f $REPO_ROOT/src/core/sdlapp.cpp ]]; then
        cand=$REPO_ROOT
    fi
    if [[ -z $cand ]]; then
        cand=$(git -C "$REPO_ROOT" worktree list --porcelain 2>/dev/null | sed -n '1s/^worktree //p')
    fi
    if [[ -z $cand || ! -f $cand/src/core/sdlapp.cpp ]]; then
        die "no checkout with an initialised src/core submodule found (looked at '${cand:-<none>}').
       Run 'git submodule update --init --recursive' in the main checkout or set GOURCE_SRC."
    fi
    (cd "$cand" && pwd)
}

ensure_image() {
    need docker
    if ! docker image inspect "$REF_IMAGE" >/dev/null 2>&1; then
        echo "image $REF_IMAGE not found; building it (several minutes)" >&2
        "$REF_DIR/build.sh"
    fi
}
