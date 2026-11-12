#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regenerates expected.txt: builds Gource's own custom-log reading code (the
# C++ reference: harness.cpp with the unmodified src/core/regex.cpp and
# src/core/timezone.cpp, PCRE2, the bundled utf8cpp, and the verbatim
# SDLAppSettings::parseDateTime) and runs it over every case in cases.txt.
# Run it after adding a case; the Rust test cpp_reference.rs compares the
# port against the result.
#
# With the argument `build` it only builds the harness, into
# target/cpp-reference/harness, and prints its path (for the ignored
# differential fuzz test, tests/cpp_fuzz.rs).
#
# Needs g++, python3, libpcre2-8 (the headers are optional: shim/pcre2.h
# stands in for them) and a Gource source tree whose src/core submodule is
# initialised (git worktrees do not have it; point GOURCE_SRC at one that
# does):
#
#     GOURCE_SRC=/path/to/Gource/src crates/gource-sources/tests/cpp-reference/regen.sh
#     GOURCE_SRC=/path/to/Gource/src crates/gource-sources/tests/cpp-reference/regen.sh build
set -eu

mode=${1:-regen}
case "$mode" in
    regen | build) ;;
    *) echo "usage: regen.sh [build]" >&2; exit 2 ;;
esac

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
src=${GOURCE_SRC:-$root/src}

if [ ! -f "$src/core/regex.cpp" ]; then
    echo "regen.sh: $src/core/regex.cpp not found; initialise the src/core submodule or set GOURCE_SRC" >&2
    exit 1
fi

if [ "$mode" = build ]; then
    work="$root/target/cpp-reference"
    mkdir -p "$work"
else
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
fi

# SDLAppSettings::parseDateTime from core/settings.cpp, as a free function in
# namespace ref. It needs SDL headers as a method, so it is cut out of the file.
{
    echo '#include "regex.h"'
    echo '#include "timezone.h"'
    echo '#include <string.h>'
    echo '#include <stdlib.h>'
    echo '#include <time.h>'
    echo 'namespace ref {'
    awk '/^bool SDLAppSettings::parseDateTime/ { p = 1 }
         p { sub(/SDLAppSettings::parseDateTime/, "parseDateTime"); print }
         p && /^}/ { exit }' "$src/core/settings.cpp"
    echo '}'
} > "$work/parse_date_time.inc"

if pkg-config --exists libpcre2-8 2>/dev/null; then
    cflags=$(pkg-config --cflags libpcre2-8)
    libs=$(pkg-config --libs libpcre2-8)
else
    cflags="-I$here/shim"
    libs="-l:libpcre2-8.so.0"
fi

# shellcheck disable=SC2086
g++ -std=c++17 -O1 -w $cflags -I"$src/core" -I"$work" \
    "$here/harness.cpp" "$src/core/regex.cpp" "$src/core/timezone.cpp" \
    -o "$work/harness" $libs

if [ "$mode" = build ]; then
    echo "$work/harness"
    exit 0
fi

# zone-less dates are local time in the reference: pin UTC
TZ=UTC python3 "$here/regen.py" "$work/harness" "$here/cases.txt" "$root" > "$here/expected.txt.new"
mv "$here/expected.txt.new" "$here/expected.txt"
echo "wrote $here/expected.txt"
