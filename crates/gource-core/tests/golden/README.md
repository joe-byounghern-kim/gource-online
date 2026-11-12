# Golden values for `gource-core`

`stringhash_golden.cpp` is a standalone copy of Core's `stringHash`,
`vec2Hash`, `vec3Hash`, `colourHash` and `normalise` (from
`src/core/stringhash.cpp` and `src/core/vectors.cpp`). It prints one row per
`(seed, string)` pair: the seed, the string as hex bytes (`-` for the empty
string), the integer hash, and the IEEE-754 bit patterns of every float the
vector hashes return.

`stringhash_golden.tsv` is its output. `src/hash.rs` loads the table with
`include_str!` and asserts that the Rust port reproduces every value bit for
bit (`cargo test -p gource-core hash`).

## Regenerating

The harness needs only g++ and the header-only GLM library:

```
curl -sSL https://github.com/g-truc/glm/archive/refs/tags/1.0.1.tar.gz | tar xz -C /tmp
cd crates/gource-core/tests/golden
g++ -std=c++17 -O2 -I/tmp/glm-1.0.1 stringhash_golden.cpp -o /tmp/stringhash_golden
/tmp/stringhash_golden > stringhash_golden.tsv
```

The table was generated on x86-64 with g++ 13 at `-O2` (Gource's default
build flags), where `char` is signed. `-O0`, `-O1` and `-O2 -fwrapv` give the
same table except for the rows whose hash is `INT_MIN` (seed `-2147483647` and
`16777217`): at `-O2` g++ inlines `stringHash`, assumes `-val` never overflows
and divides the hash as unsigned, which the port reproduces (see
`src/hash.rs`). To add cases, extend
the `strings` or `seeds` lists in the harness and regenerate.

# Directory tree layout (`dirtree_golden.tsv`)

`dirtree/` compiles Gource's own `dirnode.cpp`, `file.cpp`, `pawn.cpp` and
Core's `quadtree`, `frustum`, `plane`, `stringhash` and `vectors` against
no-op stubs for SDL, GL, fonts, VBOs, splines and settings (`dirtree/stubs/`),
then replays a scenario: fourteen files (the first one renames the root, the
second forks a new root), 300 frames at 1/60 s, a remove at frame 100, a
rotation at frame 150 and two late additions at frame 200, in three variants
(`plain`, `elastic` with `--elasticity 0.5`, `idle` with
`--file-idle-time 1.5`). At fixed frames it prints the bit patterns of every
dir's `pos`, `spos`, `dir_radius` and colour and every file's absolute and
relative position, alpha and colour.

`src/tree_tests.rs` (`layout_matches_cpp_golden`) replays the same scenario
through `DirTree` and requires every row to match bit for bit.

The one intended difference from a stock Gource build: `rand()` in
`dirnode.cpp` (overlap resolution, which every frame-0 layout hits because
all nodes start at the origin) is replaced through `stubs/port_rand.h` by the
port's keyed stream, so both sides draw the same numbers. g++ evaluates the
two `rand()` arguments of `vec2(...)` right to left, which the port mirrors.

Regenerate with (needs Gource's `src/` with the Core submodule initialised):

```
GOURCE_SRC=/path/to/Gource/src GLM_DIR=/tmp/glm-1.0.1 \
    crates/gource-core/tests/golden/dirtree/build.sh
```

The same build writes `frustum_golden.tsv` (`dirtree_golden frustum`): the six
planes of Core's `Frustum` for four cameras on the stub 1280x720 display, and
`intersects` / `contains` results for a grid of boxes, checked by
`frustum::tests::matches_cpp_golden`.

The output is identical at `-O0`, `-O2` and `-O3`.

# Simulation (`sim_golden.tsv`)

`sim/` compiles Gource's own `user.cpp`, `action.cpp`, `zoomcamera.cpp`,
`dirnode.cpp`, `file.cpp` and `pawn.cpp` (plus Core's quadtree, frustum,
plane, stringhash and vectors) against the dirtree stubs extended by
`sim/stubs/` (textures with the sizes of `file.png` and `user.png`, a
1280x720 display, fonts without metrics). `sim/sim_golden.cpp` drives them
with the per-frame part of `gource.cpp` copied nearly verbatim: `readLog`,
`processCommit`, `addFileAction`, `addFile`, `addUser`, `deleteFile`,
`deleteUser`, `updateBounds`, `interactUsers`, `updateUsers`,
`interactDirs`, `updateDirs`, `updateCamera`, `logic` and `update`, minus
GL, input, captions and the file key. It replays `sim/scenario.log` (100
lines: an import, modifications with an explicit colour, deletes, a
multi-day gap that triggers auto-skip, a directory delete, a time-travel
commit, seven users on one file, a file that turns into a directory, a deep
path) for 2400 frames at 1/60 s. Every 20 frames it prints the clock
(`currtime`, `lasttime`, `subseconds`, `idle_time`, camera position,
`rotation_remaining_angle`, queue length, log position) and every user
(position, acceleration, alpha, colour, `action_interval`, list lengths and
the `actionCount` / `activeCount` counters); every 100 frames also every dir
and file.

`src/sim/tests.rs` (`simulation_matches_cpp_golden`) replays the scenario
through `Simulation::update` and requires every row to match bit for bit.
The run includes an automatic 90 degree rotation of the scene.

`rand()` in `dirnode.cpp` and `user.cpp` is replaced through
`sim/stubs/port_rand.h` by the port's keyed streams, one per call site,
keyed `"<file>.<function>"` (`dirnode.applyForceDir`,
`user.applyForceUser`, `user.applyForceAction`).

`build.sh` also links the harness a second time without `port_rand.h`, so
`dirnode.cpp` and `user.cpp` call glibc's own unseeded `rand()` as in a stock
build, and writes `sim_golden_glibc.tsv`.
`simulation_matches_cpp_golden_with_glibc_rand` replays the scenario with
the hidden option `rand-mode glibc` (one shared emulation of glibc's
`rand()`, `src/rand.rs`) and requires the same bit-for-bit match. The
scenario draws from `applyForceDir` and `applyForceUser` interleaved, so
the shared call order is covered too.

# glibc `rand()` (`glibc_rand_1000.txt`)

The first 1000 values of `rand()` without `srand` from glibc 2.39 on x86-64,
printed by `scripts/parity/glibc-rand.c` (see the comment there for the
command). `rand::tests::matches_glibc_rand` checks `GlibcRand` against it.

Regenerate with:

```
GOURCE_SRC=/path/to/Gource/src GLM_DIR=/tmp/glm-1.0.1 \
    crates/gource-core/tests/golden/sim/build.sh
```

`sim/gen_scenario.py` writes `scenario.log`; change the script, rerun it in
`sim/`, then regenerate the table.

# AI decorations (`deco_golden.tsv`)

Port-only, no C++ counterpart. `gource-cli`'s `ai_tests::deco_golden` plays
`tests/fixtures/custom-log/ai-coauthors.log` through the real custom-log
parser for 900 frames at 1/60 s in three option sets (defaults; viewer rules
with the agent key; `ai-style=hat` with `decor-motion=off`) and records
`hash_view` every 30 frames, including its `DECO`, `AKEY` and `FNOT`
sections. Regenerate after an intended change with
`GOURCE_BLESS=1 cargo test -p gource-cli deco_golden`.
