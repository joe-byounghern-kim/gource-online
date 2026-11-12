#!/bin/sh
# Rebuild the dirtree golden table from Gource's C++ sources.
#
#   GOURCE_SRC   Gource's src/ directory with the Core submodule initialised
#                (default: the main checkout's src/)
#   GLM_DIR      directory containing glm/glm.hpp (default /tmp/glm-1.0.1)
set -eu
here=$(cd "$(dirname "$0")" && pwd)
src=${GOURCE_SRC:-$here/../../../../../src}
glm=${GLM_DIR:-/tmp/glm-1.0.1}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/core"
cp "$src"/dirnode.cpp "$src"/dirnode.h "$src"/file.cpp "$src"/file.h \
   "$src"/pawn.cpp "$src"/pawn.h "$work/"
for f in bounds.h quadtree.h quadtree.cpp frustum.h frustum.cpp plane.h plane.cpp \
         stringhash.h stringhash.cpp vectors.h vectors.cpp pi.h; do
    cp "$src/core/$f" "$work/core/"
done
# Stubs replace SDL, GL, fonts, VBOs, splines and settings.
cp -R "$here"/stubs/. "$work/"

cd "$work"
cxx="g++ -std=c++17 -O2 -I$glm -I."
$cxx -include port_rand.h -c dirnode.cpp -o dirnode.o
$cxx -o dirtree_golden "$here"/dirtree_golden.cpp dirnode.o file.cpp pawn.cpp \
    core/quadtree.cpp core/frustum.cpp core/plane.cpp core/stringhash.cpp core/vectors.cpp
./dirtree_golden > "$here/../dirtree_golden.tsv"
./dirtree_golden frustum > "$here/../frustum_golden.tsv"
echo "wrote $(wc -l < "$here/../dirtree_golden.tsv") lines to tests/golden/dirtree_golden.tsv"
echo "wrote $(wc -l < "$here/../frustum_golden.tsv") lines to tests/golden/frustum_golden.tsv"
