# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repository is

This is a fork of Gource (C++/OpenGL, GPL-3.0-or-later, v0.57). The goal of the fork is a
**Rust rewrite targeting WebAssembly** so the visualization runs in a modern browser (and
headless in the cloud). The C++ tree stays in place as the **reference implementation**:
the port must reproduce Gource's simulation and look, then extend it beyond git history to
issues, pull requests, and other project events.

Hard constraints for the port:

- The core visualization logic (tree layout forces, user/action/file behaviour, camera,
  time model, colour hashing, draw order) is Gource's. Port it faithfully first; restyle
  later. The section "Reference architecture" below is the map.
- **No third-party rendering engines** (no three.js, Babylon, PixiJS, bevy, etc.). Rendering
  is hand-written against the raw GPU API (WebGL2 via `web-sys` first; a WebGPU backend can
  sit behind the same trait). Batch everything, minimize draw calls and allocations.
- Input is an event stream, not a `git` subprocess. Browsers cannot run `git`; logs arrive as
  files, fetched from a service, or produced by a cloud worker. The existing pipe-delimited
  *custom log format* is the interchange baseline and must stay supported.
- Gource is GPL-3.0-or-later. The Rust port is a derivative work, so it must ship under a
  compatible licence.

A detailed implementation plan will be supplied by the maintainer; do not invent scope beyond
what it asks for.

## Reference implementation (C++)

### Build and test

`src/core` is a git submodule (acaudwell/Core: SDL app loop, display, fonts, quadtree, VBO,
shaders, textures). It is empty until initialized; almost every source file includes from it.

```
git submodule update --init --recursive     # required once
./autogen.sh                                # autoreconf + submodule init
./configure && make                         # produces ./gource
make check                                  # builds and runs ./gource_tests (Boost.Test)
./gource_tests --run_test=parse_date_time_tests   # single test case (also: regex_tests)
./gource_tests --log_level=all              # verbose
```

Dependencies: SDL2, SDL2_image (PNG+JPEG), PCRE2, FreeType2, GLEW, GLM, Boost.Filesystem
>= 1.69, libpng. `gource.pro` is a qmake file for Windows/MinGW only.

Visual regression scenarios live in `tests/test.conf` (multiple `[gource]` sections, each a
short run against `tests/logs/*.log`). Paths in the conf are relative to the working
directory, so run it from `tests/`:

```
cd tests && ../gource test.conf
```

Useful log tooling built into the binary:

```
gource --output-custom-log out.log <repo-or-log>   # normalize any supported log to the custom format
gource --log-command git                           # print the git log command Gource runs
```

Unit tests (`src/test/`) only cover regex and date parsing; behaviour is verified visually.

### Data pipeline

```
path / stdin
   └─ RLogMill (logmill.cpp, runs on an SDL thread)
        finds the VCS root, then tries RCommitLog subclasses in order
        (git, hg, bzr, gitraw, cvs-exp, svn, cvs2cl, custom, apache) until checkFormat() passes
   └─ RCommitLog (formats/commitlog.cpp)
        wraps a SeekLog (seekable file, percent-based seeking) or StreamLog (stdin)
        applies --file-filter / --user-filter regexes; assigns file colour by extension hash
   └─ RCommit { timestamp, username, files: [RCommitFile { filename, action "A"|"M"|"D", colour }] }
   └─ Gource::readLog  → std::deque<RCommit> commitqueue (max 100 ahead of currtime)
   └─ Gource::processCommit → RFile / RUser / RAction objects
```

Filenames are always rooted with a leading `/`. A filename ending in `/` is a directory and
is only meaningful with action `D` (deletes every file beneath it). The git parser runs
`git log --reverse --raw --no-renames --pretty=format:user:%aN%n%ct` and reads
`user:` / timestamp / `\t`-separated status lines.

Custom log format (one line per file change, the interchange format for the port):

```
timestamp|username|A/M/D|/path/to/file[|RRGGBB]
```

`timestamp` is unix seconds or ISO 8601; consecutive lines with the same timestamp and user
form one commit. Captions use `timestamp|text` (`--caption-file`).

### Simulation model (per frame, `Gource::logic`)

Time: `runtime += min(dt, 1/60) * time_scale`; simulation clock
`currtime += dt * 86400 * days_per_second`. Commits are popped from the queue while
`commit.timestamp <= currtime`. If every user is idle for `auto_skip_seconds` (3 s) the clock
jumps to the next commit. Non-monotonic timestamps rewind `currtime` unless
`--no-time-travel`. When exporting video, dt is fixed to the frame rate.

Entities (all live in a 2D plane at z=0; the camera is a perspective camera looking down -z):

- **RDirNode** (`dirnode.cpp`, the heart of the layout). A tree keyed by absolute path in the
  global `gGourceDirMap`. The root is at the origin and is re-rooted upward when a file
  outside the current root appears (`addFile` forks a new parent; `Gource::addFile` then
  walks `root` up). Nodes are reaped when they have no files and no children. Files that turn
  out to be directories are force-removed.
  - Radius: `dir_radius = max(1, sqrt(area)) * 1.5`, where area is the sum of visible file
    disc areas (file diameter 8) plus child areas.
  - Forces (`applyForces`): overlap repulsion from non-related nodes found via the quadtree
    (and from the parent, checked unconditionally), gravity `10 * distanceToParent` toward
    the parent's rim, a push along the grandparent→parent direction to
    `parent.radius + radius`, and repulsion from visible siblings scaled by
    `(parent.radius * π) / (visible children + 1)`. `move()` integrates `pos += accel * dt`
    (optional elasticity term) and clears accel each frame.
  - Initial position: parent position plus a unit vector from `vec2Hash(path)` blended with
    the parent edge direction. `spos` is a lagging midpoint used as the spline control point
    for the edge to the parent.
  - Files are placed on concentric rings around the node (`calcFileDest`): the centre holds
    one file, ring `k` sits at distance `k * file_diameter` and holds `floor((k+1) * π)`
    files, evenly spaced by angle. File positions are relative to their node.
  - Colour: average of visible file colours, brightened for 3 s after a change.
- **RFile** (`file.cpp`): moves toward its ring slot with speed 5 and no momentum. Colour is
  `colourHash(extension)` (white if none). `touch()` flashes the action colour for 1 s,
  shows the name for `filename_time` (4 s), un-hides it. Fades out over 1 s after
  `file_idle_time` of inactivity or after a RemoveAction completes, then is deleted next
  frame via `gGourceRemovedFiles`.
- **RUser** (`user.cpp`): holds pending `actions` and `activeActions`. A pending action
  becomes active when the user is within beam distance (100) of the file, or immediately
  once it is older than `max_file_lag` (5 s). Forces: repulsion from other users (personal
  space 100, reduced when busy), attraction/repulsion to up to 3 active action targets
  (desired 50, pulled in beyond 100). Velocity is capped at `max_user_speed` (500) and
  damped by `user_friction`. Users fade after `user_idle_time` (3 s) and are deleted after
  10 s idle. Avatar size 20 × `user_scale`; colour `colourHash(name)` mixed 60/40 with white.
- **RAction** (`action.cpp`): Create (green), Modify (orange, then sets file colour), Remove
  (red, removes file on completion). On first tick it `apply()`s to the file; progress
  advances at `min(10, rate * max(1, pending_count))` per second (rate 0.5, or 2.0 when
  activated by `max_file_lag`); drawn as a beam quad from user to file that fades with
  progress.
- **Spatial index**: two `QuadTree`s (dirs, users) are rebuilt from scratch every frame,
  max depth 1 until `dir_bounds.area() > 10000`, then 6. They serve force pair-finding and
  mouse picking.
- **ZoomCamera** (`zoomcamera.cpp`): fov 90°, up = (0,-1,0), starts at z = -300. Each frame
  it targets the centre of a bounds box (all dirs in overview mode; active users or the
  selected entity in track mode), with distance `max(w,h) * padding(1.1) / (2 tan(fov/2))`
  after correcting the shorter side for the viewport aspect ratio, clamped to [50, 10000],
  and lerps toward it. Auto-rotation spins the whole scene 90° when
  the bounds aspect ratio drops below 0.67. Manual drag/zoom/rotate override.
- **Colour hashing** (`core/stringhash.cpp`): `stringHash` with global seed 31,
  `colourHash = normalise(vec3(h/7 % 255, h/3 % 255, h % 255))`. Pressing `S` reseeds.
  Reproduce exactly (including `char` signedness quirks) for identical colours.

Defaults for every tunable are in `GourceSettings::setGourceDefaults()`
(`gource_settings.cpp`); the CLI/conf parser in the same file maps option names to them.

### Render pass (per frame, `Gource::draw`)

1. Clear to background colour; optional background image (2D).
2. Mouse trace against the quadtrees (hover/select).
3. Perspective projection, `camera.focus()`. Frustum-cull dir nodes. Project every node, node
   spline point, file label and user label to screen space (`gluProject`) once.
4. `updateVBOs`: rebuild the file, user, action and bloom vertex buffers from scratch.
5. `drawScene`, in this order and all alpha-blended unless noted:
   1. **Edges**: drawn in 2D screen space from the projected positions as textured quad
      strips along a quadratic spline (parent → spos → child), 2.5 px half-width, beam
      texture; a shadow pass offset (2,2) first.
   2. File shadows (shadow shader, offset 2,2), then action beams, then files.
   3. User shadows, then users.
   4. **Bloom**: additive (`GL_ONE, GL_ONE`) quads per visible dir node of radius
      `dir_radius * 2 * bloom_multiplier`, coloured `node_colour * bloom_intensity`. The
      bloom shader computes the glow procedurally from the quad centre/radius packed into the
      texcoord (see `data/shaders/bloom.frag`); the fixed-function fallback uses
      `bloom.tga`.
6. Back to 2D: directory names (at the spline label position), file names and user names
   through the batched font VBO with the text shader (drop shadow via a texel offset);
   selected names drawn last on top.
7. HUD: logo, splash, date (top centre), title (bottom left), captions, file-extension key,
   position slider, hover text box, debug overlay.

Vertex formats: `quadbuf_vertex` = pos vec2, colour vec4, texcoord vec2 (32 bytes) with
texture-change ranges; `bloom_vertex` = pos vec2, colour vec4, texcoord vec4 (padded to
64 bytes). Legacy GLSL 1.x shaders (`gl_Vertex`, `ftransform`) live in `data/shaders/`; they
must be rewritten for WebGL2/WebGPU, but the maths in `bloom.frag` and `text.frag` is the
look to preserve. Textures: `file.png`, `user.png`, `beam.png`, `bloom*.tga`, plus per-user
avatar images from `--user-image-dir`.

Layers above the scene: `GourceShell` (`gource_shell.cpp`) owns one `Gource` per `[gource]`
conf section, cross-fades between them, and handles fullscreen/resize by unloading and
reloading every GL resource. `main.cpp` turns argv into a `ConfFile`, applies it, and can
exit early after `--save-config` or `--output-custom-log`.

## Rust port workspace

The Cargo workspace lives beside the untouched C++ tree. The toolchain is pinned in
`rust-toolchain.toml` (1.98.1 with the `wasm32-unknown-unknown` target, clippy and rustfmt);
`rustup` installs it on first use.

```
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --workspace                                    # native build of every crate
cargo test --workspace                                     # all tests
cargo test -p gource-core <name_substring>                 # one test
cargo build -p gource-web --target wasm32-unknown-unknown  # browser build (cdylib)
cargo run -p gource-cli                                    # native runner (binary: gource-rs)
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p gource-web --target wasm32-unknown-unknown -- -D warnings
```

The player (`apps/player/`, Vite + TypeScript; Node 22 and pnpm) and scripts:

```
scripts/build-web.sh [--release|--dev]  # gource-web -> wasm-bindgen (CLI 0.2.129) -> packages/viewer/src/wasm/
scripts/dev.sh                          # wasm + the player's Vite server on http://127.0.0.1:5174
scripts/serve.sh                        # wasm + apps/player/dist served statically with its headers (vite preview) on :8787
scripts/build-cli.sh [--pack]           # wasm + apps/player/dist -> apps/cli/dist (+ apps/cli/gource-online-<version>.tgz)
pnpm install && pnpm -C apps/player typecheck && pnpm -C apps/player build
pnpm -C apps/player test                     # unit tests (Node, no browser)
pnpm -C apps/player test:e2e                 # Playwright in system Chrome: placeholder engine, built player on real origins, real engine
E2E_CHANNEL=chromium pnpm -C apps/player test:e2e   # the same in Playwright's bundled Chromium (CI; no Chrome on Linux arm64)
GOURCE_PLAYER_TEST_BUILD=1 VITE_E2E_HOOKS=1 scripts/serve.sh
E2E_REAL=1 E2E_BASE_URL=http://127.0.0.1:8787 pnpm -C apps/player exec playwright test tests/viewer-real.spec.ts tests/ai-real.spec.ts tests/pr-real.spec.ts tests/record-real.spec.ts
pnpm install && pnpm -C apps/cli typecheck && pnpm -C apps/cli test   # the npx command (node:test)
E2E_CLI=1 pnpm -C apps/player exec playwright test tests/cli-real.spec.ts  # the packed CLI via npx + real engine (build-cli.sh --pack)
```

The player's address: `#src=<log URL>&title=<text>&o.<option>=<value>` plays a log (the protocol is in
`packages/protocol/src/player.ts`); a page may hand one over by postMessage; a local file can be opened or
dropped on it. In a test build `?mock=1` uses the Canvas2D placeholder engine and `?fixed-dt=<s>` makes the
simulation advance only through `window.__gource.step(n)`. A build fixes which origins may send it a log and
which it may fetch one from (`apps/player/README.md`), and enforces that in its CSP.

Crates (all `publish = false`, GPL-3.0-or-later, edition 2024):

- `gource-core` — event model, directory tree and forces, users, actions, files, camera,
  clock, colour hash. No I/O, no GPU, no platform dependencies. Deterministic for a given
  event stream, dt sequence and hash seed.
- `gource-sources` — adapters from bytes to the core event stream: custom log, git `--raw`
  output, issue and pull-request events. Never spawns processes.
- `gource-render` — backend-agnostic frame batches (the `updateVBOs` equivalent), draw-pass
  order, and the backend trait. No GPU calls.
- `gource-render-webgl2` — the backend trait implemented with raw `web-sys` WebGL2 calls.
  Compiles to an empty crate on non-wasm targets.
- `gource-web` — `wasm-bindgen` API (PLAN section 4): `GourceApp` (event log, simulation,
  frame builder and WebGL2 backend on one canvas; real-time or fixed-dt `step`) and
  `LogParser` (Web Worker: custom-log bytes to encoded batches). The engine is generic over
  backend and clock and is tested natively.
- `gource-cli` — native runner for headless runs, frame dumps and parity checks against the
  C++ build.
- `gource-mux` — H.264 MP4 muxer for the browser video export; no dependencies, no I/O.
- `packages/protocol` — `@gource-online/protocol` (MIT): repository URL rules and vectors, API
  types, feature registry, Gource's git log command. No runtime dependencies.
- `packages/viewer` — `@gource-online/viewer` (GPL): `<gource-viewer>`, the canvas with the wasm
  engine on it, its pointer/key input and render loop, the log parser worker (`LogLoader`) and
  Gource's option definitions. Pages add their controls around it (`packages/viewer/README.md`).

`apps/cli/` is the `gource-online` npm package (plain ESM JavaScript with JSDoc types, no runtime
dependencies, Node 18.18+): `npx gource-online` in a checkout runs Gource's git command (pinned
to `PRETTY_AI` by a test), keeps the output in memory and serves it with the bundled player
(`apps/cli/dist`, gitignored, from `scripts/build-cli.sh`) on 127.0.0.1 at a per-run token path, then
opens the browser at the player's own link to it. One version covers every public artifact
(`scripts/check-versions.mjs`); `.github/workflows/release.yml` publishes them on `v*` tags.

The hosted service's clone service is a separate, private program that implements
`packages/protocol/src/api.ts`; this repository contains the player, the engine, the CLI and the
contract. Pull requests are event kinds in the same stream (`ChangeEvent`, `ev=` in the custom
log's meta field).

`tests/fixtures/custom-log/` holds small edge-case inputs (equal timestamps, duplicates,
unordered and negative timestamps, unicode paths, malformed lines); `tests/logs/` holds the
C++ visual scenarios. Both are inputs for the port's tests. `packages/protocol/vectors/repo-urls.json`
holds the URL/branch/date/path vectors every client and clone service must pass.

Guiding rules for the port:

- Keep the `Gource::logic` / `Gource::draw` split: simulation never touches the GPU; the
  renderer only consumes the frame builder's batches.
- Replace Gource's `rand()` calls (overlap resolution, colour reseeding) with seeded,
  keyed hashing so browser and cloud runs agree.
- Model "issue opened", "PR merged", etc. as new event kinds in the same stream as commits
  rather than a parallel system, so they flow through the same time model, queue and
  auto-skip logic.
- Text rendering needs a glyph atlas built from `data/fonts/FreeSans.ttf` (or the browser's
  canvas as a rasterizer); do not pull in a scene-graph or UI framework for it.
