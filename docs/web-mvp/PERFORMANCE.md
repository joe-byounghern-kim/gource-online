<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Web MVP performance

> **History.** A design record from the MVP. Mentions of `gource-server` refer to the original
> Rust clone service, which is not part of this repository; the hosted service implements the
> same API (`packages/protocol/src/api.ts`).

How fast the Rust/WebAssembly port runs in Chrome compared with the native
port (`gource-rs`) and the C++ Gource, what was changed to make it faster,
and where the remaining limits are. Measured on branch `ws-perf` (from
`web-mvp` at `0b264ef`), 2026-10-02.

## Summary

- **Per frame, the browser build is faster than the C++ Gource on the same
  machine and the same frames.** On the git/git history it spends 1.7 ms of
  main-thread CPU per frame (C++ with the GPU: 3.0 ms). With a 100 000-file
  tree it spends 14.9 ms (ANGLE GL) to 18.0 ms (ANGLE Vulkan); the C++ needs
  39 ms there (logic 15 ms + draw 24 ms).
- **Real time:** git/git plays at a locked 60 fps (0 dropped frames in 60 s,
  1.6 ms CPU per frame). Synthetic trees hold 60 fps up to **75 000 visible
  files** with ANGLE GL (Chrome's default on Linux) and somewhere between
  25 000 (60 fps, 0 dropped) and 50 000 (57 fps) with ANGLE Vulkan; at 100 000 files the page runs at 54 fps (GL) / 40 fps
  (Vulkan).
- **Changes in this workstream** (all behaviour-preserving, view hashes
  unchanged on every fixture, scenario and benchmark log): browser frame time
  at 100k files 16.8 → 14.9 ms (GL) and 22.3 → 18.0 ms (Vulkan); git/git
  2.18 → 1.73 ms. Frame building is 32–43 % faster, the simulation 5–9 %, and
  vertex uploads with ANGLE Vulkan 24 % faster.
- **Loading:** first frame 0.35–0.45 s after navigation for every log size;
  the wasm parser runs at ~230 MiB/s. A 36 MB, 150 000-commit log (the
  server's commit cap) is complete after ~1.5 s, limited by gource-server's
  on-the-fly gzip, not by the page. git/git through the clone service: first
  frame after 10.8 s cold (clone + history), 0.97 s from the cache.
- **What limits large trees now** is data volume, not code paths: ~20 MiB of
  vertex data per frame at 100k files (the same 32-byte vertex format as the
  C++), which Chrome's GPU process must copy (5.5 ms with GL, 8.6 ms with
  Vulkan, after the 3.3 / 6.4 ms the page spends handing it over), and
  memory-bound passes over 100k file records in the simulation.

## Machine and software

| | |
|---|---|
| CPU | Intel Core i7-11700KF (8 cores / 16 threads, 3.6 GHz base), 32 GB RAM |
| GPU | NVIDIA GeForce RTX 3080 Ti, 12 GB, driver 595.71.05 |
| OS | Ubuntu 24.04.5, kernel 7.0.0-34 |
| Browser | Google Chrome 154.0.8037.57, headless (`--headless=new` via Playwright), viewport 1280×720 at DPR 1 |
| Rust | 1.98.1, release profile (`opt-level = 3`, `lto = "fat"`, `codegen-units = 1`), wasm-bindgen 0.2.129 |
| C++ | Gource 0.57 from `src/`, Ubuntu 24.04 g++ (`scripts/reference/` image) |

The host is shared: a CarlaUE4 simulator used ~1.8 cores throughout and
other agents compiled in parallel. Timings are therefore medians of
repeated runs where noted, A/B comparisons alternate the builds, and native
A/B comparisons also report instruction counts (`perf stat`), which load
does not move. Run-to-run noise is about ±5 %.

Chrome's GPU flags: `--use-angle=gl --ignore-gpu-blocklist --enable-gpu`
("ANGLE GL", what Chrome uses by default on Linux) and
`--use-angle=vulkan --enable-features=Vulkan --ignore-gpu-blocklist
--enable-gpu` ("ANGLE Vulkan"). Both report the RTX 3080 Ti as the
renderer. Without cross-origin isolation `performance.now()` has 0.1 ms
resolution in the page, which is why browser timings end in tenths.

## Inputs

| Log | How it is made | Size | Commits | Played as | Peak entities |
|---|---|---|---|---|---|
| **git/git** | `git clone --bare --filter=blob:none https://github.com/git/git`, Gource's log command (`git log --reverse --raw --no-renames … --pretty=format:user:%aN%n%ct`), `gource-rs convert --from git` | 6.8 MB | 61 022 git commits with file changes = 43 312 custom-log commits (same second + author merge), 138 576 changes | `seconds-per-day=0.01`, 5000 frames = the whole history 2005–2026 | 6 076 files, 709 users, 244 dirs |
| **stress** (100k) | `scripts/perf/gen-stress-log.py` (defaults) | 6.5 MB | 12 000 over 5 days | defaults (10 s/day), 3000 frames = the whole log | 100 000 files, 473 users, 2 631 dirs |
| stress75k / stress50k | `gen-stress-log.py --files 75000` / `50000` | 5.3 / 3.7 MB | 12 000 | defaults | 75 000 / 50 000 files |
| cap150k | `gen-stress-log.py --files 60000 --users 2000 --commits 150000 --days 7300` | 36 MB | 150 000 (the server's cap) | load tests only | — |

The stress generator is deterministic (same arguments, same bytes): files
are spread over ~2 600 directories up to five levels deep, most commits add
~10 files to one directory, a few prolific users and a long tail. It is a
deliberate worst case: every file stays visible (Gource's default
`file-idle-time` is 0), and with 4 s file-name labels about 8 400 labels are
on screen at the end. The logs live in the scratch directory, not in the
repository; the commands above recreate them.

## Method

**Native (`gource-rs bench`).** Loads the whole log, then runs N frames at
dt = 1/60 s with a 1280×720 viewport, timing `Simulation::update`,
`Simulation::extract_view` and `FrameBuilder::build` separately (the build
uses a monospace glyph stand-in, no GPU). This workstream added
`--options` (Gource options as a URL query, as the page takes them) and the
peak entity counts. `scripts/perf/native-suite.sh` runs each log three
times per build, alternating builds, and reports the median run.

**Browser, fixed-dt (`browser-bench.mjs --mode fixed`).** Opens
`/?log=/perf/<log>&fixed-dt=1/60` on a local `gource-server --static-dir`,
waits until the whole log is parsed, then calls `window.__gource.app.step(1)`
N times in one page task (yielding every 120 ticks) and reads the new
`EngineStatus.last_sim_ms / last_build_ms / last_gpu_ms` after each tick.
These are exactly the frames the native bench runs (same log, dt, viewport,
seed; the final entity counts and simulation time match native). `sim` =
update + extract_view, `build` = frame building with the real canvas glyph
atlas, `gpu submit` = the WebGL calls of `draw_frame` on the main thread.
`--finish` adds a 1×1 `readPixels` after each tick, which waits for Chrome's
GPU process to execute the frame (`gl.finish()` does not block in Chrome).
For builds without `last_*` (the baseline) the per-tick value is recovered
exactly from the moving averages read just before and after the tick
(weight 0.1).

**Browser, real time (`--mode realtime`).** Normal playback for 60 s after
the log is complete: every `requestAnimationFrame` interval, the CPU time of
each frame, draw calls, vertices, uploaded bytes, JS heap and wasm memory
(new `GourceApp.memory_bytes()`). "Dropped" counts intervals over 20.7 ms
(a missed vsync at 60 Hz).

**C++ Gource (`scripts/perf/cpp-timing/`).** `instrument.py` patches a
temporary copy of `src/gource.cpp` (the repository's C++ tree is not
touched) to time `logic()` and `draw()` per frame with `steady_clock`, and
optionally `glFinish()`. The image builds on `gource-reference:0.57`. Runs
use `--output-ppm-stream /dev/null --output-framerate 60`, which makes
Gource tick at a fixed 1/60 s like the port (the frame read-back is outside
the timed region), `--viewport 1280x720`, and the first N frames. Two GL
stacks:
- *llvmpipe*: Xvfb + Mesa software GL (fair for `logic()`, `draw()` is a CPU
  rasteriser);
- *NVIDIA*: Docker CDI (`--device nvidia.com/gpu=all`), SDL's `offscreen`
  video driver with the NVIDIA EGL vendor library — real GPU rendering
  (frames verified non-blank). This worked on the second attempt (an empty
  `SDL_VIDEO_EGL_DRIVER` broke EGL loading; `docker run --init` is needed so
  `xvfb-run` gets its ready signal).
The C++ entity counts at the last frame match the port's within a few files
(frame alignment differs by a tick or two).

**Profiles.** Native: `scripts/perf/native-profile.sh` runs `perf record`
in a container with `CAP_PERFMON` (the host's `perf_event_paranoid` is 4)
and writes a flame graph. Browser: `--profile` records a CDP CPU profile at
100 µs sampling against a build that keeps the wasm name section
(`stage-site.sh --profiling`).

## Results

### Same frames: CPU time per frame (ms, mean / p95)

| | git/git (5000 frames, 0.01 s/day) | stress 100k (3000 frames) |
|---|---|---|
| **Browser, ANGLE GL** (after) | **1.73** / 2.9 (sim 1.02, build 0.46, submit 0.25) | **14.92** / 20.9 (sim 8.84, build 2.78, submit 3.29) |
| **Browser, ANGLE Vulkan** (after) | **2.00** / 5.3 (sim 1.04, build 0.48, submit 0.48) | **18.04** / 27.2 (sim 8.82, build 2.78, submit 6.44) |
| Browser, ANGLE GL (baseline) | 2.18 / 3.6 | 16.76 / 22.8 |
| Browser, ANGLE Vulkan (baseline) | 2.39 / 4.8 | 22.27 / 32.6 |
| Native Rust `gource-rs bench` (after) | 0.93 / 1.6 (update 0.66, extract 0.07, build² 0.19) | 9.61 / 13.6 (update 5.87, extract 1.98, build² 1.76) |
| Native Rust (baseline) | — (no `--options` yet) | 9.89 / 13.8 (update 6.16, extract 1.97, build² 1.76) |
| C++ Gource, NVIDIA GPU | 3.04 / 4.6 (logic 1.75, draw 1.29) | 39.10 / 60.4 (logic 15.13, draw 23.96) |
| C++ Gource, llvmpipe | 9.02 / 13.7 (logic 2.20, draw 6.82) | 88.13 / 177.8 (logic 19.06, draw 69.07)¹ |

² Native frame building uses the monospace glyph stand-in, no atlas and no GPU.

¹ This run overlapped with native benchmarks on the same host, so its
logic time is pessimistic; the NVIDIA run (quiet host) gives 15.1 ms.

GPU-side completion with a per-frame sync (`--finish` / `FINISH=1`), stress
100k: the browser's GPU process needs another 5.5 ms (GL) / 8.6 ms (Vulkan)
after the submit; the C++ `glFinish()` waits 7.0 ms. Without the sync these
overlap with the next frame's CPU work.

How to read it: per frame the wasm simulation is 1.2–1.4× the native Rust
one (sim 8.84 vs update + extract 7.85 ms at 100k files, 1.02 vs 0.74 ms on git/git), and both are far ahead of the
C++ `logic()` (15 ms), mostly thanks to the arena/quadtree structures of the
port (no `std::map`/`std::list` walks, no per-frame allocation). Frame
building in the browser costs more than the native bench because it uses
the real glyph atlas instead of the monospace stand-in.

### Real-time playback, 60 s, NVIDIA GPU

| Log | Backend | fps after (baseline) | dropped frames after (baseline) | CPU/frame mean after (baseline) |
|---|---|---|---|---|
| git/git, 0.01 s/day | GL | **60.0** (60.0) | 0 (0) | 1.63 ms (2.05) |
| git/git, 0.01 s/day | Vulkan | **60.0** (60.0) | 0 (0) | 1.72 ms (2.09) |
| stress 50k | GL | **60.0** (60.0) | 0 (0) | 7.07 ms (8.06) |
| stress 50k | Vulkan | 56.8 (54.8) | 191 (312) | 7.31 ms (8.19) |
| stress 75k | GL | **60.0** (59.1) | 2 (55) | 11.90 ms (13.17) |
| stress 75k | Vulkan | 46.5 (43.6) | 809 (981) | 10.58 ms (11.69) |
| stress 100k | GL | 54.3 (51.3) | 342 (521) | 15.69 ms (17.25) |
| stress 100k | Vulkan | 40.5 (37.6) | 1152 (1304) | 12.82 ms (14.27) |

Always 10 draw calls per frame (9 when a pass is empty), whatever the
entity count. Uploaded bytes per frame: 3.2 MiB mean on git/git, 19.6 MiB at
100k files (peak 28 MiB). JS heap does not grow during playback (−1 to
−4 MiB, GC); wasm linear memory grows with the entities only: +10 MiB over
git/git, +109 MiB to 100k files (133 MiB total), and is reused after that.
(The baseline build has no `memory_bytes()`, hence no memory column for
it.)

With Vulkan the page drops frames while its CPU time is still well under
16.7 ms (e.g. 7.3 ms at 50k files): the bottleneck is Chrome's GPU process
executing the uploads, not the page.

### Loading and time to first frame

| Log | First frame | Log complete | Notes |
|---|---|---|---|
| git/git (6.8 MB), `?log=` | 0.39 s | 0.54 s | |
| stress 100k (6.5 MB), `?log=` | 0.39 s | 0.54 s | |
| cap150k (36 MB, 150k commits), `?log=` | 0.35–0.43 s | 1.47–1.56 s | gzip-bound, see below |
| git/git via `?repo=github.com/git/git`, cold | 10.8 s | 13.4 s | clone + `git log` + streaming |
| git/git via `?repo=…`, cached | 0.97 s | 1.07 s | |

"First frame" is from navigation to the engine having commits and a frame
(wasm compile, font load and engine creation included); "complete" is when
the whole log is parsed and seeking unlocks. The wasm `LogParser`
(`scripts/perf/parse-throughput.mjs`, Node) parses the 36 MB log at
225–241 MiB/s (native `gource-rs`: 123 ms ≈ 280 MiB/s). In the page the
batches arrive at a steady ~33 MB/s while the main thread is idle (a single
127 ms long task, at start-up) — the rate of
gource-server's gzip (level 6, single stream, 1.06 s for this file). On a
real network compression pays off (8.7 MB instead of 36 MB); see the open
findings.

## Optimisations

Each one keeps the view hashes of `gource-rs simulate` (3000 frames, every
60th) identical on all 20 fixture and scenario logs plus git/git and stress
(`scripts/perf/compare-hashes.sh`), and `cargo test --workspace` (C++ golden
comparisons, render no-allocation tests) stays green.

| # | Change (commit) | Where | Effect |
|---|---|---|---|
| 1 | **ASCII glyph table** (`4d85127`) | `gource-render-webgl2/src/atlas.rs`: `GlyphAtlas::glyph`, `rasterise`, `reset` | The builder asks the atlas for each character of each label every frame; the `(size, char)` SipHash `HashMap` lookup was 5.1 % (+1.3 % hasher) of a busy browser frame. ASCII entries are now also in a direct-indexed table per font size. Browser build 4.33 → 2.78 ms at 100k files, 0.84 → 0.47 ms on git/git. |
| 2 | **4 MiB `bufferSubData` pieces** (`5219a69`) | `gource-render-webgl2/src/streams.rs`: `VertexStream::upload` | One call per stream (up to 15 MB) cost 7.8 ms per frame with ANGLE Vulkan; 4 MiB pieces 6.4 ms, GL unchanged (3.3 ms). Sweep, Vulkan / GL submit ms: 1 call 7.77 / 3.32; 0.5 MiB 5.72 / 3.96; 1 MiB 5.64 / 3.56; 2 MiB 6.13 / 3.77; 4 MiB 6.43 / 3.28. 4 MiB is the size that does not slow GL down. Also tried and rejected: orphaning (`bufferData(size)` + sub data: Vulkan 8.8, GL 10.8) and `bufferData(data)` (Vulkan 6.65, GL 9.6). |
| 3 | **Hardware square root** (`d8265f3`) | `gource-core/src/vectors.rs` (`length`, `length3`, `normalise2`, `normalise3`), `dirnode.rs` (`distance_to_parent`, `update_spline_point`), `user.rs` (3 sites) | glam's `libm` feature sends `length()` to `libm::sqrtf`, which on wasm32 is libm's *software* routine (the wasm intrinsics need a nightly cfg; x86 already uses SSE). IEEE 754 sqrt is correctly rounded, so `f32::sqrt` (one `f32.sqrt` instruction) is bit-identical: checked exhaustively for all 2³² inputs against libm's soft path, plus a unit test against glam. Browser sim 9.22 → 8.83 ms at 100k files. |
| 4 | **Dedupe before relation tests in the dir force pass** (`eb67e6a`) | `gource-core/src/dirnode.rs`: `DirTree::apply_forces` (quadtree visitor closure) | `DirForceFunctor` ran five relation tests (two ancestor-chain walks) before its seen-set check, for every leaf an item spans: 126k visits per frame at 100k files, 108k of them to multi-leaf items. Checking the seen stamp first gives the same forces in the same order. |
| 5 | **calcColour in the file loop** (`14d39a1`) | `gource-core/src/dirnode.rs`: `DirTree::dir_logic`, `colour_brightness` | The colour sums were a second pass over the node's files after the children's subtrees had evicted them from cache. Summed right after each file's logic (same floats, same order). |

Native A/B of 4 + 5 on stress (3000 frames, `perf stat`): instructions
173.7 G → 157.8 G (−9.2 %), update time −5 %; in the browser sim 10.27 →
9.24 ms together with the glyph change. The per-frame before/after of the
whole set is in the first table: browser −11 % (GL) / −19 % (Vulkan) at 100k files and −21 % / −16 % on git/git; native −3 % frame / −5 % update (median of 3 interleaved runs, stress 50k, 75k and 100k alike).

Measurement additions: `EngineStatus.last_sim_ms / last_build_ms /
last_gpu_ms` and `GourceApp.memory_bytes()` (`gource-web`: `Timings::record`,
`Engine::status`, `EngineStatus::to_json`, `GourceApp::memory_bytes`), and
`gource-rs bench --options` plus peak counts (`gource-cli/src/bench.rs`).

## Findings not fixed

| Finding | Evidence | Status |
|---|---|---|
| Vertex volume dominates large trees in the browser | 19.6 MiB/frame mean at 100k files (files 128 B/quad, glyphs 128 B/quad, same 32-byte vertex as the C++ `quadbuf_vertex`); the GPU process needs 5.5 / 8.6 ms (GL / Vulkan) per frame for it; Vulkan drops frames from 50k files at 7 ms CPU. Halving it needs either a packed vertex (colour as 4×u8: −37 %, but quantises colours before blending, a visible-parity question) or instanced quads (one instance per quad: −75 %, new shader inputs in `gource-render` and the backend). Both are restructurings of the render contract and belong with the render/parity owners. | open |
| File records are memory-bound | `perf annotate` of `dir_logic`: 31 % of its samples on the first touch of the file's arena slot (`arena.rs:135`), 17 % on the `Option` niche check; `RFile` is 272 bytes (native), 100k of them do not fit in L3, and `dir_logic`, `extract_view` and the builder each stream over them. A hot/cold split of `RFile`/`Pawn` (or structure-of-arrays for position, colour and timers) would cut the traffic; it touches most of `gource-core`. | open |
| gource-server gzips logs on the fly at level 6 | 36 MB synthetic log: 1.06 s to compress (gzip −6), 0.30 s at −1 (11.4 MB instead of 8.7 MB); git/git: 0.08 s. This bounds "log complete" on fast links; on a real network the smaller transfer wins. | fixed after this report (review fixes): the pipeline writes a `<key>.log.gz` next to each log while history is read, and the log endpoint streams that stored copy instead of compressing per request |
| Full glyph-atlas re-upload on every new glyph | 59 of 5000 git/git frames re-upload the whole 4 MiB atlas (new characters in names). Cost on those frames: +0.15 ms submit (GL), +2.3 ms (Vulkan); 1 % of frames. A dirty-rectangle `texSubImage2D` would remove it; not worth the complexity now. | wontfix |
| Ancestor walks in the force visitor | After fix 4 the visitor is still ~2.2 ms per frame in the browser at 100k files (60k force applications per frame); pruning `is_parent` by `depth` would save ~0.5 ms but trusts `depth` to be consistent in every re-rooting path. | fixed after this report (review fixes): the tree is numbered depth-first once per frame and ancestor tests use the numbering, bit-identical to the walk (C++ goldens and view hashes unchanged); a 300-level chain drops from 11.1 to 0.84 ms per tick, the 100k stress log runs 7 % fewer instructions. Sibling repulsion stays O(siblings²) per parent, as in Gource |
| Status polling, DOM updates, worker batches | The viewer polls `status()` at 4 Hz and rewrites the status line (negligible); batches are transferred, not copied, and `append_batch` never shows up as a long task. | no issue |

Side note for whoever runs servers here: port 18803, assigned to this
workstream, was taken by another workstream's gource-server
(`gource-wt/flow`); these measurements used 18813–18816.

## Remaining limits

- **Largest tree at 60 fps:** about 75 000 visible files with ANGLE GL and
  25 000–50 000 with ANGLE Vulkan, on an RTX 3080 Ti and a fast desktop CPU,
  in the worst case where every file stays visible and thousands of names are
  labelled. With `--file-idle-time` (files fade out when idle) or
  `hide=filenames`, the visible count, not the repository size, is what
  matters.
- **Largest repository:** history length costs little per frame — git/git
  (21 years, 61k commits) at 0.01 s/day uses 1.7 ms. The server caps logs at
  150 000 commits; such a log loads in ~1.5 s locally.
- **Spikes:** one git/git frame takes ~40 ms in the browser (native update
  38 ms, C++ logic 96 ms on llvmpipe / 88 ms on NVIDIA): a commit that adds
  hundreds of files at once. It is Gource's behaviour (all files are created
  in one tick) and the port is faster than the C++ there too.
- **Timer resolution:** browser numbers are quantised to 0.1 ms.

## How to rerun

```sh
export PATH="$HOME/.cargo/bin:$PATH"
S=/path/to/scratch                         # logs, sites, results

# inputs
git clone --bare --filter=blob:none https://github.com/git/git $S/git.git
git -C $S/git.git -c core.quotepath=off log --reverse --raw --encoding=UTF-8 \
    --no-renames --no-show-signature --pretty=format:user:%aN%n%ct HEAD > $S/git.raw
cargo build --release -p gource-cli -p gource-server
target/release/gource-rs convert --from git $S/git.raw $S/logs/git.log
scripts/perf/gen-stress-log.py > $S/logs/stress.log
scripts/perf/gen-stress-log.py --files 75000 > $S/logs/stress75k.log
scripts/perf/gen-stress-log.py --files 50000 > $S/logs/stress50k.log

# native
scripts/perf/native-suite.sh $S/logs 3 target/release/gource-rs
scripts/perf/native-profile.sh $S/logs/stress.log $S/prof --frames 3000   # flame.svg, top.txt
scripts/perf/ab-native.sh <base gource-rs> <cand gource-rs> $S/logs/stress.log 2 --frames 3000
scripts/perf/compare-hashes.sh <base gource-rs> <cand gource-rs> $S/logs/*.log

# browser (pnpm -C web install first)
scripts/perf/stage-site.sh $S/site $S/logs/*.log            # add --profiling for named wasm
target/release/gource-server --listen 127.0.0.1:18813 --static-dir $S/site \
    --cache-dir $S/cache --rate-limit 0 &
scripts/perf/browser-suite.sh http://127.0.0.1:18813 $S/suite after
scripts/perf/summarize.py $S/suite after
node scripts/perf/browser-bench.mjs --base http://127.0.0.1:18813 --log /perf/stress.log \
    --mode fixed --frames 3000 --gpu gl --finish --profile $S/stress.cpuprofile
node scripts/perf/browser-bench.mjs --base http://127.0.0.1:18813 \
    --repo github.com/git/git --mode fixed --frames 1     # clone service, cold then cached
node scripts/perf/parse-throughput.mjs $S/logs/cap150k.log

# C++ (Docker; GL=nvidia needs the NVIDIA container toolkit / CDI)
scripts/perf/cpp-timing/build.sh
scripts/perf/cpp-timing/run.sh $S/logs/git.log $S/cpp/git 5000 --seconds-per-day 0.01
GL=nvidia scripts/perf/cpp-timing/run.sh $S/logs/stress.log $S/cpp/stress 3000
GL=nvidia FINISH=1 scripts/perf/cpp-timing/run.sh $S/logs/stress.log $S/cpp/stress-sync 3000
```

For a before/after comparison, build the other revision from
`git archive <rev>` into a scratch directory with its own `CARGO_TARGET_DIR`,
stage it with its own `stage-site.sh` and serve it on a second port.
