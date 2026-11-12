# Gource Web MVP: implementation plan

> **History.** A design record from the MVP. Mentions of `gource-server` refer to the original
> Rust clone service, which is not part of this repository; the hosted service implements the
> same API (`packages/protocol/src/api.ts`).

Status: MVP complete. Every phase in section 6 is merged: contracts, the parallel
workstreams, integration, end-to-end verification against real GitHub/GitLab
repositories, parity with the C++ reference renders (`PARITY.md`), performance on a real
GPU (`PERFORMANCE.md`), and a multi-lens code review whose confirmed findings were fixed.
Later additions: F1 (AI co-authors), F3 (recording) and, switched off, F2
(`docs/features/`); then the MVP scope of `docs/features/mvp-scope.md`: private histories
opened from the user's disk (git log output read in the worker), the feature registry and
request counts, pull request activity, and the security headers on every response.

## 1. Goal

A browser (Chrome) version of Gource, written in Rust and compiled to
WebAssembly, with this primary user flow:

1. **Enter a repository.** The landing page has one URL box that accepts
   public GitHub and GitLab repositories (`github.com/owner/repo`,
   `https://gitlab.com/group/sub/repo`, `.git` suffixes, `/tree/<branch>` and
   `/-/tree/<branch>` links). An "Advanced" section takes an optional branch,
   date range (since/until) and subdirectory. The URL is validated while
   typing; anything that is not a public GitHub/GitLab repository is rejected
   in the page.
2. **Analysis.** The page starts a job on the clone service and shows real
   progress from server-sent events: queued → resolving → cloning (git's own
   counting / receiving % / resolving deltas %) → counting commits → reading
   history (N of M commits) → loading (bytes received, commits parsed). A
   repository already analysed at the same HEAD returns from the cache in
   about a second. Errors are specific: invalid URL, not found or private,
   too large (with a hint to use a date range or subdirectory), timeout, git
   failure, empty history.
3. **Visualization.** Gource's simulation and look in WebGL2, starting as
   soon as the first commits are parsed (the seek slider unlocks once the
   whole log has arrived, like Gource's non-seekable stream logs). A side
   panel customises the run live:
   - **Colours & bloom:** background, font/dir/selection/highlight/filename
     colours, bloom intensity and multiplier, colour reseed.
   - **Playback & camera:** pause/play, seek slider with date hover, speed
     (seconds per day), time scale, auto-skip, camera mode (overview/track),
     follow user, padding, auto-rotate on/off, user speed/friction.
   - **Visibility & labels:** hide date/users/tree/files/usernames/filenames/
     dirnames/bloom/root/mouse, file-extension key, title, font scale and
     sizes, dir name depth/position, highlight dirs/users, file extensions
     only, user scale, file idle time, filename time.
   - **Filters (re-simulate):** file filter / file show filter, user filter /
     user show filter (regular expressions), start/stop date. These rebuild
     the simulation from the already-downloaded events; nothing is fetched
     again.

   Settings are mirrored in the page URL (Gource option names), so a link
   reproduces the same view.

Out of scope for the MVP: private repositories and tokens, issues and pull
requests, video export, user avatars, captions, logo/background images,
repositories above the commit cap.

## 2. Constraints

From `CLAUDE.md` and the maintainer:

- **No third-party rendering engines or frontend packages.** Rendering is
  hand-written against WebGL2 through `web-sys` from Rust. The page ships no
  third-party JavaScript. Vite, TypeScript and Playwright are dev-time tools
  only (bundling, type checking, end-to-end tests).
- **WASM does the work.** Parsing, simulation, frame building and GL
  submission run in WebAssembly. TypeScript is limited to the DOM UI,
  network plumbing (fetch, SSE, Web Worker messaging) and font loading.
- **Performance close to native Gource:**
  - about a dozen draw calls per frame regardless of entity count
    (one batched vertex stream per pass, shared quad index buffer);
  - no per-frame heap allocation in steady state (reused `Vec`s in
    `SceneView`, `Frame` and backend upload buffers);
  - one JS→WASM call per animation frame; vertex uploads are zero-copy
    `bufferSubData` views into WASM memory;
  - parsing in a Web Worker; release builds with `lto = "fat"`,
    `opt-level = 3`.
- **Faithful port.** Simulation, colour hashing, layout forces, camera,
  time model, draw order and shaders follow the C++ reference under `src/`.
  Restyling comes later.
- **Deterministic.** Transcendental maths through `libm`, `glam` with
  `scalar-math`, Gource's `rand()` replaced by seeded keyed hashing. The
  same event log, dt sequence and hash seed give the same frames on native
  and wasm32.
- **Licence.** Every crate and file is GPL-3.0-or-later.
- **Git.** Local commits on branch `web-mvp` and its worktree branches only.
  Never push.

## 3. Architecture

```
Browser (Chrome)
┌───────────────────────────────────────────────────────────────────────────┐
│ web/ (TypeScript, no runtime dependencies)                                │
│   landing ─► analysis ─► visualization + control panel                   │
│      │ POST /api/analyze, EventSource /api/jobs/:id/events               │
│      ▼                                                                    │
│   parser.worker.ts ── fetch /api/jobs/:id/log (streamed) ──► LogParser    │
│      │                  (gource-web wasm: CustomLogParser + BatchEncoder) │
│      │ postMessage(batch: ArrayBuffer, transferred)                       │
│      ▼                                                                    │
│   main thread: GourceApp (gource-web wasm)                                │
│      EventLog ◄─ apply_batch                                              │
│      Simulation (gource-core) ─► SceneView ─► FrameBuilder (gource-render)│
│      ─► Frame ─► WebGl2Backend (gource-render-webgl2) ─► <canvas>         │
└───────────────────────────────────────────────────────────────────────────┘
                     ▲ HTTP + SSE
┌───────────────────────────────────────────────────────────────────────────┐
│ gource-server (Rust, axum)                                                │
│   validate URL → git ls-remote (HEAD sha, cache key)                      │
│   → git clone --bare --filter=blob:none (progress parsed from stderr)     │
│   → git rev-list --count → git log --reverse --raw … (Gource's command)   │
│   → GitRawParser → CustomLogWriter → cache file (followed by readers)     │
│   also serves web/dist in production                                      │
└───────────────────────────────────────────────────────────────────────────┘
```

### Crates

| Crate | Role | Platform deps |
|---|---|---|
| `gource-core` | events, settings, input, simulation, camera, scene view | none (glam, libm, regex-lite) |
| `gource-sources` | custom-log and git-raw parsers, sinks, worker batch codec | none |
| `gource-render` | `FrameBuilder`: `SceneView` → `Frame` (vertex streams + draw commands); `GlyphSource` and `Backend` traits | none |
| `gource-render-webgl2` | `WebGl2Backend`: shaders, buffers, textures, canvas-2D glyph atlas | web-sys (wasm32 only) |
| `gource-web` | `wasm-bindgen` API: `GourceApp` (main thread), `LogParser` (worker) | wasm-bindgen, web-sys |
| `gource-cli` | `gource-rs`: convert, simulate, bench (native) | none |
| `gource-server` | clone service + static hosting | axum, tokio, git binary |

### Contracts (stable surfaces, already in the tree)

- `gource-core/src/events.rs` — `EventLog` (interned strings, commits with
  byte offsets, completion flag, fraction↔index mapping; AI attribution
  sets interned by content, `CommitRecord.attrs`, `push_commit_attr`,
  `attr_set`, `attr_sets`, `attr_commit_counts`).
- `gource-core/src/agents.rs` — the AI agent table (`AGENTS`,
  `REGISTRY_VERSION`, `resolve` for ids outside the table).
- `gource-core/src/settings.rs` — `Settings` with Gource defaults;
  `apply_option(name, value) -> Result<OptionEffect, OptionError>` and
  `option_value(name)` using Gource option names.
- `gource-core/src/input.rs` — `InputEvent`, `Key`, `PointerButton`.
- `gource-core/src/sim.rs` — `Simulation` facade: `new`, `set_option`,
  `set_viewport`, `handle_input`, `update(log, dt)`,
  `extract_view(log, &mut SceneView)`, `restart`, `seek`, `set_paused`,
  `status`, `date_at`.
- `gource-core/src/view.rs` — `SceneView` (dirs, files, users, actions,
  labels, HUD, text store) and `CameraView` matrices (GLU formulas).
- `gource-render/src/lib.rs` — `QuadVertex` (32 B), `BloomVertex` (40 B),
  `DrawCommand`, `Frame`, `GlyphSource`, `MonospaceGlyphs`,
  `FrameBuilder::build(view, settings, dt, glyphs, frame)`, `Backend`.
- `gource-sources/src/lib.rs` — `CommitSink`, `ParsedCommit`,
  `CustomLogParser`, `GitRawParser`, `EventLogSink`, `CustomLogWriter`,
  `BatchEncoder`, `apply_batch`; `ParsedCommit.attrs`,
  `ParseStats.bad_meta`, `meta` (the custom log's optional meta field after
  an explicit colour slot, `...|/path||ai=co:claude:Claude Opus 5.5`) and
  `classify` (the AI classifier the server and `gource-rs convert --from
  git` use; the browser never classifies).
- `gource-render-webgl2/src/lib.rs` — `WebGl2Backend::new(&canvas)`
  implementing `Backend`.

Implementations may add to these surfaces. Breaking a signature needs a
note in the workstream report so integration can adapt.

### Frame order (from `Gource::draw`)

1. Clear to `background_colour`.
2. Mouse trace (hover/select) and frustum test (in `extract_view`).
3. Edges (unless `hide_tree`): screen-space spline quads, 2.5 px
   half-width, beam texture; shadow pass offset (2, 2) px with the shadow
   program at 0.5, then the textured pass.
4. File shadows (world offset (2, 2), shadow 0.5, file texture), action
   beams (beam texture), files (file texture).
5. User shadows (world offset (2, 2) × `user_scale`), users (user texture).
6. Bloom (additive, procedural bloom program), per visible in-frustum dir,
   radius `dir_radius * 2 * bloom_multiplier`, colour
   `node colour * bloom_intensity` (the node colour is the average of its
   visible file colours).
7. Text (text program, shadow 0.7): dir names, file names, user names;
   then the selected user/file names on top.
8. HUD: date (top centre), title (bottom left), file key, hover text box,
   status message.

## 4. Browser API (`gource-web`)

```ts
// Main thread. Canvas must be in the DOM; the "GourceSans" font face
// (data/fonts/FreeSans.ttf) must be loaded before construction.
export class GourceApp {
  constructor(canvas: HTMLCanvasElement); // throws if WebGL2 is unavailable
  free(): void;

  // events
  begin_log(): void;                      // new repository: drop events + entities
  append_batch(batch: Uint8Array): void;  // from LogParser.push/finish, in order
                                          // (batch format 2: 32-byte header, string
                                          // and attribution-set tables; v1 refused)
  finish_log(total_bytes: number): void;  // log complete → seekable

  // loop
  frame(now_ms: number): void;            // advance + render; call from requestAnimationFrame
  resize(css_width: number, css_height: number, device_pixel_ratio: number): void;

  // input: CSS pixels relative to the canvas top-left
  pointer_move(x: number, y: number, buttons: number): void;
  pointer_down(x: number, y: number, button: number): void;
  pointer_up(x: number, y: number, button: number): void;
  pointer_leave(): void;
  wheel(delta_y: number): void;
  key(key: string, pressed: boolean, shift: boolean, ctrl: boolean): boolean; // DOM KeyboardEvent.key; true = handled
  // The page forwards Tab only while the canvas has focus and Shift is up
  // (Gource's next-user key); Shift+Tab and Esc move focus out of the canvas.

  // settings: Gource option names and value syntax
  set_option(name: string, value: string): string; // "live" | "restart"; throws on invalid input
  get_option(name: string): string | undefined;
  restart(): void;                        // rebuild from the event log after "restart" options

  // playback
  set_paused(paused: boolean): void;
  seek(fraction: number): void;
  date_at(fraction: number): string;
  status(): string;                       // JSON EngineStatus
}

// Web Worker.
export class LogParser {
  constructor();
  push(chunk: Uint8Array): Uint8Array;    // encoded batch (may be empty)
  finish(): Uint8Array;
  commits(): number;
  bytes(): number;
  skipped_lines(): number;
}

interface EngineStatus {
  date: string; time: number; fraction: number; seekable: boolean;
  paused: boolean; finished: boolean;
  commits_loaded: number; commits_played: number;
  files: number; users: number; dirs: number;
  fps: number; frame_ms: number; sim_ms: number; build_ms: number; gpu_ms: number;
  draw_calls: number; vertices: number;
}
```

Option names used by the control panel (Gource syntax; booleans are
`"true"`/`"false"`; colours are `RRGGBB`): `background-colour`,
`font-colour`, `dir-colour`, `highlight-colour`, `selection-colour`,
`filename-colour`, `bloom-multiplier`, `bloom-intensity`, `hash-seed`,
`seconds-per-day`, `time-scale`, `auto-skip-seconds`, `camera-mode`,
`follow-user`, `padding`, `disable-auto-rotate`, `max-user-speed`,
`user-friction`, `elasticity`, `hide` (comma list of `date,users,tree,files,
usernames,filenames,dirnames,progress,bloom,mouse,root`), `key`, `title`,
`font-scale`, `font-size`, `file-font-size`, `dir-font-size`,
`user-font-size`, `dir-name-depth`, `dir-name-position`, `highlight-dirs`,
`highlight-users` (all), `highlight-user`, `file-extensions`, `user-scale`,
`file-idle-time`, `filename-time`, `max-file-lag`, `file-filter`,
`file-show-filter`, `user-filter`, `user-show-filter` (multi-value options
take newline-separated lists), `start-date`, `stop-date`, `loop`,
`no-time-travel`.

## 5. Clone service API

```
POST /api/analyze
  {"url": "...", "branch"?: "...", "since"?: "YYYY-MM-DD", "until"?: "YYYY-MM-DD", "path"?: "sub/dir"}
  202 {"job": "<id>", "repo": {"host": "github.com", "path": "owner/repo", "display": "owner/repo"}, "cached": bool,
       "ai_detect": bool}     ai_detect: whether the log carries AI attributions
  400 {"error": {"code": "invalid_url" | "unsupported_host" | "invalid_option", "message": "..."}}
  408 / 413 {"error": {"code": "invalid_option", ...}}  body not sent in time / over 16 KiB
  429 {"error": {"code": "busy" | "rate_limited", "message": "..."}}
      busy: the clone queue is full (cache hits and requests joining a
      running job are still served); rate_limited: too many requests per
      minute, or too many analyses in progress, from this client

GET /api/jobs/:id          → current job state as JSON (polling fallback):
                             {"id", "repo", "ai_detect", "state", "events"}
GET /api/jobs/:id/events   → text/event-stream
  event: progress  data: {"stage": "queued|resolving|cloning|counting|history",
                          "label": "Receiving objects", "percent": 45.2,
                          "current": 1234, "total": 5678, "bytes": 1234567}
  event: log       data: {"url": "/api/jobs/:id/log", "commits_total": 12345}
  event: done      data: {"commits": 12345, "bytes": 3456789, "head": "<sha>",
                          "cached": bool, "elapsed_ms": 1234}
  event: error     data: {"code": "not_found|too_large|timeout|git_failed|empty|busy|internal",
                          "message": "..."}
                   (busy: the job was refused because the queue was full,
                   seen by requests that joined it; messages are fixed texts,
                   git's own output is only logged on the server)
GET /api/jobs/:id/log      → text/plain custom log, streamed while history is
                             being read and complete once the job is done.
                             With Accept-Encoding: gzip the stored gzip copy
                             is sent (Content-Encoding: gzip, and
                             Content-Length once complete).
                             409 {"error": ...} when the job failed, 410 when a
                             cached log was evicted, 429 rate_limited over the
                             per-client download limit
GET /api/health            → {"ok": true}
GET /api/features          → {"available": [...], "requestable": [...]}
                             (docs/features/mvp-scope.md section 3)
POST /api/interest         {"feature": "...", "host"?: "github"|"gitlab"|"other"} → 204;
                             a request for a requestable feature, counted once per
                             client per day
POST /api/analyze also takes "pull_requests": true (when the service offers it):
                             pull request events merged into the log
                             (docs/features/mvp-scope.md section 4)
```

The types are in `packages/protocol/src/api.ts`, the URL rules in
`packages/protocol/src/repo-url.ts`.

Implementation notes of the hosted clone service are not part of this repository.

## 6. Work breakdown

### Phase 0: contracts and plan (orchestrator) — done

Branch `web-mvp`, workspace member `gource-server`, contract modules listed
above, this document.

### Phase 1: parallel workstreams (isolated worktrees)

| # | Workstream | Owner | Scope | Acceptance |
|---|---|---|---|---|
| 1 | core | opus | Port `dirnode`, `file`, `user`, `action`, `pawn`, `zoomcamera`, `core/quadtree`, `core/bounds`, `core/stringhash`, Gource logic (`readLog`, `processCommit`, `logic`, `updateCamera`, `mousetrace`, `keyPress`, mouse, `seekTo`, `reset`, filters, `changeColours`, date formatting) and the settings option table into `gource-core`; fill `SceneView` | unit tests per module; scenario tests over `tests/logs` and `tests/fixtures/custom-log` (no panics, expected file/user/dir counts, determinism: same inputs → same state hash); clippy clean |
| 2 | sources | sonnet | `gource-sources/src/{custom,sink,batch}.rs`: custom-log parser (`formats/custom.cpp`, ISO dates, colours, grouping, UTF-8 filtering, leading `/`), `EventLogSink`, `BatchEncoder`/`apply_batch` | tests over every fixture and every chunking (1 byte … whole file) give identical commits; encoder→apply round-trip equals direct parse |
| 3 | render | opus | `FrameBuilder`: port `updateVBOs`, `drawScene`, spline edges, beams, bloom quads, text layout (FXFont semantics: top alignment, drop shadow, rounding), labels, file key (`key.cpp`), text box, date/title/message HUD; ≤ 16 commands per frame | tests on synthetic `SceneView`s: vertex counts, command order, spline maths vs C++ formulas, label positions; zero allocations after warm-up |
| 4 | webgl2 | opus | `WebGl2Backend`: GLSL ES 3.00 ports of `bloom`, `shadow`, `text` shaders plus a textured program, VAOs, growable VBOs with zero-copy uploads, shared quad index buffer, textures from `data/*.png` (decoded at build time), canvas-2D glyph atlas using the `GourceSans` font face with dynamic glyph insertion, context-loss recovery | builds for wasm32 with clippy clean; a standalone test page (`crates/gource-render-webgl2/examples/` + headless Chrome screenshot) draws a synthetic frame |
| 5 | server | opus | `gource-server` per section 5, plus `gource-sources/src/{git,writer}.rs`: git raw parser (`formats/git.cpp`, quoted paths) and `CustomLogWriter` (sanitises `\|` and newlines) | integration tests against a local bare repo (`file://` behind a test-only flag) and one real GitHub clone; SSE stage sequence; cache hit; error codes; `cargo clippy` clean |
| 6 | web-ui | opus | `web/` Vite + TypeScript app: landing/analysis/visualization screens, control panel, URL state, SSE client, parser worker, engine adapter typed per section 4, mock engine for development, Playwright config using `/usr/bin/google-chrome` | `pnpm -C web build` and `pnpm -C web typecheck` pass; mock-mode e2e test walks the three screens |
| 7 | reference | sonnet | Docker build of the C++ Gource from this tree; headless (xvfb + mesa) render of `tests/logs/*.log` and a real repository log at fixed frame rate to PNG frames under `target/reference/` | script reruns from scratch; frames exist and are non-blank |

### Phase 2: integration (opus)

`gource-web` glue (`GourceApp`, `LogParser`, rAF timing, DPR handling,
input mapping, status JSON), `gource-cli` (`convert`, `simulate`, `bench`),
`scripts/build-web.sh` (cargo → wasm-bindgen → `web/src/wasm`), font copy,
`scripts/dev.sh` (server on :8787 + Vite on :5173 with `/api` proxy),
production serving through `gource-server --static-dir web/dist`,
`Dockerfile`.

### Phase 3: end-to-end verification

Playwright with system Chrome against the real server and real
repositories:

- `https://github.com/acaudwell/Gource` → every stage appears, progress
  increases, the canvas shows non-background pixels, the date label
  advances, files/users/dirs in `status()` grow.
- A GitLab repository (`https://gitlab.com/gitlab-org/gitlab-test`, then a
  larger one).
- Invalid URL, unsupported host, missing repository → the right messages.
- Second run of the same repository → cache hit.
- Controls: background colour changes pixels; hide tree/files/names change
  the frame; pause stops the date; seek moves it; filters re-simulate with
  fewer files; URL state restores settings after reload.
- Performance: `gource-rs bench` (native) vs the in-browser bench on the
  same log; frame time and draw calls recorded in `docs/web-mvp/RESULTS.md`.
- Look: side-by-side screenshots against the C++ reference frames.

### Phase 4: review and hardening

Multi-dimension review (correctness against the C++, security of the clone
service, performance, UX) with adversarial verification of each finding,
fixes, then a full rerun of Phase 3.

## 7. Commands

```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p gource-web --target wasm32-unknown-unknown -- -D warnings
scripts/build-web.sh --release          # wasm + bindings into web/src/wasm
pnpm -C web install && pnpm -C web build
cargo run -p gource-server --release -- --listen 127.0.0.1:8787 --static-dir web/dist
pnpm -C web test:e2e                    # Playwright, system Chrome
```

## 8. Risks

- **Fidelity drift** between the port and the C++. Mitigated by porting
  function by function with the C++ open, scenario tests, and reference
  frames.
- **Large repositories:** clone time and log size. Mitigated by the commit
  cap, date/path narrowing, streaming playback and the cache.
- **Text metrics differ** between FTGL and canvas 2D. Accepted: same font
  file and sizes, small sub-pixel differences.
- **HiDPI:** rendering at device resolution with `ui_scale` multiplying
  screen-space constants keeps Gource's proportions.
