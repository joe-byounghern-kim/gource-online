#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Render every reference scenario with the C++ Gource into
# target/reference/<scenario>/ (frames, input.log, command.txt, manifest.json,
# contact-sheet.png; see render.sh).
#
#   scripts/reference/render-all.sh [scenario ...]    # default: all
#
# Scenarios:
#   * every custom-format log in tests/logs/ (files in other formats, such as
#     the SVN XML log and the caption file, are skipped)
#   * visual-hierarchy-activity, dense-contributors, deletion-recreation from
#     tests/fixtures/custom-log/
#   * gource-history: this repository's own history up to the last upstream
#     commit (Gource v0.57), converted to the custom log format by the
#     container's `gource --output-custom-log` against a read-only mount of the
#     main checkout, then rendered for its first year.
#
# Environment (besides the REF_* variables render.sh documents):
#   REF_OUT           output root (default <repo>/target/reference)
#   REF_HISTORY_REV   git revision for gource-history (default 70db547, the
#                     commit "Bump version and fix ChangeLog", Gource v0.57)
#   GOURCE_SRC        checkout holding the git history (default: as build.sh)
set -euo pipefail

# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# Absolute, because it is bind-mounted into containers (a relative path would
# be taken for a Docker volume name).
OUT_ROOT=${REF_OUT:-$REPO_ROOT/target/reference}
mkdir -p "$OUT_ROOT"
OUT_ROOT=$(cd "$OUT_ROOT" && pwd)
HISTORY_REV=${REF_HISTORY_REV:-70db547}

# --- scenario table --------------------------------------------------------
# add <name> <log path relative to the repo root | @history> [gource args]
# Arguments are split on spaces; none of them needs quoting.
NAMES=()
LOGS=()
ARGS=()
add() {
    NAMES+=("$1")
    LOGS+=("$2")
    ARGS+=("${3:-}")
}

# True when the first non-empty line looks like the custom format
# (timestamp|user|A/M/D|path).
is_custom_log() {
    local first
    first=$(grep -m1 -v '^[[:space:]]*$' -- "$1" || true)
    [[ $first =~ ^[^\|\<]+\|[^\|]*\|[ADM]?\|[^\|]+ ]]
}

shopt -s nullglob
for f in "$REPO_ROOT"/tests/logs/*.log; do
    if is_custom_log "$f"; then
        add "$(basename "$f" .log)" "tests/logs/$(basename "$f")"
    else
        echo "skipping tests/logs/$(basename "$f"): not a custom-format commit log" >&2
    fi
done
shopt -u nullglob

for name in visual-hierarchy-activity dense-contributors deletion-recreation; do
    add "$name" "tests/fixtures/custom-log/$name.log"
done

# One year of history (2009-09-11 .. 2010-08-31: 1200 file events, up to 9
# contributors in the first month) played at 0.15 s per day, about a minute of
# video.
add gource-history @history "--start-date 2009-09-01 --stop-date 2010-09-01 --seconds-per-day 0.15"

# --- history log -----------------------------------------------------------
generate_history_log() {
    local src out_dir
    src=$(find_gource_src)
    [[ -d $src/.git ]] || die "'$src' has no .git directory (a secondary worktree?); set GOURCE_SRC to the main checkout"
    out_dir=$1
    mkdir -p "$out_dir"
    echo "generating custom log for $src @ $HISTORY_REV" >&2
    # safe.directory: the bind mount may belong to another uid than the container user.
    docker run --rm --init --user "$(id -u):$(id -g)" -e HOME=/tmp \
        -e GIT_CONFIG_COUNT=1 -e GIT_CONFIG_KEY_0=safe.directory -e GIT_CONFIG_VALUE_0='*' \
        -v "$src:/repo:ro" -v "$out_dir:/out" \
        "$REF_IMAGE" gource --output-custom-log /out/input.log --git-branch "$HISTORY_REV" /repo
    [[ -s $out_dir/input.log ]] || die "gource produced no custom log for $HISTORY_REV"
}

# --- run -------------------------------------------------------------------
WANTED=("$@")
wanted() {
    [[ ${#WANTED[@]} -eq 0 ]] && return 0
    local w
    for w in "${WANTED[@]}"; do [[ $w == "$1" ]] && return 0; done
    return 1
}

for w in ${WANTED[@]+"${WANTED[@]}"}; do
    known=0
    for n in "${NAMES[@]}"; do [[ $n == "$w" ]] && known=1; done
    [[ $known -eq 1 ]] || die "unknown scenario '$w'; known: ${NAMES[*]}"
done

ensure_image

SUMMARY=$(mktemp)
trap 'rm -f "$SUMMARY"' EXIT
failed=()

for i in "${!NAMES[@]}"; do
    name=${NAMES[$i]}
    wanted "$name" || continue
    out=$OUT_ROOT/$name
    echo "=== $name" >&2

    if [[ ${LOGS[$i]} == @history ]]; then
        mkdir -p "$out"
        generate_history_log "$out"
        log=$out/input.log
    else
        log=$REPO_ROOT/${LOGS[$i]}
    fi

    read -r -a extra <<<"${ARGS[$i]}"
    if "$REF_DIR/render.sh" "$log" "$out" ${extra[@]+"${extra[@]}"} | tee -a "$SUMMARY"; then
        :
    else
        failed+=("$name")
    fi
done

echo >&2
echo "=== summary" >&2
cat "$SUMMARY"
if [[ ${#failed[@]} -gt 0 ]]; then
    die "failed: ${failed[*]}"
fi
echo "frames under $OUT_ROOT" >&2
