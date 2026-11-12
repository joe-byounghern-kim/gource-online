<!-- Research notes gathered on 2026-10-02 (branch web-mvp, HEAD 8a3ecec) for the feature specs in docs/features/. Working notes: claims are marked verified or inferred; file:line anchors describe that commit. -->

# Playback, timing and render loop: notes for the in-browser recording spec

Branch `web-mvp`. No files were edited. "V" means verified in code or docs. "I" means inference.

## 1. Driving the engine deterministically at a chosen fps

**Verified: the hooks already exist.**
- `Engine::set_fixed_dt(s)` (`crates/gource-web/src/engine.rs:367`) and `Engine::step(ticks)` (`engine.rs:326-346`) are exposed through wasm at `wasm.rs:111,117`.
  - In fixed-dt mode `frame(now)` only redraws and runs no tick (`engine.rs:307-313`).
  - `step(n)` runs n ticks of `fixed_dt`, or 1/60 if fixed-dt is off (`engine.rs:327-331`).
  - Each tick is a full update, and every tick except the last also does `extract_view` and `build` so HUD animation matches (`engine.rs:333-340`). Only the last tick is drawn.
- The page's rAF loop calls `engine.frame(now)` unconditionally (`web/src/screens/viewer.ts:323-333`). While a recording owns the engine, `frame()` must not tick it. Fixed-dt mode gives exactly that.
- `window.__gource.{app,status,step,setFixedDt}` is always installed (`web/src/main.ts:314-319`), not just under `?fixed-dt`. The `?fixed-dt=` URL param only calls `set_fixed_dt` at start (`main.ts:311`, `url-state.ts:35-36`).
- A small host-side hook can run a loop of `step(k)` then capture. It needs no engine change.

**Verified gotcha: the simulation caps each tick at 1/60 s.**
- `Simulation::update` uses `min(dt, 1/60)` (`crates/gource-core/src/sim.rs:565`, `max_tick_rate` at `sim.rs:381`). The frame builder clamps its HUD dt the same way (`builder.rs:202`, `MAX_TICK`).
- A `fixed_dt` above 1/60 therefore silently slows the simulation. The `gource-rs simulate --dt` help documents the same cap (`crates/gource-cli/src/simulate.rs:30-32`).
- Recording must pick `fixed_dt <= 1/60` and `ticks_per_frame = ceil(60/fps)` when fps divides 60, mirroring C++ (see section 5).
  - 60 fps: `fixed_dt=1/60`, `step(1)`.
  - 30 fps: `fixed_dt=1/60`, `step(2)`.
  - 25 fps: `fixed_dt=1/75`, `step(3)`.
  - 24 fps (not a C++ option, I): `fixed_dt=1/72`, `step(3)`.
- `time_scale` multiplies the capped dt (`sim.rs:568`). `time_scale` is therefore a second speed knob that works in fixed mode without changing the tick count.

**Verified: input and pause during a recording.**
- `set_paused` stops `runtime` (`sim.rs:571`), and a paused tick skips log reading (`sim.rs:955-961`).
- Pointer, key and wheel events still reach the sim through `handle_input` (`engine.rs:426-483`). A recording either needs a lock on viewer input (the viewer binds input on the canvas at `viewer.ts:514-696`) or should run in a separate engine instance (see section 2).

**Verified: timing state and status.**
- Real-time dt is `clamp(now - last, 0, 0.25 s)` (`engine.rs:286-299`). The first frame uses 1/60 (`FIRST_FRAME_DT`).
- `EngineStatus` carries `ticks`, `fixed_dt`, `finished`, `fraction`, `time`, `width` and `height` (`crates/gource-web/src/status.rs:10-45`). `ticks` is a progress counter.

## 2. Arbitrary output resolution

**Verified: the same app can change its drawable size, but not cleanly alongside the live view.**
- `Engine::resize(css_w, css_h, dpr)` sets the drawable to `round(css*dpr)` (`engine.rs:386-405`).
  - It calls `backend.resize`, which sets `canvas.width/height` and `gl.viewport` (`crates/gource-render-webgl2/src/backend.rs:346-356`).
  - It then calls `sim.set_viewport(w, h, dpr)` and redraws.
- `resize(1920, 1080, 1)` would give a 1920x1080 backbuffer. The CSS box stays `width:100%;height:100%` (`web/src/styles.css:762-768`), so the display only scales.
- Side effects on the live view and run:
  - The viewer's `ResizeObserver` and DPR watcher call `resize()` again (`viewer.ts:678-695`) and would fight the override.
  - The sim viewport feeds the default `font_scale = 1 + height/1600` (or 1 when `ui_scale > 1`) (`sim.rs:540-549`).
  - The sim viewport also feeds the camera aspect (`sim.rs:1498-1501`, `1547-1549`, `1569-1572`). Aspect decides the fit distance and the auto-rotate threshold. The camera path, and therefore the recorded video, depends on the output aspect ratio.
  - Pointer coordinates are scaled by `dpr` only (`engine.rs:422-424`), so mapping is wrong while the backbuffer and CSS box differ.
- Verified: the backend renders to the default framebuffer only.
  - There is no offscreen FBO or `readPixels` in the repo. A grep for `readPixels`, `captureStream`, `OffscreenCanvas` and `Framebuffer` over `crates`, `web/src` and `docs` found nothing.
  - Context attributes are `alpha=false`, `antialias=false`, `depth=false`, `stencil=false`, `preserveDrawingBuffer=false`, `HighPerformance` (`backend.rs:147-154`).
  - There is no resolution cap in the repo. The effective limit is `MAX_TEXTURE_SIZE` and `MAX_RENDERBUFFER_SIZE` (I).
- **Recommendation (I):** a recording uses a second `GourceApp` on its own detached canvas, either hidden in the DOM or an `OffscreenCanvas`, because:
  - `GourceApp::new` takes an `HtmlCanvasElement` and needs the canvas in the DOM (`wasm.rs:65-68`). An `OffscreenCanvas` would need a backend change (`web_sys::HtmlCanvasElement` in `backend.rs:130,146`).
  - `GlyphAtlas::new` takes `&Document` (`backend.rs:163`), so it cannot run in a worker without work.
  - A second instance replays the same `EventLog` from the same bytes with the same hash seed. Per `sim.rs:45-53`, `rand` is keyed, so the run is deterministic. The instance needs its own `begin_log` plus batches, since the `EventLog` lives in the engine (`engine.rs:134`).
  - The cost is a second copy of the log (and of the sim state).
- Alternatively, the hidden canvas could carry the viewport as `resize(w, h, 1)`, which makes `font_scale` depend on output height.
- **Capture mechanics (I, with doc support):**
  - With `preserveDrawingBuffer=false`, the buffer must be read in the same task as the draw. That means `new VideoFrame(canvas, {timestamp})` or `createImageBitmap` straight after `step()`.
  - `VideoFrame` accepts an `HTMLCanvasElement` or `OffscreenCanvas` and requires `timestamp` in microseconds (https://developer.mozilla.org/en-US/docs/Web/API/VideoFrame/VideoFrame). The page does not mention WebGL.
  - `VideoEncoder` is secure-context only. It has `encodeQueueSize` for backpressure, and `flush()` (https://developer.mozilla.org/en-US/docs/Web/API/VideoEncoder).
  - `canvas.captureStream(25)` is real-time (https://developer.mozilla.org/en-US/docs/Web/API/HTMLCanvasElement/captureStream). `captureStream(0)` plus `requestFrame()` is manual. Piping it into `MediaRecorder` still uses wall-clock timestamps (I, not tested). That makes faster-than-real-time output unreliable, so WebCodecs with explicit timestamps `frame_index * 1e6/fps` is the fit.
  - An MP4 or WebM muxer is needed for WebCodecs output. Nothing in the repo does that today, and `web/` has no runtime dependencies (CLAUDE.md).

## 3. What determines run length, and whether it can be estimated

**Verified: the formulas.**
- Settings (`settings.rs:99-118`; options `options.rs:122-132,594`): `seconds-per-day` is stored as `days_per_second` (default 0.1, which is 10 s/day, also the web default at `options.ts:102`). Also `time-scale` (range 0.1-4), `auto-skip-seconds` (default 3, negative disables), `start/stop-date`, `start/stop-position`, `stop-at-time`, `stop-at-end`, `loop`, `loop-delay-seconds` (default 3).
- Simulation clock per tick: `time_inc = dt_scaled * 86400 * days_per_second` (`sim.rs:988`).
- The clock jumps to the next commit when `idle_time >= auto_skip_seconds`, where `idle_time` is the time all users have been idle (`sim.rs:1017-1024`, `1480-1485`).
- Commits with `timestamp <= currtime` are popped (`sim.rs:1013-1030`).
- A commit older than the previous one rewinds `currtime` unless `no_time_travel` (`sim.rs:1032-1042`).

**Verified: stop conditions.**
- `read_log` latches `stop_position_reached` for `stop_at_end` with the log finished, for `stop_position`, for `stop_date` (`stop_timestamp`, `sim.rs:1128-1133`), and for `stop_at_time` against `runtime` (`sim.rs:575-577`).
- `is_finished` is `commit_queue.is_empty() && (stop_position_reached || log complete && cursor >= count)` (`sim.rs:719-729`). It turns true when the last commit has been popped, before beams, fades and idle users finish.
- The simulation never quits. `app_finished` is set when `user_map` is empty and a stop is latched (`sim.rs:1476-1478`), but it is folded into the same `finished` flag. A recorder cannot tell the two apart from `EngineStatus`, so it needs a tail (see below) or a new status field.
- `loop` re-seeks to 0 after `loop_delay_seconds` of idle (`sim.rs:969-976`). The run is then unbounded, so recordings need an explicit end (a frame or time limit).

**Estimating length up front.**
- Verified upper-bound formula (log must be complete): with `T = (last_ts - first_ts)/86400`, run time `<= T * seconds_per_day / time_scale` (the time_scale divisor follows from `sim.rs:568`).
- Auto-skip only shortens the run, as long as it is enabled. Per gap it is about `min(gap_days*spd/ts, auto_skip + user settle time)`. This is I: the settle time depends on beam and idle timers, so a closed-form estimate is approximate. Non-monotonic timestamps break the formula.
- The reliable answer (I) is a dry run: a second `Simulation::update` loop without `extract_view`, `build` or `draw`, with dt=1/60. It reports exact tick count to `finished`, plus a configurable tail for the last beams, fade-out and `file_idle_time_at_end`.
- Cost of a dry run (`PERFORMANCE.md:137-141`, measured sim+extract): about 1.0 ms/tick on git/git (6k files) and about 8.8 ms/tick at 100k files in wasm. A 5000-tick git/git run is about 5 s. That is roughly 16x real time on git/git, and about 1.9x at 100k.
- Streaming caveat: the viewer opens on the first commit (`session.ts:218`). A complete log (`finish_log`, `engine.rs:272`) is required for seek, `fraction` and any estimate. `EventLog::is_complete()` gates this (`sim.rs:715-717`).

## 4. Canvas versus DOM elements, and what a recording shows

**Drawn in the WebGL canvas** (`crates/gource-render/src/builder.rs:231-649`):
- Tree edges, files, action beams, users, bloom (`hide_*` flags honoured).
- Dir, file and user names.
- Splash (`hud.splash`, drawn at `builder.rs:494-549`; a splash appears at startup).
- Date, top centre (`builder.rs:555-574`), unless `hide=date`.
- Title, bottom left (`builder.rs:575-585`).
- File-extension key, when `show_key`.
- Hover and selection text box (`builder.rs:611-629`). It appears whenever the pointer has hovered something.
- Transient message, shown for 5 s (`sim.rs:635-639`).
- Not ported: captions, logo, background images and avatars (`sim.rs:74-75`).

**Drawn as DOM, so absent from a recording:**
- The mouse cursor. The render crate has no cursor drawing (grep for `cursor|pointer` in `gource-render/src` found nothing). The `hide=mouse` option hides the DOM cursor via a CSS class (`viewer.ts:277`, `styles.css:1448`).
- The progress/seek slider (`viewer.ts:115-129`) and the speed select. `hide-progress` only sets seekability in the sim (`sim.rs:716`).
- The playback bar, status line, loading box, debug overlay, banner and settings panel.

**Therefore:**
- A canvas capture already excludes the slider and all chrome.
- The hover text box and highlight can appear in a recording if the pointer moves over the canvas. The options are `hide=mouse` for the recording instance (which clears hover, `sim.rs:487-493`, `extract.rs:183-187`) or no input events to the recording engine. A second instance gets no input anyway.
- The splash and message timers also run in the recording. The splash fades over about 15 s of sim time and would overlay the first seconds.

## 5. C++ export semantics to mirror

**Verified in `src/`:**
- `-o/--output-ppm-stream FILE` (`-` for stdout) and `-r/--output-framerate FPS`, values 25, 30 or 60 (default 60). They are parsed in `src/core/settings.cpp:88-89,452-467`, and the help text is at `gource_settings.cpp:104-105`.
- The exporter makes dt fixed: `Gource::update` sets `scaled_dt = max_tick_rate` regardless of real dt (`gource.cpp:248-256`). `time_scale` still multiplies it.
- `setFrameExporter` (`gource.cpp:505-522`): `gource_framerate` is the smallest multiple of the video fps that is `>= 60`, `frameskip` counts the added multiples, and `max_tick_rate = 1/gource_framerate`. The frame is dumped only when `framecount % (frameskip+1) == 0` (`gource.cpp:271-276`). Results:
  - 60 fps: 60 Hz, skip 0.
  - 30 fps: 60 Hz, skip 1, so 2 ticks per output frame.
  - 25 fps: 75 Hz, skip 2, so 3 ticks per output frame.
- Each tick runs the full `logic` plus `draw` (`gource.cpp:265-269`), so HUD animation advances every tick.
- Frames are read with `glReadPixels` as RGB (`src/core/ppm.cpp`, `FrameExporter::dump`) at window size (`display.width*3`). They are flipped and written as P6 PPM on a thread.
- The display is not resizable while recording (`main.cpp:147`). The slider starts hidden when recording (`gource.cpp:166-168`).
- `GourceShell::getNext` forces `stop_at_end = true` when exporting, unless `dont-stop`, `loop` or stdin (`gource_shell.cpp:221-226`). Fullscreen and frame toggles are disabled (`gource_shell.cpp:54,76`).
- Gource quits on `appFinished` after the last commit and once users are gone (`gource.cpp:1402`). Dumping stops on `shutdown`.
- Gource does not hide any element automatically during export. The user passes `--hide` (progress, mouse and so on) and `--title`. The mouse cursor is drawn by `cursor.draw()` unless `hide_mouse` (`gource.cpp:278-281`).

**Mapping to the port:** fixed `max_tick_rate` is `fixed_dt`, `frameskip+1` is `step(ticks)`, and implied `stop_at_end` should be applied unless loop is on. Web options use the same names, set through `set_option`.

## 6. Per-frame cost, for faster-than-real-time encoding

**Verified from `docs/web-mvp/PERFORMANCE.md` (RTX 3080 Ti, Chrome 154, 1280x720, DPR 1, ANGLE GL):**

| | git/git (6k files) | stress (100k files) |
|---|---|---|
| browser CPU per frame | 1.73 ms (sim 1.02, build 0.46, submit 0.25) | 14.92 ms (sim 8.84, build 2.78, submit 3.29) |
| ANGLE Vulkan | 2.00 ms | 18.04 ms (submit 6.44) |
| extra GPU-side wait after submit (not in CPU time) | not given | 5.5 ms GL, 8.6 ms Vulkan |

- Draw calls are always 10 or fewer per frame (`PERFORMANCE.md:178`).
- Upload per frame is 3.2 MiB mean on git/git, 19.6 MiB mean (peak 28 MiB) at 100k files. Wasm memory is about 133 MiB at 100k files.
- Real-time playback holds 60 fps with GL up to about 75k visible files (`PERFORMANCE.md:251-258`).
- Spikes: about 40 ms for one git/git frame with hundreds of new files (`PERFORMANCE.md:262-265`).
- So non-real-time stepping can run 10-100x faster than real time on typical repos, and 2-4x at 100k files. It is bounded by sim plus GPU-process work.
- Not measured in the repo (so unknown): `readPixels`, `VideoFrame` creation and encode throughput at 1080p or 4K. At the output resolution, vertex upload scales with entity count, not pixels. Fill cost scales with pixels (I). `GlyphAtlas` caps at 2048 (`atlas.rs:41-43`).
- Cost per output frame at fps < 60 (I): `ticks * (sim + extract + build) + 1 * gpu`. Each of the two or three intermediate ticks pays full build cost (`engine.rs:333-340`), so 30 fps costs about 2x the CPU per output frame of 60 fps.
- A background tab throttles `requestAnimationFrame`. A fixed-dt `step` loop should not depend on rAF (`setTimeout` or `MessageChannel`, in a visible tab), because throttled timers cap throughput (I).

## Gaps the spec should call out
- No exposure of `app_finished` or tail-until-idle in status (section 3).
- The hash seed and `s` recolour: `Settings::hash_seed` plus the recolour key change colours mid-run. Recording must pin the seed (`sim.rs:463-466`, `609-621`).
- The engine's local-timezone offset table is probed per page (`wasm.rs:34-55`, `engine.rs:200-236`), so dates are browser-local. A cloud-rendered recording would differ unless the zone is fixed.