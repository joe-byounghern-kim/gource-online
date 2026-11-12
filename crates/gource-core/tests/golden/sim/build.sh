#!/bin/sh
# Rebuild the simulation golden table from Gource's C++ sources.
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
for f in dirnode file pawn user action zoomcamera; do
    cp "$src/$f.cpp" "$src/$f.h" "$work/"
done
for f in bounds.h quadtree.h quadtree.cpp frustum.h frustum.cpp plane.h plane.cpp \
         stringhash.h stringhash.cpp vectors.h vectors.cpp pi.h; do
    cp "$src/core/$f" "$work/core/"
done
# Stubs replace SDL, GL, fonts, textures, VBOs, splines and settings: the
# dirtree harness's, overridden by this harness's.
cp -R "$here"/../dirtree/stubs/. "$work/"
cp -R "$here"/stubs/. "$work/"

cd "$work"
cxx="g++ -std=c++17 -O2 -w -I$glm -I."
$cxx -DPORT_RAND_FILE='"dirnode"' -include port_rand.h -c dirnode.cpp -o dirnode.o
$cxx -DPORT_RAND_FILE='"user"' -include port_rand.h -c user.cpp -o user.o
$cxx -o sim_golden "$here"/sim_golden.cpp dirnode.o user.o action.cpp zoomcamera.cpp file.cpp pawn.cpp \
    core/quadtree.cpp core/frustum.cpp core/plane.cpp core/stringhash.cpp core/vectors.cpp
./sim_golden "$here/scenario.log" > "$here/../sim_golden.tsv"
echo "wrote $(wc -l < "$here/../sim_golden.tsv") lines to tests/golden/sim_golden.tsv"

# The same harness with glibc's own rand() (no port_rand.h): the reference
# for the port's `rand-mode glibc`.
$cxx -c dirnode.cpp -o dirnode-glibc.o
$cxx -c user.cpp -o user-glibc.o
$cxx -o sim_golden_glibc "$here"/sim_golden.cpp dirnode-glibc.o user-glibc.o action.cpp zoomcamera.cpp file.cpp pawn.cpp \
    core/quadtree.cpp core/frustum.cpp core/plane.cpp core/stringhash.cpp core/vectors.cpp
./sim_golden_glibc "$here/scenario.log" > "$here/../sim_golden_glibc.tsv"
echo "wrote $(wc -l < "$here/../sim_golden_glibc.tsv") lines to tests/golden/sim_golden_glibc.tsv"
