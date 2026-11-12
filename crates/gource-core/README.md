# gource-core

The simulation: everything that decides what is on screen and where. No I/O, no GPU, no platform code. For the
same event stream, time steps and seed it produces the same scene everywhere, native or wasm.

## What it ports

| File | C++ source | Notes |
|---|---|---|
| `sim.rs`, `sim/` | `gource.cpp`: `Gource::update`, `logic`, `readLog`, `processCommit`, and the read side of `draw` (`mousetrace`, the frustum test) | `sim/extract.rs` produces the scene snapshot in Gource's draw order; `sim/input.rs` is `mouseMove`, `mouseClick`, `mouseWheel`, `keyPress` |
| `dirnode.rs` | `dirnode.cpp` (`RDirNode`) | The directory tree, its layout forces and the file rings, without the GL drawing |
| `tree.rs` | Globals of `gource.cpp`, `dirnode.cpp`, `file.cpp`: `gGourceDirMap`, `gGourceRemovedFiles`, `root`, `dir_bounds` | One owner instead of globals |
| `file.rs` | `file.cpp` (`RFile`) | |
| `user.rs` | `user.cpp` (`RUser`, `UserForceFunctor`) | |
| `action.rs` | `action.cpp` (`RAction`, `CreateAction`, `ModifyAction`, `RemoveAction`) | |
| `pawn.rs` | `pawn.cpp` (`Pawn`) | The state files and users share |
| `camera.rs` | `zoomcamera.cpp` (`ZoomCamera`) | Without the GL calls |
| `settings.rs`, `options.rs` | `gource_settings.cpp`: `setGourceDefaults`, `importGourceSettings` | Gource's option names, aliases, value syntax and defaults |
| `filters.rs` | `formats/commitlog.cpp`: `RCommit::addFile`, `RCommit::isValid` | `--file-filter`, `--user-filter` and their `show` forms |
| `datefmt.rs` | `core/settings.cpp`: `SDLAppSettings::parseDateTime`, and the `strftime` subset `--date-format` uses | |
| `hash.rs` | `core/stringhash.cpp`: `stringHash`, `vec2Hash`, `vec3Hash`, `colourHash` | Bit for bit against the x86-64 g++ build |
| `quadtree.rs` | `core/quadtree.cpp` (`QuadTree`, `QuadNode`) | |
| `frustum.rs` | `core/frustum.cpp`, `core/plane.cpp` | |
| `bounds.rs` | `core/bounds.h` (`Bounds2D`) | |
| `vectors.rs` | `core/vectors.cpp` | Including the two normalisations that round differently |
| `input.rs` | SDL events as `Gource` receives them | Host input in device pixels |

## What is new

| File | What |
|---|---|
| `events.rs` | The event stream: an append-only, string-interned store of commits. It replaces `RCommitLog` plus the commit queue, so a whole history can be seeked and replayed |
| `arena.rs` | Generational slots for directories, files, users and actions, instead of raw pointers |
| `rand.rs` | Where Gource calls `rand()`: a keyed, seeded stream per call site, so runs agree across machines. A glibc-compatible mode exists for comparison with the C++ |
| `view.rs`, `view_hash.rs` | The per-frame scene snapshot handed to the renderer, and a fingerprint of it for parity tests |
| `agents.rs`, `sim/attr.rs`, `style.rs` | AI co-authors: the agent table, attribution counts and the contributor style rules |
| `pull_request.rs`, `pr_style.rs` | Pull requests as events in the same stream |

## Tests against the C++

`tests/golden/` holds values printed by small C++ harnesses built from Gource's own sources (string hash,
frustum, directory layout, a full simulation scenario, glibc `rand()`); the Rust tests compare bit patterns.
`tests/golden/README.md` says how to regenerate them.

    cargo test -p gource-core

GPL-3.0-or-later, derived from Gource. See `LICENSING.md`.
