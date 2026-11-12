# F3: Recording a video of a customised run

> **History.** A design record from the MVP. Mentions of `gource-server` refer to the original
> Rust clone service, which is not part of this repository; the hosted service implements the
> same API (`packages/protocol/src/api.ts`).

Status: draft specification, 2026-10-02. Branch `web-mvp` (MVP complete). Nothing in this document is implemented.
Companion specs: `ai-coauthors.md` (F1), `private-repos.md` (F2). Terms follow D3: "viewer" is the person using the web app, "contributor" is a person or agent in the repository history. Stories of other specs are cited with the feature (`F1 US-12`); a bare `US-n` is a story of this spec. Research notes: `docs/features/research/playback-record.md`, `docs/features/research/video-research.md`, `docs/features/research/user-render.md`. Shared decisions, the glossary and the shared constants (`SHARE_URL_SOFT_LIMIT`, `NAME_BEARING_OPTIONS`, the wasm size budget) are in `docs/features/README.md`.

Anchors such as `crates/gource-web/src/engine.rs:326-346` point at the current tree. Requirement ids (`REC-n`) are stable: tests, milestones and plans refer to them. User stories are `US-n`, tests are `T-n`.

## 1. Summary

A viewer who has tuned a run (speed, colours, filters, hidden elements, title) presses **Record** and gets a video file of that run, at a chosen resolution and frame rate, independent of the window and of real time.
Nothing is uploaded. The page forks a second engine on a hidden canvas (same event log, same settings snapshot, fixed time step, no input), encodes each frame with WebCodecs `VideoEncoder` (H.264 High), and streams it through a small Rust MP4 muxer (new crate `gource-mux`) into a file the viewer picked with `showSaveFilePicker`.
Timing mirrors Gource's own `--output-framerate` exporter (1 to 3 simulation ticks per output frame at 60, 72 or 75 Hz), so the same log, options and hash seed give the same frames, native and wasm.
The viewer shell stays responsive (the live simulation is paused until M2, REC-20). A settings snapshot (URL state plus recording options) is offered as a "reproduce" link and a recipe `.json` (D9) and, from M2, also embedded in the file.
A spike (M0) settles the open facts first. M1 (split into M1a engine and file path, M1b dialog and polish) is Chrome desktop, H.264/MP4, main-thread export with the core controls. M2 adds the embedded recipe, preview, more sizes, date range and loop, exact length, WebM/VP9, worker export, faststart MP4 and fallbacks. M3 adds server-side rendering, GIF and audio.

## 2. Goals / Non-goals

### Goals

- G1. Faithful output: the recording shows what the viewer would see at the chosen aspect and size, with Gource's exporter timing, no dropped or duplicated frames, no dependence on machine speed.
- G2. The recording uses the viewer's current settings (speed included) as a snapshot that the recording dialog can edit, and the snapshot is the recipe that reproduces the video (D9).
- G3. A file that plays everywhere common: H.264 High, 4:2:0, BT.709 limited range, MP4, correct duration, seekable.
- G4. Works for long runs: constant memory (streaming to disk), a progress bar with a trustworthy ETA, cancel with a defined result.
- G5. No third-party runtime JavaScript. The muxer, timing, planning and estimation are Rust with native tests. TypeScript only touches WebCodecs, File System Access, DOM and the page lifecycle.
- G6. Default behaviour of the viewer, the parity tests and every frame hash stay unchanged.

### Non-goals

- N1. Audio (M3 plans a silent-track option only if a target needs it).
- N2. Recording the DOM (cursor, playback bar, panels). Only the canvas content is recorded.
- N3. Recording user camera gestures (manual drag, zoom, rotate, selection) as a replayable path. A recording starts from Gource's automatic camera. See REC-45.
- N4. Captions, logo, background image, avatars: not ported yet (`crates/gource-core/src/sim.rs:74-75`, `docs/features/research/playback-record.md` section 4), so not recorded. They join the recording automatically when ported, because they are part of the frame builder's output.
- N5. Live streaming of a run (RTMP, WebRTC).
- N6. Re-encoding existing videos, editing, trimming after the fact.
- N7. Non-Chromium browsers in M1. Firefox and Safari are not blocked, but they are untested; see section 10.

## 3. User stories

| id | Story | Testable outcome |
|---|---|---|
| US-1 | As a viewer, I tune a run (speed 2 s/day, hide file names, title) and press Record. The video shows that run. | Decoded frames of the file match frames the same engine produces with the same options through `step` (T-2). |
| US-2 | I pick 1080p, 30 fps, "High" quality and see an upper bound for the duration and for the file size before starting (M1); the "about X" estimate comes in M2. | Dialog fields are filled from the plan; the upper bound is at least the exact length (T-9a); in M2 the estimate is within 0.5x to 2x of it. |
| US-3 | I pick a file name and place once, at click time, and the file is written while the video is encoded, so a 20 minute 4K video does not fill memory. | Peak JS heap and wasm memory growth stay under the budget in section 7 (T-12). |
| US-4 | I keep using the viewer shell (scroll, open panels, edit options) while the recording runs. The live simulation is paused until M2 (REC-20), and nothing I do appears in the video. | The recording's pre-encode frame hashes are identical with and without live interaction (T-5), and the live engine is still paused after a restart-class edit (T-30). |
| US-5 | I press Cancel and can choose to keep what was recorded so far or discard it. | A kept partial file plays and has the right duration; a discarded one leaves no data (T-14). |
| US-6 | A recording that ends on its own does so after the last commit plus a short tail, not after a minute of empty screen. | Frame count equals plan; the last frame is at most `record-tail-seconds` after the last commit (T-4). |
| US-7 | I record a vertical 9:16 video and the camera frames the tree for that aspect, with labels the same size relative to the frame's short side as in 16:9. | The recorded frames use the vertical viewport's camera, and label size relative to the short side matches the 16:9 export within 1 % (T-6). |
| US-8 | My browser lacks an H.264 encoder at 4K; the dialog says so and offers the nearest working setting before I start. | `isConfigSupported` ladder result is shown in the dialog (T-10). |
| US-9 | The history is still downloading; Record is disabled with a reason and enables when the whole log is in. | Button state follows `log_complete` (T-1). |
| US-10 | I open the file's properties, or the "reproduce" link, and get the settings that made the video. | The embedded recipe parses and re-creates identical frames (T-16). |
| US-11 | I record a private repository (F2), or a log of unknown origin, and neither the file, the file name, the title nor the sidecar carries the repository name unless I choose so. | `record-metadata` is `none`, the title is empty, the stem is `gource` and the sidecar is redacted (T-17). |
| US-12 | I leave the tab in the background during a long export and it still finishes. | Frame counter advances in a hidden tab (T-13). |
| US-13 | I save a still of the current frame at the chosen size (M2). | PNG has the output size. |
| US-14 | As a viewer, I apply the F1 rule `ai:claude => cape=agent` (or use the built-in look when no rule is given) and the recording shows it, at 1080p and at 4K. | A golden frame with the rule differs from one without in the styled region and sprites scale with `ui_scale`, including the far-zoom minimum badge (T-29). F1 adds the `decorations` hide element and the `agent-key` option; both appear in the "Show" list (REC-11). |
| US-15 | I save my recording settings as a named preset, or drop a recipe `.json` on the dialog to prefill it (M2). | The prefilled dialog reproduces the recipe's options, sizes and plan parameters (T-11). |
| US-16 | As a viewer who has zoomed or rotated the live view, I am told before recording that my zoom and rotation are not recorded and the video uses Gource's automatic camera. | The dialog shows "Camera: automatic. Your zoom and rotation are not recorded." (T-31). |

## 4. UX

### 4.1 Entry points

| id | Requirement | M |
|---|---|---|
| REC-1 | A **Record** button (icon `record`, label "Record video") sits in `div.viewer-tools` beside the share and debug buttons (`web/src/screens/viewer.ts:167`). It opens the Record dialog. | M1 |
| REC-2 | Keyboard shortcut `Alt+R`, matched on `KeyboardEvent.code === 'KeyR'` so keyboard layouts do not matter. The viewer already ignores keys with `altKey` set (`viewer.ts:641`), so the engine never sees it; the handler calls `preventDefault`. Gource's own `r` (hide root) is unaffected. | M1 |
| REC-3 | The page URL accepts `record=1`: after the log is complete the dialog opens prefilled from the URL's recording options. It never starts a recording by itself, because the file picker needs a user click (REC-15). A reproduce link longer than `SHARE_URL_SOFT_LIMIT` is offered as the recipe JSON instead (REC-73). | M2 |
| REC-4 | Record is disabled, with a tooltip, in these cases: log not complete ("Waiting for the full history, 63 %"), no WebCodecs `VideoEncoder` or insecure context, decided by feature detection only ("This browser cannot encode video here. Chrome and Edge, opened over https or localhost, are tested."), mock engine (`?mock=1`, except in the test mode of REC-93). | M1 |

### 4.2 Record dialog

M1 sketch. Controls added in M2 are listed under the sketch.

```
+--------------------------------------------------------------+
| Record video                                              [x] |
+--------------------------------------------------------------+
|  Size       ( ) Match window 1456x816   (*) 1920x1080 (16:9)  |
|             ( ) 1280x720  ( ) 2560x1440 ( ) 3840x2160         |
|  Frame rate (30) 60         Quality  Standard (High) Max      |
|  Format     MP4 (H.264)                                       |
|  Speed      [ 2.0 ] seconds per day   [Fit to ___ s]          |
|  Time scale [ 1.0 ]     Auto-skip [3.0] s                     |
|  Range      (*) Whole history  ( ) From current position      |
|  Ends       After last commit + [5] s                         |
|  Show       [x] Title  [__owner/repo__]  [x] Date  [ ] Key    |
|             [x] User names [x] File names [x] Dir names       |
|  Camera: automatic. Your zoom and rotation are not recorded.  |
|  Estimate   up to 6:30 .  File up to 280 MB                   |
|             Encoder: H.264 High, hardware preferred, 1920x1080|
|             [Details]   Note: the video can contain flashing. |
|                                                                |
|                         [ Cancel ]            [ Record... ]    |
+--------------------------------------------------------------+
```

Added in M2: the live preview thumbnail with "Refresh preview", the sizes 1080x1080, 1080x1920 and Custom, 24 and 25 fps, the Dates range, "Loop, max N s", the `about X` estimate and "Calculate exact length first", and remembered values.

| id | Requirement | M |
|---|---|---|
| REC-5 | Every field maps to an option in section 5.2. Changing a field never changes the live view; the dialog edits a copy of the snapshot (REC-35). | M1 |
| REC-6 | The size presets are 1280x720, 1920x1080, 2560x1440, 3840x2160, 1080x1080 (square), 1080x1920 (vertical 9:16) and "Custom" (`WIDTHxHEIGHT`). "Match window" is a seventh choice. In M1 the presets are 1280x720, 1920x1080, 2560x1440, 3840x2160 and "Match window"; the square, vertical and Custom sizes are M2. It resolves, when the dialog opens, to the live canvas's device-pixel size rounded down to even sides, then scaled down (same aspect, sides rounded down to even) until both sides are at most 4096 and the macroblock limit of REC-44 holds; `ui_scale` is multiplied by the same factor. The dialog shows the resolved size ("Match window 1456x816", or "4096x2304 (window 5120x2880, scaled down)") and the recipe stores the resolved `record-viewport`. Defaults: 1920x1080. | M1 |
| REC-7 | The preview thumbnail always has the output aspect ratio and is rendered by the recording engine (section 5.4), not cropped from the live canvas, because the camera framing depends on the aspect (`crates/gource-core/src/sim.rs:1498-1501`, `1547-1549`, `1569-1572`). It shows the run 5 simulated seconds after the chosen start (so a few users and files exist), is regenerated 400 ms after the last change, and has a "Refresh preview" button. The preview is sliced (`preview_for(sim_seconds, ms)`, at most 8 ms of work per macrotask, REC-114), is cancelled by the next change, and shows the previous image until the new one is done. A preview that finishes without having processed any commit marks the run empty (REC-122). | M2 |
| REC-8 | "Speed" shows `seconds-per-day`, `time-scale`, `auto-skip-seconds` from the viewer's current values. "Fit to N s" sets `seconds-per-day = N / span_days` (span from the plan, section 5.9); a note says auto-skip can make the video shorter. | M1 |
| REC-9 | "Range": whole history; from the viewer's current position (sets `start-position` to the viewer's `fraction`); or dates (`start-date`, `stop-date`). "From current position" is disabled while the viewer's `fraction` is 0 (log not initialised) or at least 1 (finished), because `start-position` accepts only values strictly between 0 and 1 (`crates/gource-core/src/options.rs:612-623`). Starting mid-history shows only the files changed from there on, as in Gource's own `--start-position`; the dialog says so in one sentence. The Dates range is M2. | M1 |
| REC-10 | "Ends": the default is Gource's exporter behaviour (stop at the end, `stop-at-end`) plus `record-tail-seconds` (default 5, in video seconds; -1 reads "Until every user has left (about 12 s after the last commit)"). User idle and fade advance on `dt * time_scale` (`sim.rs:568`), so below `time-scale` 1 the dialog notes that a fixed tail may cut fading users off and suggests a longer tail or -1. "Loop" requires a maximum length (`record-max-seconds`, default 60 when loop is chosen). A maximum length may also be set without loop. Loop is M2. | M1 |
| REC-11 | "Show" lists the `hide` elements the engine reports (date, users, tree, files, usernames, filenames, dirnames, bloom, root; F1 adds `decorations`), the file-extension key (`key`), the agent key (`agent-key`, F1 AI-59), and a title field. The title field defaults to the repository display name for public repository sources, and is empty for private sources (F2) and sources of unknown provenance (a local file, `?log=`), because the title is burned into every frame (REC-103). It is empty when unchecked. The list of hide toggles is built from the engine's `hide` options at run time, so elements that F1 adds appear without a change here. When `ai-filter` is not `all` the dialog adds the line "The video will show the AI filter line" (it is drawn even when the date is hidden, F1 AI-74). Mouse and progress are not offered: they are not in the canvas (REC-40). | M1 |
| REC-12 | M1: the estimate line shows `up to Y`, the upper bound of section 5.9 (instant). M2: it shows `about X, up to Y`, X being the heuristic of section 5.9. The file size is shown as an upper bound (`bitrate x Y`), never as a promise, because flat Gource frames usually encode below the target. "Calculate exact length first" runs the dry run (REC-67), shows progress, and replaces the estimate with the exact frame count, duration and size bound. It is off by default below 5 minutes of upper bound and offered prominently above 10 minutes. | M1 (upper bound) / M2 (about X, dry run) |
| REC-13 | The encoder line shows the first entry of the support ladder (REC-52) that `VideoEncoder.isConfigSupported` accepts for the current size, frame rate and quality, with a plain-language note when it differs from the request, for example "H.264 hardware encoding is not available at 3840x2160 here; using software at 3840x2160" or "using 1920x1080 instead". The viewer must accept the downgrade by pressing Record; nothing is downgraded silently after that. WebCodecs reports only whether a configuration is supported, not whether hardware is used, so the UI says what was requested ("hardware preferred", "software preferred", "automatic") and never claims what was actually used; the recipe stores the requested preference and the accepted candidate. T-7 and T-10 measure whether `prefer-*` is strict on real encoders; the ladder order does not rely on it. | M1 |
| REC-14 | Warnings, shown inline and not blocking: upper bound over 60 minutes ("This could take about N minutes to render"); estimated size over the in-memory cap when the sink is a Blob (REC-63); `seconds-per-day` below 0.05 (quick) or above 30 (unreasonably slow); fps 60 at 4K ("Software encoders may be slow"); upper bound over 10 minutes ("A crash or closing the tab loses the whole file", REC-64); the Blob cap stated as minutes for the chosen size. | M1 |
| REC-124 | Record asks for a blocking confirmation step, which lists the values, when the size upper bound exceeds 2 GB, the duration upper bound exceeds 60 minutes, or any recording option was supplied by the URL (REC-3) and not typed in the dialog. The dialog states the size upper bound in plain text ("up to 280 MB"). A hostile URL (4096x2304, 60 fps, `record-bitrate=100`, loop) therefore cannot be started by a single click. Test T-28. Because M1 has no `record=1` (REC-3), M1 relies on the inline warnings of REC-14. | M2 |

### 4.3 File picking and start

| id | Requirement | M |
|---|---|---|
| REC-15 | Pressing "Record..." calls `showSaveFilePicker` first, in the click handler, with `suggestedName` (REC-62) and `types: [{description: 'MP4 video', accept: {'video/mp4': ['.mp4']}}]`, then does the asynchronous work (ladder check is already done by the dialog; encoder creation follows). The picker is the user activation the browser requires, and it must not wait for any `await`. | M1 |
| REC-16 | If `showSaveFilePicker` is missing, the dialog states that the video will be built in memory and downloaded at the end, with the size cap (REC-63), and Record then starts without a picker. If the viewer dismisses the picker (`AbortError`), the dialog stays open. | M1 |

### 4.4 Progress

```
+--------------------------------------------------------------+
| Recording  owner/repo  1920x1080 30 fps                       |
|  +----------+   [#########-----------------]  38 %            |
|  | thumbnail|   2026-03-14   frame 4,213 of about 11,100      |
|  +----------+   Elapsed 1:42   about 2:50 left   2.9x real time|
|                 File so far 38 MB   Encoder: automatic         |
|  Keep this tab open. You can keep using the viewer.            |
|                       [ Cancel ]                               |
+--------------------------------------------------------------+
```

| id | Requirement | M |
|---|---|---|
| REC-17 | Progress is `expected(cursor) / about_s`: `expected` is the cumulative per-gap table of the heuristic of section 5.9 (auto-skip compression and uneven commit density included), restricted to the resolved range (`start-position`, `start-date`, `stop-date`, `stop-position`), and `cursor` is the recorder's commit index (REC-80). The log byte fraction of the seek slider (`crates/gource-core/src/sim.rs:646-660`) is not used: it correlates poorly with video time, and a ranged run would start above 0 % and never reach 100 % before the tail. In the tail progress moves linearly to 100 % over the tail frames. The bar never goes backwards (clamped monotone). After an exact dry run (M2) the denominator is the exact frame count. In M1 the table drives only the progress bar and carries no accuracy claim. | M1 |
| REC-18 | The ETA is `elapsed / progress - elapsed`, smoothed (EMA with alpha 0.1 per 500 ms) and shown only after 3 s and 2 % progress. After an exact dry run the ETA uses frames done over the exact frame count. | M1 |
| REC-19 | The thumbnail updates at most every 500 ms, copied from the recorder canvas right after a draw (section 5.1, step 2). The counters (frame, date, elapsed, file bytes, speed as simulated video seconds per wall second) update at 4 Hz, the cadence of `STATUS_INTERVAL_MS` (`viewer.ts:15`). | M1 |
| REC-20 | While recording, the live viewer shows a banner "Recording in progress: the live view is paused; changes here do not affect the video". Its simulation is paused (`set_paused(true)`, restored afterwards). The seek slider and options with a Restart effect (`RESTART_OPTIONS`, `web/src/options.ts:157`) are disabled with that reason, because `Simulation::restart` builds a fresh simulation and thereby ends a pause (`crates/gource-core/src/sim.rs:596-612`); the viewer also re-asserts `set_paused(true)` after every `set_option` and seek while a recording is `running`. Other panel options stay editable and never change the video. Session loss on a private source (sign-out or expiry, F2 PR-121) ends the recording through "Stop and save", then frees the fork and the muxer (REC-118). The `beforeunload` handler asks for confirmation. Test T-30. | M1 |
| REC-21 | **Cancel** opens a choice: "Keep recording", "Stop and save what is recorded" (finalises a valid file of the frames so far), "Discard". The same "Stop and save" is offered when an error occurs after the first frame (section 5.11). | M1 |
| REC-22 | **Discard** aborts the sink: with File System Access it calls `writable.abort()`, so the destination file stays as the picker created it (empty, or the previous content of the file the viewer overwrote). The finished screen says so ("An empty file named X was left where you saved it; delete it if you like"), because a handle cannot delete the file. With the Blob sink nothing is left. | M1 |

Cancel choice:

```
+-----------------------------------------------+
| Stop recording?                               |
|  4:12 of about 6:10 is recorded (38 MB).      |
|  [ Keep recording ] [ Stop and save ] [ Discard ] |
+-----------------------------------------------+
```

### 4.5 Finished screen

```
+--------------------------------------------------------------+
| Saved  owner-repo_1920x1080_30fps_2026-10-02.mp4              |
|  6:10, 11,100 frames, 61 MB, H.264 High                       |
|  Stored on this computer. Nothing was uploaded.               |
|  [Copy reproduce link] [Download recipe (.json)] [Record again] |
+--------------------------------------------------------------+
```

| id | Requirement | M |
|---|---|---|
| REC-23 | On success: the file name, duration, frame count, size, resolution, the encoder candidate accepted (REC-13), a **Open** link (only for the Blob sink: a download link; for File System Access the picker already saved it), **Copy reproduce link**, **Download recipe (.json)** and **Record again**. The text states clearly that the file is stored on this computer and was not uploaded. | M1 |
| REC-24 | The browser's own notification is not used; the tab title is prefixed with "[Done]" when the page is hidden at completion, and reset on focus. | M2 |

## 5. Design

### 5.1 Architecture

```
 viewer (live)                       Record dialog / controller (TypeScript)
 GourceApp --fork-->  GourceRecorder (wasm)         VideoEncoder (WebCodecs)
  (EventLog, Settings   fork: EventLog clone,         ^  encode(VideoFrame)
   snapshot)            Settings snapshot, fresh      |
                        Simulation, own canvas,       |  EncodedVideoChunk + decoderConfig
                        own WebGl2Backend,            v
                        ui_scale, fixed_dt   --canvas--> new VideoFrame(canvas, {timestamp})
                                                      |
                        Mp4Writer (gource-mux via wasm)  <-- chunk bytes, pts, key flag
                              | bytes (init, fragments, tail, patches)
                              v
                        Sink: FileSystemWritableFileStream | OPFS | Blob
```

The export loop, per output frame (all within one macrotask until the final await):

1. `recorder.step_frame()` runs `ticks_per_frame` ticks of `fixed_dt`, draws once.
2. `new VideoFrame(recorder canvas, {timestamp, duration})` in the same task as the draw (the drawing buffer is not preserved, `crates/gource-render-webgl2/src/backend.rs:147-154`).
3. `encoder.encode(frame, {keyFrame})`, then `frame.close()`.
4. Backpressure: await the encoder `dequeue` event while `encodeQueueSize > 6`, await the sink while more than 2 writes are pending, yield a macrotask (MessageChannel, not `setTimeout`, not `requestAnimationFrame`) once per frame.

### 5.2 Options and the settings snapshot

Recording options carry the `record-` prefix (the two that echo Gource's exporter are renamed, because their semantics differ: C++ `-r` accepts 25, 30 and 60, and C++ `--viewport` sets the window size). They are not engine `Settings`: this is a recorded deviation from the shared rule that options go through `set_option` (README, cross-cutting decisions); the reason is that a recording option must never reach the live simulation or the parity tests. They never pass through `Simulation::set_option`, so the plain viewer, the parity tests and the existing option round-trip test (`crates/gource-core/src/options.rs:904`) are untouched. They live in a new Rust module (`crates/gource-web/src/record/options.rs`) with the same strict string-in/string-out contract (`RecordOptions::apply(name, value)`, `RecordOptions::value(name)`, `RecordOptionError::InvalidValue {option, value, reason}`), and a TypeScript table `RECORD_OPTION_DEFS` (`web/src/record/options.ts`) mirroring `OPTION_DEFS` (`web/src/options.ts:89`). A Rust test and a TypeScript unit test check both tables agree (names, defaults, ranges).

| Option | Gource? | Values | Default | Meaning |
|---|---|---|---|---|
| `record-framerate` | no (echoes Gource's `-r`; 25, 30, 60 in C++, `src/core/settings.cpp:88-89,452-467`) | 30, 60 in M1; 24, 25, 30, 60 in M2 | 30 | Output frame rate. 24 is new. C++ default is 60; see open question Q1. |
| `record-viewport` | no | `WIDTHxHEIGHT`, or empty = match window | empty | Output size in pixels. |
| `record-ui-scale` | no | `auto` or a number in 0.25..4 | `auto` | Screen-space scale (REC-38). `auto` is `min(width, height) / 1080` for explicit sizes, and for "match window" the device pixel ratio scaled with the size (REC-6). |
| `record-format` | no | `mp4`, `webm` (M2) | `mp4` | Container and codec: `mp4` is H.264, `webm` is VP9. |
| `record-layout` | no | `auto`, `fragmented`, `faststart` (M2) | `auto` | MP4 layout (REC-59). |
| `record-quality` | no | `standard`, `high`, `max` | `high` | Bitrate preset (REC-50). |
| `record-bitrate` | no | Mbps, 0 = from quality | 0 | Overrides the preset. Range 0.5 to 100; the level rises until it fits (REC-50). |
| `record-keyframe-seconds` | no | 0.5 to 10 | 2 | Keyframe interval. |
| `record-hardware` | no | `auto`, `prefer-hardware`, `prefer-software` | `auto` | Maps to `hardwareAcceleration` (`no-preference`, ...). |
| `record-tail-seconds` | no | -1, or 0 to 120 | 5 | Video seconds recorded after the last commit; -1 means until Gource would quit (every user deleted, about 12 s). |
| `record-max-seconds` | no | 0 (none), or 1 to 14400 | 0; 60 when `loop` | Hard cap on video length. Required when `loop` is true. |
| `record-metadata` | no | `embed`, `none` | `embed` for public repository sources, `none` for private (F2) and unknown-provenance sources (local file, `?log=`) | Reproducible inputs in the file (REC-71). |
| `record-name` | no | file name stem | derived (REC-62) | Suggested file name. |
| `record` | no (URL only) | `1` | absent | Opens the dialog after load (REC-3, M2). |

M1 options: `record-framerate`, `record-viewport`, `record-ui-scale`, `record-quality`, `record-tail-seconds`, `record-max-seconds`, `record-name`. M2 options: `record-format`, `record-layout`, `record-bitrate`, `record-keyframe-seconds`, `record-hardware`, `record-metadata`, `record`, and the extra frame rates. The `record-*` and `record` URL parameters are added to the url-state non-default and unknown-parameter tests and are excluded from the Gource option round-trip test (T-11), as F2 PR-91 excludes its transient parameters.

The recording **snapshot** is built when the dialog opens:

- `engine_options`: the full `Settings` of the live simulation, copied with the same adjustments `Simulation::restart` applies (`crates/gource-core/src/sim.rs:602-612`, including the base `file_idle_time`), so keyboard-changed state that lives in `Settings` (colour reseed `s`, `+`/`-` speed, `.` `,` `/` time scale, hide toggles, camera mode, `hash-seed`) is included. Not covered, and therefore not recorded: camera state, selection, hover, pause and pointer state, and the tree's own runtime config, which the `z` key (gravity toggle, read nowhere in Gource) and the `[` / `]` keys (`force_gravity`) change (`crates/gource-core/src/sim/input.rs:283-306`, `crates/gource-core/src/tree.rs:88-107`; neither is in `OPTION_NAMES`). The recorder uses the default tree config (REC-125).
- The dialog's edits as option overrides on top (`seconds-per-day`, `time-scale`, `auto-skip-seconds`, `hide`, `title`, `key`, `start-position`, `start-date`, `stop-date`, `loop`).
- Forced overrides (REC-35, REC-38): for explicit sizes `default_font_scale` false and `font_scale` 1 unless the viewer set `font-scale`; `stop-at-end` true unless `loop`; `hide` gains `mouse` and `progress` (no visible effect, set so the snapshot is explicit); `disable-input` semantics are provided by the recorder having no input path.
- `recording`: the table above.

The **recipe** (D9) is the URL-state serialisation of the snapshot (non-default Gource options by name, repeated parameters for lists, as `web/src/url-state.ts:41-85`) plus every non-default recording option, plus the resolved plan parameters (REC-70). The recipe has two renderings: the embedded copy (REC-71, M2), which holds only reproducible inputs, and the sidecar (REC-72, M1) with the result of the run. "Reproduce link" is the page URL with these parameters and `record=1`.

| id | Requirement | M |
|---|---|---|
| REC-30 | Recording options are parsed and validated in Rust (`RecordOptions`), with a round-trip test (every `value(name)` output is accepted by `apply(name, ...)` unchanged), like `options.rs:904`. Invalid values return `RecordOptionError` with a human-readable reason that the dialog shows next to the field. | M1 |
| REC-31 | The snapshot is exact: a native test builds a `Simulation` from `Settings` copied through the snapshot path and another from the same options applied one by one by name (what the URL does), runs 600 ticks on a fixture log, and requires equal `SceneView` hashes (`hash_view`, moved to `gource-core` and re-exported by `crates/gource-cli/src/hash.rs`, REC-82). In M2 a second test enumerates every field the keyboard can mutate and asserts it is either in `Settings` (so the snapshot carries it) or on the explicit not-recorded list (REC-125, T-26). | M1 |
| REC-125 | The tree runtime config changed by `z` and `[` / `]` is not part of the snapshot. `SimStatus` gains `tree_tweaked: bool` (`TreeConfig != default`, `TreeConfig` derives `PartialEq`), and when it is true the dialog says "Gravity changes made with [ ] are not recorded"; the Details popover lists "not recorded: gravity keys". Q13 asks whether to promote them to options. | M2 |

### 5.3 Timing model (mirrors `Gource::setFrameExporter`)

C++ (`src/gource.cpp:505-522`): the engine tick rate becomes the smallest multiple of the video frame rate that is at least 60 Hz, and one frame is dumped every `frameskip + 1` ticks. Every tick runs `logic` and `draw`, so HUD animation advances each tick (`gource.cpp:265-276`). `time_scale` still multiplies the tick (`gource.cpp:248-256`).

The Rust engine has the same two ingredients: `Engine::step(ticks)` runs `ticks` ticks of `fixed_dt` and draws once, with a full extract and build on every tick but the last so HUD animation matches (`crates/gource-web/src/engine.rs:326-346`); and `Simulation::update` clamps each tick to 1/60 s (`crates/gource-core/src/sim.rs:381`, `565`). A `fixed_dt` above 1/60 would silently slow the simulation, so the plan uses `fixed_dt <= 1/60`.

For frame rate `f`: `k = ceil(60 / f)`, `tick_hz = k * f`, `fixed_dt = 1 / tick_hz`, `ticks_per_frame = k`.

**Phase.** C++ dumps a frame when `framecount % (frameskip + 1) == 0` after the update and draw of that tick, and `framecount` starts at 0 (`src/gource.cpp:271-284`). The first frame is therefore the state after **1** tick, the next after `1 + k`, and frame `n` after `n * k + 1` ticks. The parity harness relies on the same fact ("saved frame k is the state after 2k + 1 ticks" at 30 fps, `docs/web-mvp/PARITY.md`, Method 1). The plan reproduces it: frame 0 runs `step(1)`, every later frame runs `step(k)`. This refines the simple "k ticks per frame" rule and makes recorded frames line up with the C++ reference renders.

| fps | k | tick rate | `fixed_dt` | ticks before frame n | pts of frame n |
|---|---|---|---|---|---|
| 60 | 1 | 60 Hz | 1/60 | `n + 1` | `round(n * 1e6 / 60)` us |
| 30 | 2 | 60 Hz | 1/60 | `2n + 1` | `round(n * 1e6 / 30)` |
| 25 | 3 | 75 Hz | 1/75 | `3n + 1` | `round(n * 1e6 / 25)` |
| 24 | 3 | 72 Hz | 1/72 | `3n + 1` | `round(n * 1e6 / 24)` |

For 25 fps this reproduces C++ exactly (75 Hz, skip 2). 24 fps is new (the C++ exporter accepts only 25, 30 and 60). Because `n * 1e6 / f` is rounded per frame, error never accumulates.

| REC-32 | The frame plan (`FramePlan {fps, ticks_per_frame, fixed_dt, ticks_for_frame(n), pts_us(n), duration_us(n), keyframe_every}`) is a pure Rust type in `crates/gource-web/src/record/plan.rs` with native tests for the four rates: cumulative tick counts (`n * k + 1`) over 10 minutes, `pts_us` monotonic, `pts_us(n+1) - pts_us(n)` equals the nominal period within 1 us, sum of durations equals `pts_us(N)`. | M1 |
| REC-33 | `duration` of each `VideoFrame` is `pts_us(n+1) - pts_us(n)`; the last frame uses the nominal period. | M1 |
| REC-34 | Speed knobs are exactly Gource's: `seconds-per-day` (stored as `days_per_second`, `crates/gource-core/src/settings.rs:99-118`, applied at `sim.rs:988`), `time-scale` (multiplies the clamped tick, `sim.rs:568`), `auto-skip-seconds` (`sim.rs:1017-1024`). Changing `time-scale` in the dialog does not change the tick count per frame. | M1 |
| REC-35 | The recorder sets `stop_at_end = true` unless `loop` is on (`gource_shell.cpp:221-226` does the same for the exporter), and has no input path: pointer, key and wheel methods do not exist on `GourceRecorder`. | M1 |
| REC-36 | The recorder applies the same `Clock`-free determinism as the viewer: no wall time reaches the simulation (`Engine::step` uses only `fixed_dt`). A test runs the same recorder twice and requires equal frame hashes. | M1 |

Stop conditions, in priority order (evaluated after each output frame):

1. `record-max-seconds` frames reached (`frames == ceil(max_seconds * fps)`): stop, result flag `limit_reached`.
2. Loop off, `record-tail-seconds >= 0`, and the simulation's `finished` flag has been true for `record-tail-seconds` of video time: stop. An explicit tail is honoured in full, even when every user has faded earlier, so the dialog copy "After last commit + N s" is exact.
3. Loop off, `record-tail-seconds = -1`, and `app_finished` (every user deleted and a stop latched, `sim.rs:1476-1478`): stop at once. Only this rule applies for -1.
4. A hard cap of 4 hours of video (`HARD_MAX_SECONDS = 14400`), result flag `hard_cap`.

Gap closed here: `EngineStatus.finished` today folds `app_finished` into the same flag as "last commit popped" (`sim.rs:719-729`), so a driver cannot tell them apart. REC-80 adds `app_finished` and `log_complete` to `SimStatus` and `EngineStatus` (`crates/gource-web/src/status.rs:10-45`). `finished` becomes true when the last commit is popped, before beams, idle users and fades finish; at `time-scale` 1 the default 5 s tail covers user idle (3 s, `user_idle_time`) plus the 1 s fade (`crates/gource-core/src/user.rs:231-239`). Idle and fade run on `dt * time_scale` (`sim.rs:568`), so below 1 a fixed tail may cut fading users off (REC-10 note). Gource's own exporter would continue until users are deleted after 10 idle seconds (`user.rs:61`), which is why `-1` exists. For the estimates of section 5.9, -1 counts as `TAIL_ALLOWANCE_S = 12` (10 s inactivity, 1 s fade, 1 s margin) divided by `min(1, time_scale)`.

### 5.4 The second engine

A recording needs its own simulation, its own canvas and its own camera; reusing the live engine would fight the live `ResizeObserver`/DPR handling (`web/src/screens/viewer.ts:678-695`), the live pointer and key input (`viewer.ts:514-696`), and would change `font_scale` (`sim.rs:540-549`) and the camera aspect of the live view.

`GourceRecorder::create(app: &GourceApp, canvas, options_json)` builds an `Engine<WebGl2Backend, PerfClock>` from:

- `EventLog` cloned with `Clone` (`crates/gource-core/src/events.rs:88-97`, derives `Clone`): same interned strings, commits, total bytes and completeness. No serialisation, no second parse. Creation is written against a small `LogSource` trait (`fn clone_log(&self) -> EventLog`) so that M2's worker (section 5.13) adds a second implementation (batches from `export_log`) instead of rewriting `create`. The cost is one more copy of the log in the same wasm linear memory (it only grows, `PERFORMANCE.md`: 133 MiB at 100k files), released for reuse after the recorder is freed but not returned to the OS.
- `Settings` from the snapshot (REC-5), including `utc_offset_minutes` and the offset table, and the engine's time-zone probe (`engine.rs:200-236`, `wasm.rs:40-55`) so dates print identically.
- A new `WebGl2Backend` on the recorder canvas. `WebGl2Backend::new` needs only an `HtmlCanvasElement` and its document (`backend.rs:146-170`).

| id | Requirement | M |
|---|---|---|
| REC-37 | Output size: the recorder calls a new `Engine::set_output_size(width, height, ui_scale)`: it sets the drawable to exactly `width x height` (no CSS rounding as in `resize`, `engine.rs:386-405`), calls `sim.set_viewport(width, height, ui_scale)` and redraws. `ui_scale` multiplies every screen-space constant and font size (`crates/gource-render/src/builder.rs:197-215`, `text.rs:6-9`), and the simulation sees it only through `font_scale` (`sim.rs:540-549`: `font_scale = 1` when `ui_scale > 1`, else `1 + height/1600` integer division); the recorder pins `font_scale` for explicit sizes instead of relying on that rule (REC-38). | M1 |
| REC-38 | `auto` UI scale: `ui_scale = min(width, height) / 1080` for explicit sizes (the shorter side is what labels and the 2.5 px edges must stay proportional to; an area basis was considered and rejected), so the picture is the same image at every resolution and orientation: 1920x1080 and 1080x1920 both get 1.0, 4K gets 2.0 (labels and line widths keep their proportion; a 4K export is the 1080p export, sharper). For "match window", `ui_scale` is the live device pixel ratio times the REC-6 scale-down factor and the logical size equals the live canvas, so the recording looks like the live view, and the C++ `font_scale` rule stays on so the same logical size gives the live `font_scale`. For explicit sizes the snapshot pins `font_scale` to 1 (`default_font_scale` false), because the rule `1 + height/1600` (`sim.rs:540-549`) would give 2 for the 1920 px height of a 1080x1920 frame at `ui_scale` 1.0 and double its labels. A user-set `font-scale` still applies. C++ at 4K gets `font_scale = 2` but fixed 2.5 px edges, which would change proportions; the recorder deliberately differs and documents it in PARITY.md. | M1 |
| REC-39 | The camera framing depends on the aspect ratio only (`docs/features/research/playback-record.md` section 2 cites `sim.rs:1498-1501` etc.), not on the pixel count. A test records the same 100 ticks at 1280x720 and 2560x1440 and requires equal camera matrices; and at 1920x1080 vs 1080x1920 requires different ones (US-7). | M1 |
| REC-40 | The recorder never receives input, so hover, selection, text box and cursor effects cannot occur: the HUD text box (`builder.rs:611-629`) is empty in every recorded frame (unit test on `SceneView.hud`), and the splash (shown only by the `c` key, `crates/gource-core/src/sim/input.rs:281`; C++ the same, `gource.cpp:792`) never appears. This corrects the research note (`docs/features/research/playback-record.md` section 4) that said the splash appears at startup: `splash` starts at -1 (`sim.rs:402`) and nothing but the key sets it. | M1 |
| REC-41 | `GourceRecorder::create` fails with code `record_log_incomplete` unless `EventLog::is_complete()` (`crates/gource-core/src/events.rs:208`). The viewer's seek slider has a similar condition (`Simulation::can_seek`, `sim.rs:715-717`, which also requires `!hide_progress`). | M1 |
| REC-42 | The glyph atlas budget would defer new glyphs to later frames (`MAX_NEW_PER_FRAME = 512`, `crates/gource-render-webgl2/src/atlas.rs:48,390,405`): acceptable live, wrong in a video (names would pop in a frame late and the result would depend on the frame sequence of the preview). The recorder's backend is created with `GlyphAtlas::set_frame_budget(u32::MAX)` so every glyph is rasterised in the frame that needs it. The atlas still caps at 2048 x 2048 (`atlas.rs:41-43`) and evicts the least recently used shelf; a record of how many glyphs were evicted in the run is part of the result (`atlas_evictions`, read through the new `Backend::atlas_evictions()` with a default of 0, REC-81), and a non-zero value is shown as a warning ("Some labels were drawn from a smaller font cache"). The budget policy (counter and limit) lives in a pure module that compiles natively (T-22). | M1 |
| REC-43 | The recorder canvas is created with `document.createElement('canvas')`, styled only through `element.style` (CSSOM), never through a `style` attribute or a `<style>` element (F2 PR-112), and kept detached; if `getContext('webgl2')` or `new VideoFrame(canvas)` fails for a detached canvas in the M1 spike (T-S1), it is attached with `position: fixed; left: -100000px; width: 1px; height: 1px; pointer-events: none`. The choice is verified by the hidden-tab test (T-13). | M1 |
| REC-44 | After `set_output_size`, the recorder reads `gl.drawingBufferWidth/Height` through a new `Backend::drawing_buffer_size()` and fails with `record_size_unsupported` when it differs from the request (browsers silently shrink oversized drawing buffers). Before that, size limits are checked cheaply: each side at most 4096, coded macroblocks `ceil(w/16) * ceil(h/16)` at most 36864 (H.264 level 5.x frame limit, so 4096x2304 passes and 4096x4096 does not), each side at least 128, and both even. `MAX_VIEWPORT_DIMS` and `MAX_RENDERBUFFER_SIZE` are queried and must cover the size. A size and frame rate with no H.264 level (macroblock rate above level 5.2's 2,073,600 MB/s: 4096x2304 at 60 fps is 2,211,840) is rejected with `record_size_unsupported` ("This size is not supported at 60 fps; use 30 fps or a smaller size"). "Match window" is first resolved by the REC-6 rule, so it never fails these checks. | M1 |
| REC-45 | The recording camera is Gource's automatic camera for `camera-mode`, `follow-user`, `padding`, `disable-auto-rotate`, and the start position. Manual camera gestures in the live view are not replayed (N3); the dialog shows "Camera: automatic. Your zoom and rotation are not recorded." next to the size controls (F3 US-16, T-31). Camera-path capture as part of the recipe (Q6) is an M2 candidate. | M1 |
| REC-46 | A mid-recording WebGL context loss ends the recording with `record_gl_failed` and "Stop and save" (REC-21); the recorder does not attempt to resume, because resumed frames would not be reproducible. | M1 |

### 5.5 Capture and colour

The recorder draws into the default framebuffer (the backend has no FBO or `readPixels`; `backend.rs:147-154`, `docs/features/research/playback-record.md` section 2). Two capture paths are specified; the M1 colour test decides which is the default.

| Path | Mechanism | Pro | Con |
|---|---|---|---|
| A (default candidate) | `new VideoFrame(canvas, {timestamp, duration})` directly after `step_frame` | No readback copy to JS; GPU-resident in Chrome | Colour space of canvas frames is not specified by MDN or the WebCodecs excerpt reviewed; a W3C issue reports darker greens with canvas-sourced frames (https://github.com/w3c/webcodecs/issues/598) |
| B | `gl.readPixels` RGBA into a reused `Uint8Array`, then `new VideoFrame(buffer, {format: 'RGBA', codedWidth, codedHeight, colorSpace, timestamp, duration})` | Explicit colour space under our control | A GPU sync and an 8 MB copy per 1080p frame (33 MB at 4K) |

| id | Requirement | M |
|---|---|---|
| REC-47 | Colour target: BT.709 primaries, transfer and matrix, limited range, 4:2:0, tagged in the stream and the container (`colr` `nclx` 1/1/1 limited). | M1 |
| REC-48 | The colour test (T-3) encodes the Gource test card (background `1A1A1A`, the palette of `colourHash` extensions, white text, bloom) through the real pipeline, decodes it in Chrome (`<video>` + `drawImage` + `getImageData`), and compares with the rendered source. Thresholds come from the M0 spike: every flat patch within the error T-S1 measures plus a 25 % margin, and mean absolute error over the frame within the measured value plus 25 %; the spike result and the thresholds are written into this document before it is approved (section 9, M0). Path A is the default if it passes, else path B, else path A with a tagged-sRGB `colorSpace` experiment. The chosen path and measured errors are recorded in this document. | M1 |
| REC-49 | `isConfigSupported` is called with the exact configuration that will be used (codec string, size, frame rate, bitrate, `hardwareAcceleration`, `avc: {format: 'avc'}`), never a representative one. | M1 |

### 5.6 Encoder configuration

H.264 High is the baseline: High profile, 4:2:0, `avc` format (parameter sets out of band, so `decoderConfig.description` holds the `AVCDecoderConfigurationRecord` the muxer copies into `avcC`; https://www.w3.org/TR/webcodecs-avc-codec-registration/). YouTube recommends the same profile and 4:2:0 (https://support.google.com/youtube/answer/1722171).

Level selection (`avc1.6400LL`, `LL` = `level_idc`): the smallest level of the table below for which the frame size in macroblocks, the macroblock rate and the bitrate (at most 90 % of the level's High maximum, so 4.0 gives way to 4.1 above 22.5 Mbps) all fit (limits from https://en.wikipedia.org/wiki/Advanced_Video_Coding and the H.264 Table A-1):

| Level | `LL` | Max MB/s | Max frame MB | Max bitrate, High (kbit/s) |
|---|---|---|---|---|
| 3.1 | 1f | 108,000 | 3,600 | 17,500 |
| 3.2 | 20 | 216,000 | 5,120 | 25,000 |
| 4.0 | 28 | 245,760 | 8,192 | 25,000 |
| 4.1 | 29 | 245,760 | 8,192 | 62,500 |
| 4.2 | 2a | 522,240 | 8,704 | 62,500 |
| 5.0 | 32 | 589,824 | 22,080 | 168,750 |
| 5.1 | 33 | 983,040 | 36,864 | 300,000 |
| 5.2 | 34 | 2,073,600 | 36,864 | 300,000 |

The bitrate column is the High-profile limit, 1.25 times the Main-profile limit of H.264 Table A-1 (17,500 = 14,000 x 1.25 and so on); the larger High 10 values (3x) do not apply to this High 8-bit configuration. Levels below 3.1 are never selected (decoders accept a stream at a higher level than it needs).

Worked results (macroblocks `ceil(w/16) * ceil(h/16)` times fps): 1280x720 at 30 is 3600 MB and 108,000 MB/s, level 3.1; at 60, 216,000 MB/s, level 3.2. 1920x1080 is 8160 MB (the coded height is 1088): 30 fps is 244,800 MB/s, level 4.0; 60 fps is 489,600, level 4.2. 2560x1440 is 14,400 MB: 30 fps 432,000, level 5.0; 60 fps 864,000, level 5.1. 3840x2160 is 32,400 MB: 30 fps 972,000, level 5.1; 60 fps 1,944,000, level 5.2. 1080x1080 at 30 is 4,624 MB (coded 1088x1088) and 138,720 MB/s, level 3.2. 1920x1080 at 30 with `record-bitrate` 30 is level 4.1 (30 > 22.5), with 100 it is level 5.0. 4096x2304 at 30 is 36,864 MB and 1,105,920 MB/s, level 5.2; at 60 there is no level and the size is rejected (REC-44). These are test vectors for T-8.

Bitrate presets: YouTube's SDR upload recommendations are 5, 8, 16 and 35-45 Mbps at 720p, 1080p, 1440p and 2160p for 24/25/30 fps and 7.5, 12, 24 and 53-68 Mbps for 48-60 fps (https://support.google.com/youtube/answer/1722171). The standard preset takes those values; the others scale them.

| Anchor (30 fps or less) | pixels | `standard` Mbps |
|---|---|---|
| 1280x720 | 921,600 | 5 |
| 1920x1080 | 2,073,600 | 8 |
| 2560x1440 | 3,686,400 | 16 |
| 3840x2160 | 8,294,400 | 40 |

`bitrate(w, h, fps, quality) = interp(pixels) * q * f`, with linear interpolation in pixel count between anchors, proportional extrapolation below 720p (floor 1 Mbps) and above 4K (cap 100 Mbps), `q = 1.0 / 1.5 / 2.5` for `standard / high / max`, and `f = 1.5` when `fps > 30` (YouTube's ratio). `record-bitrate` overrides. The level rises until 90 % of its High maximum covers the bitrate (above), so no clamp is needed; with the presets the largest value, 1080p30 `max` at 20 Mbps, stays under 4.0's 22.5 Mbps. The numbers are starting points [I]: Gource's flat colours usually need less, and the M1 spike (T-3) measures PSNR/size to tune them. `bitrateMode` is `variable`; `latencyMode` is `quality`.

Keyframes: forced every `round(record-keyframe-seconds * fps)` frames with `encode(frame, {keyFrame: true})` (the config has no interval field; https://developer.mozilla.org/en-US/docs/Web/API/VideoEncoder/configure). Every GOP is a self-contained fragment in the fragmented MP4 (REC-57), so seeking and truncation both work. YouTube recommends a GOP of half the frame rate in seconds and closed GOPs; that is an upload recommendation and the platform re-encodes, so the default stays 2 s and `record-keyframe-seconds` can be lowered.

| id | Requirement | M |
|---|---|---|
| REC-50 | `RecordPlan::bitrate_bps` and `RecordPlan::level_idc` are pure functions with native tests against the vectors above, plus property tests: level never decreases when size or fps grows; bitrate within 90 % of the level's High maximum; every row of the table is reachable. | M1 |
| REC-51 | Even dimensions are enforced by validation (REC-44). Non-multiples of 16 (1920x1080) are passed to the encoder as is; the test T-7 checks `isConfigSupported` accepts them and that the decoded size is 1920x1080. | M1 |
| REC-52 | **Support ladder.** The plan returns an ordered list of `EncoderCandidate {codec, width, height, bitrate, framerate, hardware_acceleration, avc_format}`; TypeScript calls `isConfigSupported` on each in order and takes the first accepted. Order: (1) H.264 High at the requested size with the user's hardware preference (default `no-preference`); (2) the opposite hardware preference (`prefer-software`, then `prefer-hardware`); (3) H.264 Main at the same size; (4) the same three at 75 % and 50 % of the size, same aspect, rounded to even, with `ui_scale` unchanged relative to height; (5) VP9 in WebM when `record-format=webm` is implemented (M2); (6) MediaRecorder real-time (REC-85, M2). Each step that differs from the request produces a note for the dialog (REC-13). | M1 (1-4), M2 (5-6) |
| REC-53 | Encoder errors arrive through the `error` callback of `VideoEncoder`; the controller maps them to `record_encoder_failed`, offers "Stop and save", and never retries silently. A non-monotonic output timestamp (a B-frame reorder) is a muxer error `record_mux_reorder` in M1 (no `ctts`/DTS); the controller can retry once with `latencyMode: 'realtime'`, which avoids B-frames in the platform encoders, before failing. Whether Chrome encoders reorder in practice is checked in T-7. | M1 |
| REC-54 | `encoder.flush()` is awaited before `Mp4Writer.finish()`; `frame.close()` is called after every `encode()`; `encoder.close()` is called on every exit path. | M1 |

### 5.7 Muxer crate `gource-mux`

A new workspace member, `publish = false`, GPL-3.0-or-later, edition 2024, no dependencies, `#![forbid(unsafe_code)]`, native-testable, written from the ISO base media file format and the AVC file format boxes (box registry https://mp4ra.org/), not from any third-party muxer. Mediabunny (https://mediabunny.dev/guide/introduction) and the deprecated mp4-muxer (https://github.com/Vanilagy/mp4-muxer) are useful to read for behaviour; no code is copied.

M1 writes **fragmented MP4**:

```
ftyp  (major iso5; compatible iso5 iso6 mp41 avc1)
moov  mvhd (v1, timescale 90000, duration patched at finish)
      trak tkhd (v1, flags 3, width/height 16.16, duration patched) mdia mdhd (v1, 90000, duration 0)
           hdlr 'vide'  minf vmhd dinf/dref stbl:
             stsd avc1 { avcC (copied from decoderConfig.description), colr nclx 1 1 1 limited }
             stts stsc stsz stco (empty)
      mvex  mehd (v1, fragment_duration patched)  trex
      udta/meta (recipe, REC-71, M2)
per GOP:   moof { mfhd, traf { tfhd (default-base-is-moof), tfdt v1, trun (sample duration, size, flags, data offset) } }  mdat
at end:    mfra { tfra, mfro }   (random access index, written by finish())
```

| id | Requirement | M |
|---|---|---|
| REC-55 | `gource_mux::mp4::Mp4Writer` produces byte-identical output for identical inputs (no timestamps from the wall clock; `mvhd` creation/modification times are 0 unless the caller passes them, and the web layer passes 0 in tests). | M1 |
| REC-56 | A fragment is closed when the next keyframe arrives or at `finish()`; fragment size is therefore one GOP (default 2 s, at most about 12 MB at 4K/50 Mbps). | M1 |
| REC-57 | Sample data is passed through unchanged (length-prefixed NAL units in `avc` format; the NAL length size comes from `avcC`). The muxer validates only box-level consistency: the first sample must be a keyframe, timestamps must be strictly increasing, sample sizes must fit in 32 bits. | M1 |
| REC-58 | `finish()` returns the final bytes (last fragment, `mfra`) and `take_patches()` returns the in-place edits that make the file report its duration: `mvhd.duration`, `tkhd.duration`, `mehd.fragment_duration`, each at a known absolute offset (`Patch {offset: u64, bytes: Vec<u8>}`). `mdhd.duration` stays 0: it covers only the samples in `moov`'s own (empty) tables, and AVFoundation (QuickTime, Safari) adds the fragment durations to it, so patching it shows twice the length at half the frame rate. A sink that supports seeking applies them; a sink that does not (a sequential download) gets the file without them, which still plays (duration is then derived from fragments by most players, but not all: Chromium's MediaRecorder MP4 has zero `moov` durations and breaks seek bars in Windows Media Player, https://blog.addpipe.com/duration-in-mp4-files-produced-by-chrome-safari/). With the Blob sink the patches are applied to the buffered bytes, so the file is complete. | M1 |
| REC-59 | M2: `record-layout=faststart` writes a flat MP4 (`ftyp`, `moov` with full sample tables, `mdat`) with `moov` in front, which YouTube lists as its recommended layout. With a seekable sink the muxer reserves a `free` box sized `4 * N_max + 64 KiB` where `N_max` is the exact frame count from the dry run (or `record-max-seconds * fps`), streams `mdat` after it, then at `finish()` writes `moov` into the reserve and turns the remainder into a `free` box. `record-layout=auto` selects `faststart` for a seekable sink with a known `N_max`, else `fragmented`. | M2 |
| REC-60 | M2: `gource_mux::webm::WebmWriter` for VP9 (EBML header, `Segment` with `Info`, `Tracks` `V_VP9`, `Cluster`s of `SimpleBlock`s, `Cues`; unknown sizes while streaming, `Duration` and `Cues` patched at the end with a seekable sink; matroska notes https://www.matroska.org/technical/notes.html). The VP9 codec private data (`vpcC`-equivalent information for WebM is the codec string and profile) is derived from the codec string; AV1 is out of scope. | M2 |
| REC-61 | Native tests of the crate: (a) a box-tree walker in the test module validates sizes, nesting, required boxes and sample table consistency; (b) `Mp4Writer` is fed synthetic samples (all keyframe patterns, 1 to 100,000 samples, key intervals 1 to 300, sizes 0..1 MB, with the total payload per case capped at 64 MiB: the 100,000-sample cases use tiny payloads and the 1 MB sizes only occur with few samples) and the walker's reconstructed sample table equals the input; (c) patches applied to the byte buffer give the correct durations; (d) a truncated output (cut at any fragment boundary, and mid-fragment) walks as valid up to the last complete fragment; (e) if `ffprobe` is on `PATH` (gated by an env var `GOURCE_MUX_FFPROBE=1`), `ffprobe -show_streams` agrees on codec, size, frame count and duration. | M1 |

The wasm API between TypeScript and the muxer (in `crates/gource-web/src/record/wasm.rs`, exported with `wasm-bindgen`; `gource-mux` itself has no wasm dependency):

```ts
class Mp4Writer {
  /** config_json: {"width","height","fps","timescale":90000,"colour":{"primaries":1,"transfer":1,"matrix":1,"full_range":false},
   *                "created":0}; avcc: decoderConfig.description; recipe: optional UTF-8 bytes, the embedded copy of REC-71. */
  constructor(config_json: string, avcc: Uint8Array, recipe: Uint8Array | undefined); // throws
  /** ftyp + moov. Call once, first. */
  header(): Uint8Array;
  /** One encoded chunk. Returns the length of a finished fragment waiting in the muxer's output buffer (0 most of the time); copy it out with take_output(). Throws on a muxer error. */
  push(data: Uint8Array, pts_us: number, is_key: boolean): number;
  /** Copies the pending fragment into `dest` (length >= the value push returned); no allocation per frame. */
  take_output(dest: Uint8Array): void;
  /** Last fragment + mfra. Call after encoder.flush(). */
  finish(): Uint8Array;
  /** In-place edits, valid after finish(): repeated (offset: u64 LE, length: u32 LE, bytes). */
  take_patches(): Uint8Array;
  bytes_written(): number;   // f64
  frames(): number;
  duration_us(): number;
  free(): void;
}
```

`push` copies the chunk once (`EncodedVideoChunk.copyTo` into a reused `Uint8Array` on the TypeScript side, then one wasm call). The fragment leaves through a reused output buffer (`take_output`). The muxer keeps 16 bytes of sample metadata per frame (an hour at 60 fps is 3.5 MB).

### 5.8 Output sink

| id | Requirement | M |
|---|---|---|
| REC-62 | File name: `<stem>_<W>x<H>_<fps>fps_<YYYY-MM-DD>.mp4`, where `<stem>` is, for public repository sources, the repository path with `/` replaced by `-` (`owner-repo`), and `gource` for private (F2) and unknown-provenance sources (local file, `?log=`, REC-103); characters outside `[A-Za-z0-9._-]` become `-`; at most 80 characters of stem; `record-name` replaces the stem. | M1 |
| REC-63 | Sink selection at click time (REC-15): (1) `showSaveFilePicker` + `createWritable()`: sequential `write()` of fragments, then `seek(offset)` + `write(bytes)` for each patch, then `close()`; (2) M2: OPFS staging file (`navigator.storage.getDirectory()`, `createWritable()` on the main thread, `createSyncAccessHandle()` in a worker) followed by a download of the finished `File`, quota checked with `navigator.storage.estimate()` (https://web.dev/articles/origin-private-file-system); (3) in-memory `Blob` built from parts, cap `BLOB_CAP = 1 GiB` (below the 2 GiB x64 Chromium blob limit reported in https://issues.chromium.org/issues/375385420), downloaded through an anchor with `download`, the object URL revoked after the click. When the cap is reached the recording stops and saves what exists with the notice "Stopped at the 1 GiB in-memory limit (mm:ss of mm:ss recorded); use Chrome or Edge to save to disk". | M1 (1, 3), M2 (2) |
| REC-64 | Chrome's File System Access writes are not visible at the destination until `close()`: the specification text says changes are not written to disk until the stream is closed (https://developer.chrome.com/docs/capabilities/web-apis/file-system-access), and Chrome stages them in a temporary file (checked in the M0 spike T-S1; the UI states only the consequences below, which hold either way). Consequences, stated in the UI and in section 10: a tab crash or close loses the whole recording; there is no "play while recording". Seek and truncate are supported (same source; https://developer.mozilla.org/en-US/docs/Web/API/FileSystemWritableFileStream/seek). | M1 |
| REC-65 | Sink failures are classified by the `DOMException` name, never by message text (which is locale- and browser-dependent): `QuotaExceededError` maps to `record_disk_full` ("The disk is full; free some space and try again"); every other rejection (`NotAllowedError`, `InvalidStateError`, `NotFoundError`, anything else) maps to `record_sink_failed` with the name as a hint. After either, the controller always attempts `close()` once and reports the actual result: "Stop and save" is offered, and if the close fails the message says nothing could be saved. Tested with an injected failing sink (T-28). | M1 |
| REC-66 | M2 optional **crash-safe mode** (`record-checkpoint-seconds`, default off): every N seconds of wall time the controller `close()`s the writable and reopens it with `createWritable({keepExistingData: true})`, committing a valid fragmented file. Chrome copies the existing file into a new temporary file on reopen (measured in T-S2), so cost grows with file size; the mode is capped at 8 checkpoints per recording and is shipped only if T-S2 shows an acceptable cost. | M2 |

### 5.9 Length estimate, dry run and progress

Run length depends on the log, `seconds-per-day`, `time-scale`, auto-skip and the stop conditions (formulas: `sim.rs:988`, `568`, `1013-1042`, `1017-1024`; `docs/features/research/playback-record.md` section 3). A pure Rust function computes, from the cloned `EventLog` and the snapshot, in O(commits):

- `span_days = sum over commits in order of |ts[i+1] - ts[i]| / 86400`, restricted to `start-date`/`stop-date`/`start-position`/`stop-position`. The sum of absolute gaps (not last minus first) bounds logs with out-of-order timestamps, where the clock rewinds unless `no-time-travel` (`sim.rs:1032-1042`).
- `upper_s = span_days * seconds_per_day / time_scale + tail_s`, where `tail_s` is `record-tail-seconds`, or `TAIL_ALLOWANCE_S / min(1, time_scale)` (12 s at `time-scale` 1) for -1, so `upper_s >= exact` holds for every tail setting. With auto-skip disabled this is also the expected length.
- `about_s = sum over gaps of min(gap_days * seconds_per_day / time_scale, auto_skip + SETTLE) + tail_s`, with `SETTLE = 2.0` s (beam and idle settle time, heuristic [I]) when auto-skip is on; equal to `upper_s` otherwise.

Both are clipped by `record-max-seconds`. M1 shows only "up to Y"; M2 shows "about X, up to Y". The same pass builds the cumulative table `expected(i)` (video seconds at commit index `i`) that REC-17 uses for progress.

| id | Requirement | M |
|---|---|---|
| REC-122 | **Empty runs.** When the resolved range contains no commit (an empty log, `start-date` after the last commit), the plan has `estimate.empty = true`, the dialog disables Record with the reason "The selected range has no commits", and `GourceRecorder::create` fails with `record_empty_run`. Filters (`user-filter`, `file-filter`, ...) can empty a run the cheap count cannot see; the preview (REC-7) reports `commits == 0` in `stats_json` when the run finished before any commit was processed, which sets the same state. A range with exactly one commit is valid: the video shows that commit and the tail. Tests in T-4 (including `empty.log` and `single-event.log`). M1 covers the plan-based check; the filter-based check through the preview is M2. | M1 |

The **dry run** (`RecordPlanner`) is a `Simulation` driven exactly as the real run (same snapshot, same `set_viewport` aspect and `ui_scale`, per tick `update` then `extract_view`, no frame builder, no backend), counting ticks until the stop conditions of section 5.3. The `extract_view` call is required, not optional: it mutates simulation state (visibility caches, hover, scratch lists, `crates/gource-core/src/sim/extract.rs:163-215`), so omitting it could change later ticks. The result is the exact frame count `N`, duration `N / fps`, and the size bound `bitrate * N / fps / 8`. Cost, from `PERFORMANCE.md` (sim plus extract about 1.0 ms/tick on git/git and 8.8 ms/tick at 100k files in wasm): a 5,000-tick run is about 5 s on git/git; at 100k files about 44 s. That is about 40 to 60 percent of the real run, so it is optional and sliced (`run_for(ms)`, 8 ms slices through macrotask yields) so the dialog stays responsive and cancelable.

Decision: M1 ships the upper bound plus the progress/ETA by the expected-time table (REC-17, REC-18); M2 adds the displayed `about X` with its accuracy test (REC-69), the dry run and uses its exact `N` for `faststart` reservation (REC-59) and for the progress denominator. The two-pass cost is only paid when the viewer asks or the estimate exceeds 10 minutes.

| id | Requirement | M |
|---|---|---|
| REC-67 | `RecordPlanner::run_for(ms: f64) -> u32` (ticks run in this slice; 0 when done), `done()`, `result_json()`. The planner and the recorder share one stepping function, so they cannot diverge. | M2 |
| REC-68 | Test: for every fixture log under `tests/fixtures/custom-log/` and `tests/logs/`, at 30 fps with defaults, the planner's frame count equals the number of frames the real recorder (native, `FakeBackend`) emits; for `empty.log` both report `record_empty_run`. | M2 |
| REC-69 | Test: the heuristic `about_s` is within 0.5x to 2x of the exact duration on those fixtures at `seconds-per-day` 0.1, 1 and 10, and `upper_s >= exact` always, also at `time-scale` 0.5 and with `record-tail-seconds` -1 (the `upper_s >= exact` half is already asserted in M1, T-9a). | M2 |

### 5.10 Recipe and embedded metadata

The recipe is JSON, versioned, and contains no tokens, no contributor emails and no data from private repositories beyond the repository identifier (REC-102). It is written in two renderings. The **embedded copy** is written by `Mp4Writer::header()` before the first frame, so it holds only inputs known then. The **sidecar** is written at the end and adds the result.

```json
{
  "v": 1,
  "app": "gource-web", "build": "<git sha of the web build>", "engine": "gource-core <version>",
  "source": {"kind": "repo", "host": "github.com", "path": "owner/repo", "branch": "", "since": "", "until": "",
             "subdir": "", "head": "<sha>", "private": false},
  "options": {"seconds-per-day": "2", "hide": "filenames", "hash-seed": "31"},
  "record": {"record-framerate": "30", "record-viewport": "1920x1080", "record-quality": "high", "record-tail-seconds": "5"},
  "claims": [],
  "plan": {"fps": 30, "ticks_per_frame": 2, "fixed_dt": 0.016666668, "width": 1920, "height": 1080, "ui_scale": 1.0},
  "utc_offset_minutes": 120,
  "redacted": ["user-filter"]
}
```

`source.private` is derived from F2 `repo.visibility` (F2 PR-93). The sidecar adds, next to the same fields:

```json
  "result": {"frames": 11100, "duration_s": 370.0, "exact": false, "codec": "avc1.640028", "bitrate_bps": 12000000,
             "encoder": {"requested": "no-preference", "accepted": "avc1.640028"}, "limit": null, "atlas_evictions": 0},
  "time_zone": "Europe/Berlin", "created": "2026-10-02T12:00:00Z"
```

| id | Requirement | M |
|---|---|---|
| REC-70 | `options` holds the non-default Gource options of the snapshot (the URL serialisation; multi-value options such as `hide`, `user-style` and the filters are stored as the string `option_value()` returns, lines joined by `\n`, and the URL form uses repeated parameters), `record` holds the non-default recording options with `record-viewport` and `record-ui-scale` resolved to their concrete values, `claims` holds F1's `claims_snapshot()` (REC-73), and `plan` the parameters that determine the frames (fps, size, `ui_scale`, ticks per frame, `fixed_dt`). The embedded copy has no frame count, duration or encoder, because those are estimates or unknown when `header()` runs and change after "Stop and save", `limit_reached` or a ladder downgrade; they live in the sidecar's `result` (`exact` is true only after a dry run). The reproducibility scope is "same build and same log" (REC-73). | M1 |
| REC-71 | Embedding (M2; in M1 the recipe travels as the reproduce link and the sidecar, REC-72, REC-73): when `record-metadata=embed`, the muxer writes `moov/udta/meta` (handler `mdir`) with an `ilst` containing `©cmt` (one-line human text with the reproduce link), `©too` ("gource-web <build>", a neutral string; the maintainer confirms the naming against upstream Gource's guidance) and a freeform item `----` (`mean` `org.gource-web`, `name` `recipe`, data = recipe JSON, UTF-8). `ffprobe` shows `comment` and `encoder` tags. The recipe is capped at 16 KiB; larger recipes are truncated to the link only and flagged. The embedded copy leaves out `time_zone` and `created` (it keeps the UTC offset at the first commit) and the values of the options in `NAME_BEARING_OPTIONS` (F1 AI-37, F2 PR-91); their names are listed in `redacted`, and the full values stay in the sidecar and in the link the viewer chooses to copy. For private and unknown-provenance sources the embedded copy also omits `source.path`, `branch`, `subdir` and `head` even with `record-metadata=embed`, unless the viewer confirms in the same step as REC-72, and the `©cmt` link then carries no `repo=`. The `©cmt` link is built from the embedded copy. The Details popover (REC-87) shows the exact text that will be embedded. | M2 |
| REC-72 | Sidecar: "Download recipe (.json)" on the finished screen, same content, `<name>.recipe.json`; with File System Access and `record-metadata=embed`, a second `showSaveFilePicker` is not requested (no activation); the button downloads it. For private and unknown-provenance sources the sidecar redacts `path` and `head` unless the viewer confirms in a step that names them, because the sidecar is the file most likely to be attached to a bug report. | M1 |
| REC-73 | Reproduce link: page URL (`origin + '/'`) with `repo`, `branch`, `since`, `until`, `path`, every non-default Gource option, every non-default recording option with resolved `record-viewport`/`record-ui-scale`, and, from M2, `record=1` (in M1 the link opens the run and the viewer presses Record). Opening it loads the run (and from M2 opens the prefilled dialog, REC-3). A whole-history run with no `until` pins `until` to the date of the head commit, so later commits do not enter; opening a link whose recipe `head` differs from the repository's current head shows "The history has changed since this recording", and a `build` mismatch shows "Made with build X; frames can differ". The reproducibility claim is therefore exactly: same build and same log. Claimed contributor styles (F1 AI-49, D4b, the later spec "claimed styles") are captured at `begin_recording()` as the recipe field `claims` = `claims_snapshot()`; the fork applies them from the recipe, never from the viewer's account; with `claimed-styles=off` the field is empty. A local-file source has no reproduce link (the button is hidden; the sidecar still downloads); `?log=` sources link to the same `?log=`, which on a private-mode deployment needs the origin in `--csp-connect-src` (F2 PR-113). For private sources (F2) the link works only for viewers with access and never carries credentials. A reproduce link longer than `SHARE_URL_SOFT_LIMIT` (2,000 characters, README) is offered as the recipe JSON instead, with the note that sign-in may drop options (F2 PR-10). | M1 |
| REC-74 | Time zone: dates in the video follow the browser's zone (`engine.rs:200-236`). The recipe records the IANA zone (`Intl.DateTimeFormat().resolvedOptions().timeZone`) and the offset at the first commit; M3 server rendering applies it through the browser's timezone override (CDP `Emulation.setTimezoneOverride`). This is the one known input that the URL state does not carry; Q4. | M1 (recorded) / M3 (applied) |
| REC-75 | The hash seed is part of the snapshot (it is a `Settings` field) and the recorder gets no key input, so a colour reseed (`s`) cannot happen mid-recording. The recipe records `hash-seed` even when it is the default 31. | M1 |

### 5.11 Errors

Codes follow the style of session errors (`web/src/session.ts:28-31`, `fail(code, message)`); all are client-side in M1.

| Code | When | Message (dialog copy) | Actions |
|---|---|---|---|
| `record_unsupported_browser` | feature detection fails: no `VideoEncoder`, no secure context, or no WebGL2 | "This browser cannot encode video here. Chrome and Edge, opened over https or localhost, are tested." | Close |
| `record_empty_run` | REC-122 | "The selected range has no commits." | Fix range |
| `record_not_started` | `step_frame` before `begin_recording` (a programming error) | none (logged) | none |
| `record_log_incomplete` | log not complete (REC-41) | "The full history has not arrived yet." | Close |
| `record_invalid_option` | `RecordOptionError` | field-specific reason | Fix field |
| `record_unbounded` | `loop` with no `record-max-seconds` | "A looping recording needs a maximum length." | Fix field |
| `record_no_encoder` | ladder exhausted (REC-52) | "This browser cannot encode video at any size we tried. Try Chrome or Edge, or lower the resolution." | Choose size, M2: real-time fallback |
| `record_size_unsupported` | REC-44 | "This size is not supported on this device (largest: WxH)." | Choose size |
| `record_gl_failed` | context loss or creation failure (REC-46) | "The graphics context was lost; the recording stopped." | Stop and save |
| `record_encoder_failed` | `VideoEncoder` error callback (REC-53) | "The video encoder reported an error: <name>." | Stop and save |
| `record_mux_failed`, `record_mux_reorder` | muxer errors | "The video file could not be assembled." | Stop and save |
| `record_disk_full`, `record_sink_failed` | REC-65 | see REC-65 | Stop and save when possible |
| `record_too_large` | in-memory cap reached (REC-63) | notice, not an error | saved partial |
| `record_cancelled` | Discard | none | none |
| `record_tab_closed` | not reportable; `beforeunload` confirmation only (REC-20) | n/a | n/a |

### 5.12 Per-crate and per-file changes

| Where | Change |
|---|---|
| `crates/gource-core/src/sim.rs:602-612` | Add `Simulation::settings_snapshot() -> Settings` (the `restart` adjustment of `base_file_idle_time` as a reusable function). Add `app_finished()`, `commit_cursor()` and `tree_tweaked()` accessors next to `is_finished` (`sim.rs:719-729`) and expose them in `SimStatus` (`sim.rs:646-660`). No behaviour change. |
| `crates/gource-core/src/view_hash.rs` (moved from `crates/gource-cli/src/hash.rs`) | `hash_view` and `ENCODING_VERSION` move to `gource-core` (the function uses only `glam` and `SceneView`) and `gource-cli` re-exports them, so `gource-web` can expose a per-frame pre-encode hash (`frame_hash`). The byte stream does not change (the encoding stays version 1 byte for byte for views whose `decor.mask` is 0, F1 AI-62), so no golden file changes (REC-82). |
| `crates/gource-web/src/status.rs:10-45` | `EngineStatus` gains `app_finished: bool`, `log_complete: bool`, `commit_cursor: u32` and `tree_tweaked: bool` (REC-80). The mock engine (`web/src/engine/mock.ts`) reports them too. |
| `crates/gource-web/src/engine.rs` | `Engine::fork_for_recording(backend, clock) -> Engine` (log clone, `Simulation::new(settings_snapshot)`, zone), `Engine::set_output_size(w, h, ui_scale)` next to `resize` (`engine.rs:386-405`), `Engine::restart_for_recording()` (fresh simulation from the snapshot, frame and pts counters zeroed, glyph atlas reset; REC-120). `step` (`engine.rs:326-346`) and `set_fixed_dt` (`engine.rs:367`) are reused unchanged. |
| `crates/gource-web/src/record/` (new module) | `options.rs` (`RecordOptions`), `plan.rs` (`FramePlan`, `RecordPlan`, level, bitrate, ladder, size validation, estimate), `driver.rs` (`RecordDriver<B, C>`: stepping and stop conditions, generic over backend, native-testable with `FakeBackend`, `engine.rs:566+`), `source.rs` (`LogSource`), `planner.rs` (dry run), `wasm.rs` (`GourceRecorder`, `RecordPlanner`, `Mp4Writer`). Registered in `crates/gource-web/src/lib.rs`. |
| `crates/gource-web/src/wasm.rs:58-78` | No change to `GourceApp::new`; a `pub(crate)` accessor gives `GourceRecorder::create` the engine to fork. |
| `crates/gource-render/src/lib.rs:246-257` | `Backend::drawing_buffer_size(&self) -> Option<[u32; 2]>` with a default of `None` (REC-44) and `Backend::atlas_evictions(&self) -> u32` with a default of 0 (REC-42); `FakeBackend` (`crates/gource-web/src/engine.rs:566+`) gets matching test hooks. |
| `crates/gource-render-webgl2/src/backend.rs:346-356` | Implement `drawing_buffer_size` from `gl.drawing_buffer_width()/height()`; `WebGl2Backend::with_options(canvas, BackendOptions {frame_glyph_budget})` (REC-42). The default `WebGl2Backend::new` is unchanged. |
| `crates/gource-render-webgl2/src/atlas.rs:48,390,405` | `GlyphAtlas::set_frame_budget(u32)`; evictions counter. The budget and eviction policy (counter, limit, what counts as deferred) moves into a pure `budget.rs` compiled like `packer.rs` (`any(wasm32, test)`), because `atlas.rs` and `backend.rs` are `wasm32` only (T-22). |
| `crates/gource-mux/` (new) | `lib.rs`, `mp4/{boxes,writer,patch,meta}.rs`, `webm/` (M2), tests (REC-61). Workspace member. |
| `crates/gource-cli/` | M2: `gource-rs record-plan <log> [--options ...] [--record-framerate N] [--record-viewport WxH]` prints the plan (frames, duration, level, bitrate) and, with `--hash`, the frame hashes at the plan's cadence, for parity with the browser. It reuses `FramePlan`, `RecordOptions` and `RecordDriver` from `gource-web`, so `gource-cli/Cargo.toml` gains the dependency `gource-web` (an `rlib`; `gource-render-webgl2` is an empty crate on non-wasm targets and the wasm-bindgen code is `cfg(target_arch = "wasm32")`, checked by `cargo build -p gource-cli`). |
| `web/src/record/` (new) | `dialog.ts`, `controller.ts` (loop of section 5.1, backpressure, stop/cancel), `encoder.ts` (ladder, `VideoEncoder` wrapper), `sink.ts` (File System Access, Blob; OPFS in M2), `options.ts`, `recipe.ts`, `mux.ts` (typed wrappers over the wasm classes). |
| `web/src/screens/viewer.ts` | Record button and `Alt+R` (REC-1, REC-2), banner and pause during recording (REC-20), `engine` handle passed to the dialog. The rAF loop and input binding are unchanged. |
| `web/src/url-state.ts`, `web/src/options.ts` | `record=1` and recording option keys are parsed and written next to Gource options; they are not added to `OPTION_DEFS` (they never reach `set_option`). `RESTART_OPTIONS` is unaffected. |
| `web/src/engine/types.ts`, `web/src/engine/load.ts`, `web/src/engine/mock.ts` | Interfaces `RecorderLike`, `PlannerLike`, `MuxerLike` for `GourceRecorder`, `RecordPlanner`, `Mp4Writer`; `EngineFactory` gains `createRecorder`, `createPlanner`, `createMuxer`. The wasm factory wraps the wasm classes. `loadEngine(true)` never loads wasm, so the mock factory supplies a Canvas2D recorder (draws the same placeholder scene at the output size), a byte-counting muxer that produces a small well-formed container for the sink tests, and `EngineStatus.app_finished/log_complete/commit_cursor/tree_tweaked`. The controller depends only on the interfaces (REC-93). |
| `docs/web-mvp/PARITY.md` | A note on `ui_scale` for exports (REC-38) and the 24 fps and tail differences. |
| `CLAUDE.md`, `docs/web-mvp/PLAN.md` | Crate list gains `gource-mux`; "video export" leaves the out-of-scope list. |

| id | Requirement | M |
|---|---|---|
| REC-80 | `app_finished`, `log_complete`, `commit_cursor` and `tree_tweaked` added to `SimStatus`/`EngineStatus` and the TypeScript type; existing consumers unaffected (JSON fields only added). | M1 |
| REC-81 | `Backend::drawing_buffer_size` and `Backend::atlas_evictions` have defaults so `gource-render` tests and `FakeBackend` compile unchanged. | M1 |
| REC-82 | The `hash_view` move precedes F1 M1b (or F1 applies its DECO section at the new path); the encoding stays version 1 byte for byte for views whose `decor.mask` is 0 (F1 AI-62). The parity and frame-hash tests (`docs/web-mvp/PARITY.md`, `hash_view` after its move to `gource-core`) pass with no golden file changes. | M1 |

### 5.13 Live viewer and the recording together (M1)

M1 runs the recorder on the main thread, next to the live viewer, with the live simulation paused (REC-20). The recorder's work and the live viewer's per-frame work (a paused tick plus build and draw, about 0.7 ms on git/git per `PERFORMANCE.md`) share the main thread; the export loop yields a macrotask after every frame, so input and painting continue (each recorder frame costs 1.7 ms to 15 ms of CPU, section 7). Two WebGL contexts exist at once (the browser limit is 16).

M2 moves the recorder into a dedicated Worker with an `OffscreenCanvas`, so the live viewer stays fully real-time. Requirements: the backend accepts an `OffscreenCanvas` (`web_sys::HtmlCanvasElement` is hard-coded in `backend.rs:130,146`), the glyph atlas rasterises with an `OffscreenCanvas` 2D context and takes a canvas factory instead of `&Document` (`atlas.rs:166`, `backend.rs:163`), and the worker loads `FreeSans.ttf` with `new FontFace('GourceSans', arrayBuffer)` added to `self.fonts` (the document's face is not shared with workers). The second wasm instance cannot clone the `EventLog`, so the worker path is the second `LogSource` implementation (REC-41, section 5.4), not a rewrite of `GourceRecorder::create`: the main thread sends `GourceApp.export_log()` (a new function that encodes the `EventLog` as batches, using the existing `BatchEncoder`/`apply_batch` contract, `crates/gource-sources/src/batch.rs:98,419`; it must write F1's batch v2 with the set table once F1 M1a has landed) and `Settings` as option lines, multi-value options such as `user-style` as one string joined by `\n`; the worker applies them. The worker is a same-origin module worker (no inline or blob worker), so `worker-src 'self'` suffices under F2's CSP. `VideoFrame(OffscreenCanvas)` and `VideoEncoder` exist in workers (https://developer.mozilla.org/en-US/docs/Web/API/VideoEncoder). Timers in dedicated workers are reported as not throttled in background tabs, and `OffscreenCanvas` is widely available since March 2023 (https://developer.mozilla.org/en-US/docs/Web/API/OffscreenCanvas); the exact worker behaviour is verified in T-13.

| id | Requirement | M |
|---|---|---|
| REC-83 | M1 hidden-tab behaviour (main thread): the export loop uses no `requestAnimationFrame` and no `setTimeout`/`setInterval`; it advances on encoder and sink promises and a `MessageChannel` macrotask. Chrome does not call rAF in background tabs and throttles timer chains (https://developer.chrome.com/blog/background_tabs); message-port tasks are not timers, so the loop should keep running; T-13 asserts at least 25 % of the foreground rate. If T-13 shows the main-thread loop is throttled below 25 % of its foreground rate, M1 shows "Keep this tab visible while recording" in the progress panel and sets the tab title to "Recording (switch back to continue)" while hidden; the worker export of M2 removes the limitation. Pages are not exempt from freezing by holding Web Locks (the lifecycle guidance does not list them: https://developer.chrome.com/docs/web-platform/page-lifecycle-api), so no Web Lock is taken. | M1 |
| REC-123 | Because File System Access output is committed only at `close()` (REC-64) and a hidden tab can be frozen or discarded under memory pressure or while a laptop sleeps, the controller requests `navigator.wakeLock.request('screen')` while `running` (feature-detected, in try/catch, the recording works without it), re-acquires it on `visibilitychange` and releases it on every exit path. A wake lock is not a guarantee, so the progress panel keeps the REC-83 notices and the documented maximum hidden duration is whatever T-13's slow variant measures. | M2 |
| REC-84 | M2 hidden-tab behaviour (worker): the loop runs in the worker, which has no rAF dependency; the main thread only relays progress. | M2 |

### 5.14 M3 outline: server-side rendering

Specified in detail when M3 starts; the constraints now:

- One render path: headless Chrome loads `/?render=<job>` which runs the same controller (`web/src/record/controller.ts`) with a sink that writes to OPFS or memory, and the server collects the file through the DevTools protocol. No second renderer to keep in parity (`docs/features/research/video-research.md` section 8 rejected a native `wgpu` + ffmpeg path for that reason).
- API: `POST /api/renders` with the recipe (202 `{render, queue_position}`; 400 `invalid_recipe`; 429 `busy|rate_limited|quota_exceeded`), `GET /api/renders/:id` (state `queued|preparing|rendering|encoding|done|failed|cancelled|expired`, progress, frames), `GET /api/renders/:id/events` (SSE like `/api/jobs/:id/events`), `GET /api/renders/:id/file` (Range supported), `DELETE /api/renders/:id`.
- Quotas as flags with env mirrors, in the style of `gource-server` (`--render-max-seconds`, `--render-max-pixels`, `--render-concurrency`, `--render-daily-seconds-per-client`, `--render-retention`), cache key `sha256(canonical recipe, build id)` for public sources only. Private renders never use the shared cache: their key also includes F2's namespace (`installation_id`, `account_id`, head sha), so a second account submitting the same recipe cannot be served another account's render.
- Private repositories (F2): the render browser receives no credentials; it receives the single-use ticket of F2 PR-110 and loads the log with it; the output is stored under the owner's id with the F2 private TTL (sealed when F2 M2b is deployed) and purged by F2 PR-66. `--render-retention` for private outputs must be at most `--private-cache-ttl-secs` (F2 PR-67). Prerequisite: F2 M2a (webhooks, private cache purge) and PR-110. For private renders the browser-facing `GET /api/renders/:id/file`, `/events` and `DELETE` require the session cookie, the CSRF checks of PR-21 and an owner match; there are no bearer-style URLs, and responses carry `Cache-Control: no-store`. They are purged on logout of the last session, disconnect, webhook events and TTL (PR-66, PR-82, PR-83).
- Input validation: the server validates the recipe. Only `source.kind = repo` with the hosts `gource-server` already accepts is rendered; `?log=` URLs, local-file sources and unknown options are rejected or dropped (`400 invalid_recipe`), so the render browser cannot be steered to arbitrary URLs. The render browser runs with an egress allowlist of only the gource-server origin (no metadata or internal hosts). Cookie-authenticated `POST` and `DELETE /api/renders` inherit PR-21.
- Abuse: render quotas apply to signed-in accounts (F2). Anonymous renders are limited by a global concurrency cap only (`--render-anonymous-concurrency`, default 0, meaning off), because anonymous identities are free to rotate and software GL makes each job expensive.
- Codec: headless Chromium builds often lack H.264 and hosted H.264 encoding carries licensing considerations. The Docker image ships no H.264 encoder; server renders use VP9/WebM by default (M2 format) and H.264 only on hosts that use branded Chrome where the operator accepts the licensing.
- Time zone through the recipe (REC-74). GPU-less hosts use software WebGL (SwiftShader), which is slow; GPU hosts are a deployment choice.
- Frames are not bit-identical to a desktop GPU's (rasterisation differences) and encoders differ; the guarantee is the same simulation and the same camera, not identical bytes.

### 5.15 Controller, state machine and wasm API (reference)

Controller states (`web/src/record/controller.ts`; the dialog renders one screen per state):

| State | Entered when | Leaves on |
|---|---|---|
| `closed` | initial; dialog closed; recorder freed | Record pressed or `record=1` loaded |
| `configuring` | dialog open: fork created (`GourceRecorder.create`), plan computed, preview rendered | Cancel (back to `closed`, fork freed), Record pressed |
| `starting` | picker resolved (or skipped, REC-16), encoder configured with the chosen candidate, `begin_recording()` called (REC-120), `Mp4Writer` created on the first chunk, live engine paused | first frame encoded (`running`), any startup error (`failed`) |
| `running` | frame loop active | `LAST` flag (`stopping`), Cancel then "Stop and save" (`stopping`), Cancel then "Discard" (`cancelled`), error (`failed` or `stopping` when something can be saved) |
| `stopping` | `encoder.flush()`, `Mp4Writer.finish()`, patches, `close()` | success (`done`), error (`failed`) |
| `done` | file written | Record again (`configuring`), close |
| `failed` | an error code of section 5.11 | Close; "Stop and save" is offered only from `running`-time errors |
| `cancelled` | Discard | close |

Loop, as a sketch (not final code):

```ts
async function run(c: Ctx): Promise<void> {
  let last = false;
  while (!last && !c.stopRequested) {
    const flags = c.rec.step_frame();           // ticks_for_frame(n) ticks, one draw (Rust)
    last = (flags & LAST) !== 0;
    const frame = new VideoFrame(c.canvas, { timestamp: c.rec.pts_us(), duration: c.rec.duration_us() });
    c.enc.encode(frame, { keyFrame: (flags & KEY) !== 0 });
    frame.close();                               // same task as the draw
    if (c.thumbDue()) c.thumbCtx.drawImage(c.canvas, 0, 0, c.tw, c.th);   // same task
    c.progress(c.rec.progress(), c.rec.frame_index());
    await c.gate();                              // dequeue / pending writes / MessageChannel yield
  }
  await c.enc.flush();                           // then Mp4Writer.finish(), patches, close()
}
// encoder output callback: first chunk builds Mp4Writer(avcC from meta.decoderConfig.description);
// every chunk: copyTo(reusedBuf); bytes = mux.push(reusedBuf, chunk.timestamp, chunk.type === 'key');
// if (bytes.length) c.sinkChain = c.sinkChain.then(() => c.sink.write(bytes));
```

The controller and dialog depend on the interfaces `RecorderLike`, `PlannerLike` and `MuxerLike` (`web/src/engine/types.ts`), obtained from `EngineFactory.createRecorder/createPlanner/createMuxer` (section 5.12), never on the wasm classes directly; that is what lets the mock suite run them. `GourceRecorder` and `RecordPlanner` (wasm-bindgen, `crates/gource-web/src/record/wasm.rs`; errors are `JsError` whose message is `"<code>: <text>"` with a code of section 5.11, read with the shared helper `parseWasmError(e) -> {code, text}` of `web/src/errors.ts`, README and F1 AI-46):

```ts
class GourceRecorder {
  /** Forks the live engine (log clone, settings snapshot) onto `canvas`; options_json is the dialog's overrides + recording options. */
  static create(app: GourceApp, canvas: HTMLCanvasElement, options_json: string): GourceRecorder;
  /** New overrides: applies options, restarts the simulation, resizes the output, redraws. Keeps the canvas and context. */
  reconfigure(options_json: string): void;
  plan_json(): string;            // RecordPlan, below
  /** Preview, sliced: renders the run `sim_seconds` after the start without capturing, at most `ms` of work per call; returns true when done. reconfigure() cancels it. */
  preview_for(sim_seconds: number, ms: number): boolean;
  /** Ends configuring: restarts the simulation from the start, zeroes the frame and pts counters, resets the glyph atlas and the progress clamp. step_frame() before it throws `record_not_started`. */
  begin_recording(): void;
  /** Hex `hash_view` of the last frame's SceneView: the pre-encode oracle of T-5, T-16 and T-27. */
  frame_hash(): string;
  step_frame(): number;           // flags: 1 LAST, 2 KEY, 4 IN_TAIL, 8 LIMIT
  frame_index(): number;          // index of the frame just rendered
  pts_us(): number;  duration_us(): number;
  progress(): number;             // 0..1, monotone
  sim_date(): string;
  stats_json(): string;           // ticks, files, users, commits, draw_calls, atlas_evictions
  free(): void;
}
class RecordPlanner {             // M2
  static create(app: GourceApp, options_json: string): RecordPlanner;
  run_for(ms: number): number;  done(): boolean;  result_json(): string;  free(): void;
}
function parse_record_options(pairs_json: string): string;   // validated, normalised {name: value}; throws record_invalid_option
```

```json
{"fps":30,"ticks_first":1,"ticks_per_frame":2,"tick_hz":60,"fixed_dt":0.016666668,
 "width":1920,"height":1080,"ui_scale":1.0,"level_idc":40,"codec":"avc1.640028","bitrate_bps":12000000,
 "keyframe_every":60,"max_frames":null,
 "estimate":{"about_s":250.0,"upper_s":390.0,"exact":false,"frames":null,"empty":false},
 "ladder":[{"codec":"avc1.640028","width":1920,"height":1080,"bitrate":12000000,"framerate":30,
            "hardware_acceleration":"no-preference","avc_format":"avc"}],
 "warnings":["upper bound over 60 minutes"],"limits":{"max_side":4096,"max_macroblocks":36864}}
```

| id | Requirement | M |
|---|---|---|
| REC-85 | M2 real-time fallback (ladder step 6): `captureStream(0)` plus `requestFrame()` on the recorder canvas driven by the recorder in real time (`GourceRecorder` with `fixed_dt` off), `MediaRecorder` with the first of `video/mp4;codecs=avc1.640028` and `video/webm;codecs=vp9` that `isTypeSupported` accepts. The dialog says "Real-time recording" and the finished screen names the limits. The output carries no recipe (the container is the browser's). | M2 |
| REC-86 | Record is not offered on touch-first devices (`matchMedia('(pointer: coarse)').matches` and viewport narrower than 768 CSS px): mobile browsers freeze background tabs and have smaller encoders. The share button still copies the reproduce link. | M1 |
| REC-87 | The dialog's "Details" popover lists what is going to happen: codec string and level, tick rate and ticks per frame, bitrate, keyframe interval, hardware preference and the candidate chosen by the ladder, estimated frame count, the container layout, the sink (file on disk or memory), "Embedded in the file" with the exact text of REC-71 (M2; in M1 "Nothing is embedded"), and "Not recorded" (camera gestures, gravity keys). | M1 |
| REC-88 | The dialog remembers size, frame rate, quality, format and "end after" in `localStorage` (every access in try/catch, the dialog renders without it); title, file name, range and speed are never remembered, because they belong to the run. URL parameters win over remembered values. | M2 |
| REC-89 | The fork created for the dialog is reused for the recording (same `GourceRecorder`, `reconfigure` for changes) and freed when the dialog closes or the recording finishes (REC-118). Reuse is only through `begin_recording()` (REC-120): the preview-advanced simulation is restarted, the counters and glyph atlas are reset, and a recording after a preview gives the same view hashes as a fresh recorder (T-27). Only one recorder exists at a time; a second Record press while `running` focuses the progress panel. | M1 |
| REC-120 | `GourceRecorder::begin_recording()` (and `Engine::restart_for_recording`) restarts the fork's simulation from the snapshot, zeroes the frame index and pts counters, clears the glyph atlas and the progress clamp. Calling `step_frame()` before it is the error `record_not_started`. | M1 |
| REC-94 | Accessibility: the dialog is `role="dialog"` with `aria-modal`, a focus trap, labels on every control, Esc closes it when `configuring`, opens the cancel choice when `running`, and returns to "Keep recording" when pressed inside the cancel choice; the progress bar is `role="progressbar"` with `aria-valuenow`; a polite live region announces start, each 10 % step and completion; no motion is needed to understand state: under `prefers-reduced-motion` the thumbnail refresh and the progress bar animation are off. The dialog carries a one-line note that the exported video can contain flashing (beams and bloom), with the `bloom` toggle in "Show" as the remedy. | M1 |
| REC-95 | Test hook (like `window.__gource`, `web/src/main.ts:314-319`): `window.__record = {state(), frames(), cancel(choice), sinkBytes(), plan(), frameHash()}` and a `window.__recordSink` override of `showSaveFilePicker` (REC-92) are installed only in builds made with the build-time flag `VITE_E2E_HOOKS=1` (set by the e2e build of `scripts/serve.sh`, absent from the production build and the Docker image) and then only under `?fixed-dt` or `?mock-record=1`. While they are active a visible banner reads "Test mode: nothing will be saved". A production build ignores both URL parameters for this purpose (test). | M1 |

### 5.16 Relation to F1 and F2

- F1 (AI co-authors, contributor styles): style rules are Gource-style options in `Settings`, so the snapshot (REC-5), the recipe and the reproduce link carry them with no work in F3; the recipe lists exactly the F1 options that are non-default (`user-style`, `ai-style`, `ai-filter`, `agent-key`, `decor-motion`, `hide=decorations`, `claimed-styles`, F1 AI-72) and F1's `claims` field (REC-73), and T-16a asserts it. Sprite textures are compiled into the wasm, so the recorder draws them. Per-contributor data derived from the log lives in the `EventLog` or in `Settings`, both of which the fork copies. F3 only requires that F1 adds nothing to a frame that depends on wall time or input (F1's determinism rule, and F1 evaluates its rules inside `Simulation::update`, never in `extract`, F1 AI-45), that F1's sprites scale with `ui_scale` including the minimum badge size (F1 AI-61, T-29), that F1 adds the `decorations` hide element and the `agent-key` option, both of which appear in the Show list (REC-11), and that F1's agent colours stay distinguishable under common colour-vision deficiencies, which F1 checks on a golden frame because it cannot be fixed after export.
- F2 (private repositories): a recording works from the in-memory log, so it needs no token and no network. F2 supplies one flag (`source.private`, F2 PR-93, derived from `repo.visibility`) that drives REC-103 and the recipe. Session loss ends a running recording (REC-20, F2 PR-121). Server-side rendering (M3) is the one place that needs F2's credentials, through the ticket of F2 PR-110.

## 6. Security & privacy

| id | Requirement | M |
|---|---|---|
| REC-100 | M1 uploads nothing: logs, frames and files stay in the browser; the dialog and the finished screen say so. No new network request is made by recording. | M1 |
| REC-101 | File System Access is used only through `showSaveFilePicker` inside a click handler; no directory handle, no persistent permission, no stored handle (nothing in `localStorage`, IndexedDB or the URL). | M1 |
| REC-102 | The recipe contains only: source identity (`host`, `path`, branch, dates, subdirectory, head sha, `private` flag), option values, plan numbers, build identity and the time zone name. It never contains tokens, cookies, user emails or file contents. The embedded copy omits the time zone name and the creation time (they stay in the sidecar) and the values listed in REC-71. | M1 |
| REC-103 | For private sources (F2) and sources of unknown provenance (a local file, `?log=`: a log may come from a private repository) `record-metadata` defaults to `none`, the title defaults to empty (REC-11), the file name stem defaults to `gource` (REC-62), and the sidecar redacts `path` and `head` unless confirmed (REC-72); in M1 there is no embedded copy (REC-71 is M2), so `record-metadata=none` governs the embedded copy from M2 and the other effects apply from M1; the "Copy reproduce link" button states that only viewers with access can open it. The video itself shows contributor names and file paths as the live view does; for private sources the dialog warns once: "The video shows names and paths from a private repository." | M1 (hook), F2 provides the flag |
| REC-104 | Contributor emails are never on the page (D6); recordings therefore cannot leak them. F1's agent labels in a recording are the same generic text and shapes as in the live view. | M1 |
| REC-105 | A recording cannot be started by a link (REC-3): the picker and the click are required, so a crafted `?record=1` URL cannot write files or run an hours-long job. `record-max-seconds` and the 4 hour hard cap bound a hostile `loop` recipe in the URL, and the blocking confirmation of REC-124 (M2, together with `record=1`) lists URL-supplied values and gates large runs. Recipe values are validated by `RecordOptions` like any other option. The URL cannot enable test hooks (REC-95). | M1 |
| REC-106 | The recorder's GPU and memory use is bounded by REC-44 (size limits), the encoder queue limit (REC-83), and the Blob cap (REC-63); a hostile URL with a 4096x2304 `record-viewport` costs one large canvas, not unbounded memory; its disk and CPU cost (up to about 180 GB and hours at 100 Mbps and 4 hours) is bounded by the confirmation of REC-124 and tested in T-28. | M1 |
| REC-107 | The embedded recipe is escaped as UTF-8 bytes in a binary box; reading tools show it as text, and nothing in the page ever renders a recipe from a file as HTML. | M1 |
| REC-108 | M3: server rendering (prerequisite: F2 M2a and PR-110) follows section 5.14: private renders are keyed by F2's namespace, never served from the public cache, and readable only by their owner (session, CSRF, owner match, `no-store`); recipes are validated server side (repository sources on allowed hosts only, egress allowlist); quotas apply to signed-in accounts and anonymous renders have a global cap; render files are not publicly listed. Tests: cross-account `GET` and same-recipe submit from another account both fail; an `?log=` recipe is rejected. | M3 |

## 7. Performance budget

Inputs from `docs/web-mvp/PERFORMANCE.md` (RTX 3080 Ti, Chrome 154, 1280x720, ANGLE GL): browser CPU per frame 1.73 ms on git/git (sim 1.02, build 0.46, submit 0.25) and 14.92 ms at 100k files (sim 8.84, build 2.78, submit 3.29), plus 5.5 ms of GPU-process work after submit at 100k files; currently 10 draw calls (12 with F1 decorations); upload 3.2 MiB per frame (git/git) and 19.6 MiB (100k files). Not measured in the repo, and therefore the M1 spike's job to measure: `VideoFrame` creation, `readPixels`, and encode throughput at 1080p and 4K.

Per output frame the CPU cost is `ticks_per_frame * (sim + extract + build) + 1 * submit` (every intermediate tick builds, `engine.rs:333-340`): 30 fps costs about twice the CPU per output frame of 60 fps.

| id | Budget | Applies at |
|---|---|---|
| REC-110 | Throughput, git/git, 1080p30, hardware H.264, on the reference machine of `docs/web-mvp/PERFORMANCE.md` (Intel Core i7-11700KF, 8 cores / 16 threads, 32 GB RAM, NVIDIA GeForce RTX 3080 Ti, Google Chrome 154.0.8037.57): at least 60 output frames per second (2x real time of the video). | M1 |
| REC-111 | Throughput, 100k-file stress, 1080p30, hardware H.264: at least 10 output frames per second. | M1 |
| REC-112 | Throughput, git/git, 1080p30, software H.264: recorded in the M1 report (and in PERFORMANCE.md), not an exit criterion. | M1 |
| REC-113 | Heap: JS heap growth during a 10 minute 1080p30 recording at most 64 MiB (encoder queue at most 6 frames, 2 pending writes, one fragment buffer, reused frame buffers). Wasm memory growth for the fork at most the live engine's size at 100k files (about +133 MiB). No per-frame allocation in the recorder loop beyond the `VideoFrame` and the encoded chunk. | M1 |
| REC-114 | The fork, dialog open and preview must not delay the live view by more than one 50 ms task; the dialog opens in under 300 ms on git/git (fork cost is a log copy). The preview (about 300 ticks, about 0.2 s on git/git and about 2.4 s at 100k files at `PERFORMANCE.md:137-141` costs) therefore runs sliced (`preview_for`, 8 ms slices), never as one synchronous call. | M1 |
| REC-115 | Dry run: at most 60 % of the real run's CPU (sim plus extract against sim plus extract plus build plus encode) and sliced to 8 ms main-thread tasks. | M2 |
| REC-116 | Draw calls per frame in the recorder equal the viewer's: 10 today, at most 12 with F1 decorations (F1 AI-53). Recording adds no render pass, overlay or HUD quad; the recorder never exceeds the budget of `docs/web-mvp/PLAN.md` ("about a dozen"). `stats_json.draw_calls` is 10 or 12. | M1 |
| REC-117 | Muxer: at most 1 ms CPU per frame at 4K and at most 16 bytes of heap per frame retained. | M1 |
| REC-119 | Download size: the budgets are cumulative against `gource_web_bg.wasm` at HEAD 8a3ecec (1,558,409 bytes raw): F1 +150 KiB gzip, F3 +60 KiB gzip (the recorder, planner, muxer and recipe code), together at most +210 KiB gzip; each milestone measures against the artifact of the previous merged milestone, in CI, and records the number in `docs/web-mvp/PERFORMANCE.md`; if the budget is exceeded the recorder moves to a separate wasm module loaded on first Record. | M1 |
| REC-118 | Memory release: `GourceRecorder.free()` and `canvas.width = canvas.height = 0` plus `WEBGL_lose_context.loseContext()` run on every exit path, so a second recording after a 4K one does not exhaust GPU memory (T-15). | M1 |

## 8. Testing & acceptance

Test layers: Rust unit and property tests (native), golden and hash tests with `gource-cli`, TypeScript unit tests (`pnpm -C web test:unit`, Node, no browser), and Playwright in system Chrome (`pnpm -C web test:e2e`, mock engine with `?mock=1`; real wasm with `E2E_REAL=1`; `?fixed-dt` for determinism). WebCodecs needs a real browser, so tests that encode run in the real-engine suite only; the mock engine has a Canvas2D renderer (`web/src/engine/mock.ts`) and, through the extended `EngineFactory` (section 5.12), a mock recorder and muxer that let the dialog, progress, cancel and sink code run in the mock suite with a fake encoder (REC-93). Determinism tests compare pre-encode `frame_hash` values, never decoded frames, because hardware encoders and rate control are not bit-reproducible between runs; decoded frames are compared only with tolerances (T-2, T-3).

| id | Requirement | M |
|---|---|---|
| REC-90 | Every REC requirement in sections 4 to 7 appears in the REC column of at least one test row below, or is labelled `manual` with its checklist recorded in the PR. Failure paths are covered by injection (T-28): a sink that throws `QuotaExceededError`, a stub encoder error, a forced `loseContext()`, a Blob cap of 1 MiB, a reordering stub encoder, an oversized recipe, a hostile URL. | M1 |
| REC-91 | New tests follow the repository's commands (CLAUDE.md): `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, the wasm clippy line, `pnpm -C web typecheck`, `pnpm -C web test:unit`, `pnpm -C web test:e2e`. | M1 |
| REC-92 | `E2E_REAL=1` recording tests use `serve.sh` (one origin), the 6 small `tests/logs/*.log` scenarios and a generated 2,000-commit log as inputs, and write files to Playwright's temp dir through a test hook that replaces `showSaveFilePicker` with an in-memory sink (the picker needs a real user gesture and a native dialog). | M1 |
| REC-93 | Mock mode: `?mock=1&mock-record=1` (test builds only, REC-95; `mock-record` also works with `?mock=full`, F2 PR-95) enables Record with the mock factory's recorder, a fake encoder, the byte-counting muxer and the in-memory sink, so the dialog, progress, cancel, error and finished screens are tested without WebCodecs or wasm. `EngineFactory.createRecorder/createPlanner/createMuxer` have mock implementations (section 5.12). | M1 |

| Test | Layer | What it checks | REC |
|---|---|---|---|
| T-1 | e2e (mock) | Record disabled while the log is incomplete, tooltip shows percent; enabled at `finish_log`; disabled with no `VideoEncoder`. | 4, US-9 |
| T-2 | e2e (real) | Record 120 frames at 640x360, 30 fps, speed 2 s/day on a fixture. Decode with `VideoDecoder` (or `<video>` + `requestVideoFrameCallback`). For frames 0, 30, 60, 119, compare with frames from a second fork stepped with `step(2)` and captured as PNG: mean absolute error at most 2/255, p99 at most 12/255 (encoder loss; thresholds [I], the one place decoded frames are compared; the frame hashes of the two forks are also equal, REC-36). | 31-36, US-1 |
| T-3 | e2e (real) | Colour test (REC-48): flat patches and a Gource frame through the pipeline; per-patch error at most 4/255. Records the chosen capture path. | 47, 48 |
| T-4 | native | Driver stop conditions: tail frames count, `app_finished` early exit, `-1` tail, max-seconds cap, loop with cap, hard cap, on `FakeBackend` and fixture logs. Last frame within `tail` of the last commit. | 32-35, US-6 |
| T-5 | e2e (real) | Two runs of one recording, one with scripted pointer, key and wheel events on the live canvas and one without: the pre-encode `frame_hash` of every frame is equal (decoded frames are not compared for equality); live viewer pause restored afterwards. | 20, 35, 40, US-4 |
| T-6 | native | Camera at 1920x1080 vs 1080x1920 differs; at 1280x720 vs 2560x1440 equal (REC-39); `font_scale` is 1 for all presets including 1080x1920 (height 1920); `ui_scale` auto values (`min(w, h) / 1080`: 1.0, 1.0, 2.0 for 4K); in the frame batches the label font size relative to the short side at 1080x1920 equals that at 1920x1080 within 1 %. | 37-39, US-7 |
| T-7 | e2e (real) | 1920x1080 accepted and decodes to 1920x1080; 1080x1080, 1080x1920, 3840x2160 (skipped when no encoder); output timestamps monotonic (no reorder); `avcC` present; `EncodedVideoChunkMetadata.decoderConfig.description` copied; whether `prefer-hardware` and `prefer-software` are strict is logged (REC-13). A non-blocking CI job repeats T-7 and T-10 on Firefox and Safari where `VideoEncoder` exists. | 13, 51, 53 |
| T-8 | native | Level and bitrate vectors of section 5.6 (including 1280x720 at 60 is 3.2, 1080x1080 at 30 is 3.2, 1920x1080 at 30 with 30 Mbps is 4.1, 4096x2304 at 30 is 5.2) and the properties of REC-50; size validation table of REC-44 (4096x2304 at 60 rejected); Match window vectors of REC-6 (1457x817 gives 1456x816, 5120x2880 gives 4096x2304 with `ui_scale` x 0.8, 1251x703 gives 1250x702). | 6, 44, 50 |
| T-9a | native (M1, M2) | M1: `upper_s >= exact` on every fixture, also at `time-scale` 0.5 and tail -1; the progress table is monotone and ends at 1 for ranged runs (REC-17); `record_empty_run` for `empty.log`; `single-event.log` produces one commit and the tail (REC-122). M2: `about_s` within 0.5x to 2x of the exact duration (REC-69), an excluding date range and a filter that excludes everything through the preview. | 17, 69, 122, US-2 |
| T-9b | native (M2) | Dry run equals driver frame count (REC-68). | 67, 68, US-2 |
| T-10 | e2e (real) + native | Ladder order (REC-52) with a stubbed `isConfigSupported` that rejects hardware, then 4K, then H.264: expected candidate and dialog note each time. | 13, 52, US-8 |
| T-11 | native | Option round trip (REC-30); TypeScript and Rust option tables agree; URL round trip for `record=1` and record options; the Gource option tests remain unchanged. | 30, 3 |
| T-12 | e2e (real, slow) | 10 minute 1080p30 recording of a generated log into the in-memory sink replacement that discards bytes after hashing: JS heap growth and wasm growth within REC-113 (`performance.memory` with `--enable-precise-memory-info`, `status.memory_bytes`). | 113, US-3 |
| T-13 | e2e (real) | Hidden tab: start a recording, open and focus a second page (`page.bringToFront`), after 20 s the frame counter in the first has advanced at least 25 % of the foreground rate (REC-83); from M2 the wake lock is requested while `running` and released on every exit path (REC-123). A slow variant (nightly) stays hidden for 6 minutes and checks the recording finishes; its result is the documented maximum hidden duration. Same for the M2 worker. | 83, 84, 123, US-12 |
| T-14 | e2e + native | Cancel: "Stop and save" yields a file whose walked duration equals frames/fps and plays; "Discard" aborts the writable (spy) and leaves no Blob; partial fragmented file truncated mid-fragment still walks (REC-61 d). | 21, 22, US-5 |
| T-15 | e2e (real) | Two consecutive recordings (4K then 1080p) succeed; `loseContext` called; second recording's first frame equals a fresh page's. | 118 |
| T-16a | e2e (real) (M1) | Reproduce link and sidecar: open the link, press Record with the same options, the first 30 frames' pre-encode `frame_hash` values equal (same build, same log; a changed head shows the banner, REC-73); the sidecar parses and redacts per REC-72. A recipe made with `user-style`, `ai-filter`, `agent-key`, `decor-motion` and `hide=decorations` set (needs F1 M2) lists exactly those options and reproduces equal frame hashes. | 70, 72, 73, US-10 |
| T-16b | e2e (real) (M2) | Embedded recipe: parse the `----` item from the file bytes; it has no frame count or encoder, none of the values of `NAME_BEARING_OPTIONS` (REC-70, REC-71), and for private and unknown-provenance sources no `path`, `branch`, `subdir` or `head`. | 70, 71, 102, 107 |
| T-17 | e2e (mock) | Run with `?mock=full&mock-auth=in&mock-record=1&repo=github.com/private-co/secret` (F2 PR-95), which yields `repo.visibility=private`, and with a local-file and a `?log=` source: title empty, file name stem `gource`, warning shown, sidecar has `path` and `head` redacted until confirmed; with a public repository: repository title and stem. M2 adds: `record-metadata` defaults to `none` for these sources and `embed` for public ones, and the embedded copy omits `path`, `branch`, `subdir` and `head` for private sources even with `embed`. Covers F2 PR-108 and REC-103. | 11, 62, 72, 103, US-11 |
| T-18 | native | Muxer tests of REC-61 (a) to (d); optional `ffprobe` test (e) in CI image where available. | 55-61 |
| T-19 | e2e (real) | File plays: `<video>` loads the output, `videoWidth/Height`, `duration` within 1 frame of the plan, `currentTime = duration/2` seeks and `readyState >= 2`. | 58, G3 |
| T-20 | native | Parity guard: `cargo test --workspace` golden and hash tests unchanged (REC-82). | 82 |
| T-21 | native | HUD in recorder frames: `SceneView.hud.textbox` empty, `splash` zero, no hover over 1,000 ticks (REC-40). | 40 |
| T-22 | native | Glyph budget policy (the pure `budget.rs` of section 5.12, shared by the real atlas): with `frame_glyph_budget = u32::MAX` on 3,000 distinct names no glyph is deferred, and the eviction counter reaches the result through `Backend::atlas_evictions` on `FakeBackend` (REC-42). A `wasm-bindgen-test` or e2e case checks the real atlas draws a 3,000-name frame in one frame. | 42, 81 |
| T-23 | native (M2, gated on `target/reference/`) | `FramePlan.ticks_for_frame` cumulative counts equal the tick lists in the C++ reference `manifest.json` (`scripts/reference/render-all.sh`) at 25, 30 and 60 fps; with `rand-mode=glibc` the recorder's frames at those ticks are within the PARITY.md thresholds. | 32 |
| T-24 | e2e (mock) | `Alt+R` opens the dialog and the engine receives no key; Esc closes it when configuring and opens the cancel choice when running; focus trap; progress bar attributes; live region text. | 2, 94 |
| T-25 | e2e (mock) | Coarse-pointer narrow viewport: Record hidden, share still works; `localStorage` throwing leaves the dialog working with defaults (M2). | 86, 88 |
| T-26 | native | Snapshot exactness (REC-31) on three fixtures; (M2:) the enumeration of key-mutated state (every field `sim/input.rs` can change is in `Settings` or on the not-recorded list); `tree_tweaked` is true after `[` and false after a restart (REC-125). | 31, 125 |
| T-27 | native + e2e (real) (the preview part is M2) | Preview then recording equals a fresh recorder: after `preview_for` runs to completion and `begin_recording()`, the view hashes of the first 30 frames equal those of a recorder that never previewed; `step_frame()` before `begin_recording()` fails with `record_not_started`; a slice of `preview_for` never exceeds its `ms` budget by more than one tick on the 2,000-commit log (REC-114, REC-120). | 89, 114, 120 |
| T-28 | native + e2e (mock) | Failure injection: a sink throwing `QuotaExceededError` gives `record_disk_full` and an attempted `close()`; another rejection gives `record_sink_failed`; a stub encoder error gives `record_encoder_failed` with "Stop and save"; a reordering stub encoder gives `record_mux_reorder` and one `latencyMode: 'realtime'` retry; `loseContext()` gives `record_gl_failed`; a 1 MiB Blob cap stops with the in-memory notice; a recipe over 16 KiB falls back to the link only; (M2:) a hostile URL (`record-viewport=4096x2304`, 60 fps, `record-bitrate=100`, loop, `record-max-seconds=14400`) shows the blocking confirmation and does not start; the banner and `beforeunload` are present while `running`. | 20, 46, 53, 63, 65, 71, 105, 106, 124 |
| T-29 | native + e2e (real), depends on F1 M1b (built-in look, `hide=decorations`) and F1 M2 (`user-style`, `agent-key`) | The F1 rule `ai:claude => cape=agent` (or the built-in look when no rule is given) changes the recorded frame in the styled region against the same run without it, at 1080p and at 4K, and the sprite scales with `ui_scale`, including the far-zoom case where the badge sits at its minimum size (equal badge-to-avatar ratio at `ui_scale` 1 and 2, F1 AI-61); the `decorations` hide element and `agent-key` appear in the "Show" list. | 11, 38, US-14 |
| T-30 | e2e (real) + native | While `running`, editing a Restart-class option or dragging the seek slider is disabled, and a programmatic `set_option` restart leaves the live engine paused (the viewer re-asserts the pause); the banner is shown. | 20, US-4 |
| T-31 | e2e (mock) | The dialog shows "Camera: automatic. Your zoom and rotation are not recorded." after the live view was zoomed and rotated. | 45, US-16 |
| T-S1 | spike (M0, before M1) | Detached vs attached canvas capture, `VideoFrame(canvas)` in the same task, hidden tab, `VideoFrame` cost at 1080p/4K, readPixels cost. Also: colour thresholds (REC-48), hardware and software strictness (REC-13), whether encoders reorder frames (REC-53), File System Access staging (REC-64), and playability of a synthetic fragmented MP4 in the player checklist (section 9), which decides the layout. Results recorded here before approval (REC-43, REC-48). | 43, 48, 58, 64 |
| T-S2 | spike (M2) | Checkpoint reopen cost with `keepExistingData` at 100 MB, 1 GB (REC-66). | 66 |

Acceptance of M1 as a whole is in section 9.

## 9. Milestones

The cross-feature ordering block at the end of this section is identical in F1, F2, F3 and `docs/features/README.md`.

### M0: spike (before M1; its results are written into this document before it is approved)

Numbered tasks, each with a pass or fail output that replaces a former `[I]` marker:

| Task | Output written into this spec |
|---|---|
| 1. T-S1 capture: detached vs attached canvas, `VideoFrame(canvas)` in the same task, `readPixels` cost, `VideoFrame` cost at 1080p and 4K | the capture path (REC-43, REC-48) and the costs |
| 2. Colour test on the Gource test card | the thresholds of REC-48 (measured plus 25 %) |
| 3. Encoder behaviour: `prefer-hardware` and `prefer-software` strictness, frame reordering, `isConfigSupported` for 1920x1080 | REC-13, REC-53, REC-51 decisions |
| 4. Hidden tab on the main-thread loop | whether REC-83's notice is needed (25 % rule of T-13) |
| 5. File System Access staging and `close()` behaviour | REC-64 consequences confirmed |
| 6. Player-compatibility check of a synthetic fragmented MP4 (Chrome, VLC, QuickTime, Windows Films and TV, an upload target such as YouTube) | the MP4 layout decision: fragmented stays, or the flat layout moves into M1 now |
| 7. Software-encode throughput at 1080p30 | recorded in the M1 report (REC-112), not gated |

Reference machine for every throughput statement: the one of `docs/web-mvp/PERFORMANCE.md` (Intel Core i7-11700KF, 8 cores / 16 threads, 32 GB RAM, NVIDIA GeForce RTX 3080 Ti, Google Chrome 154.0.8037.57).

### M0 spike results (2026-10-02)

Measured with the spike page `web/spike/` (not shipped; `node web/spike/run.mjs` reruns everything, `--quick` for a smoke run, `--only=<groups>`, `--repeat=N`; raw JSON lands in the git-ignored `web/spike/results/`). The page renders a WebGL2 scene shaped like a Gource frame on a canvas with the backend's context attributes (`alpha`, `antialias`, `depth`, `stencil`, `premultipliedAlpha` and `preserveDrawingBuffer` all false, `powerPreference: 'high-performance'`; `backend.rs:147-154`). The scene has a `1A1A1A` clear, about 5,000 textured file discs with shadows, 220 thin edges, 150 beams, 30 users, 220 additive procedural glow quads, 150 labels from a canvas-rasterised atlas, and a strip of 20 flat test patches. The patches are the background, white, the action colours, saturated blue, dark green, mid grey and the `colourHash` colours of 12 extensions. Vertex data is rebuilt and uploaded every frame (about 0.9 MB), and a frame is 8 draw calls. The loop is a plain async loop with no `requestAnimationFrame`, and the timestamps are `round(i*1e6/fps)`. Path A takes `new VideoFrame(canvas, {timestamp, duration})` straight after the draw, in the same task. Path B does `readPixels`, a CPU row flip, then `new VideoFrame(rgba, {format: 'RGBA', colorSpace})`. Each frame is passed to `encode()` and closed. One `dequeue` is awaited when `encodeQueueSize >= 4`. Keyframes are forced every 2 s, and there is a single `flush()`. Colour is checked by decoding every chunk with `VideoDecoder`, drawing the decoded frame into a 2D canvas (`drawImage`, as `<video>` + `drawImage` does), and comparing with `readPixels` of the same source frame. The checks run on frames 0 (key), 61 and 119 of 120.

**Machine.** Reference machine of `docs/web-mvp/PERFORMANCE.md`:

- Intel Core i7-11700KF (16 threads), 32 GB, NVIDIA GeForce RTX 3080 Ti (driver 595.71.05).
- Ubuntu 24.04.5, kernel 7.0.0-34.
- Google Chrome 154.0.8037.57 headless (`--headless=new` via Playwright 1.63), served from `http://127.0.0.1` (secure context).

Two flag sets were used:

- **GL**: `--use-angle=gl --ignore-gpu-blocklist --enable-gpu`, renderer "ANGLE (NVIDIA ..., OpenGL 4.5.0)". Vulkan ANGLE loses contexts headless on this machine.
- **default**: no flags. Headless Chrome then falls back to "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)))", a CPU rasteriser, which is what CI and `playwright.config.ts` get.

**Encoder.** Chrome's software H.264 encoder. `prefer-hardware` is rejected by `isConfigSupported` at every size, profile and latency mode tried, under both flag sets. Chrome on Linux has no NVENC path, and there is no NVIDIA VA-API encode driver. The stream really is High profile: `avcC` `profile_idc` 100, with the `level_idc` of the codec string.

#### Throughput (task 1, task 7, REC-110, REC-112)

End-to-end output frames per second, from the first draw to `flush()` resolving. Path A, `no-preference`, `quality`, 600 frames (240 at 4K). The GL figures are the median of three runs. Run 1 had three outliers, all slower: 720p30 at 162, and 4K60 `prefer-software` at 19 and 4K60 `realtime` at 24. Runs 2 and 3 were within 7 % of each other.

| Size | fps | GL: frames/s (x real time) | default (SwiftShader): frames/s (x real time) |
|---|---|---|---|
| 1280x720 | 60 | 260 (4.3x) | 71 (1.2x) |
| 1280x720 | 30 | 261 (8.7x) | 71 (2.4x) |
| 1920x1080 | 60 | 123 (2.0x) | 39 (0.66x) |
| 1920x1080 | 30 | 124 (4.1x) | 39 (1.3x) |
| 3840x2160 | 60 | 33.2 (0.55x) | 10.6 (0.18x) |
| 3840x2160 | 30 | 33.3 (1.1x) | 11.0 (0.37x) |

- `prefer-software` and `no-preference` gave the same numbers; they select the same encoder here. `latencyMode` `realtime` versus `quality` also made no difference: 1080p60 gave 123 and 125 (run 2) and 130 and 130 (run 3); 4K60 gave 33.8 and 33.8.
- **The encoder is the bottleneck under GL.** Main-thread cost per frame is about 1.1 to 1.5 ms of scene build and draw, 0.08 to 0.14 ms for the path A capture and 0.01 ms for `encode()`. The rest is the wait on `dequeue`: about 2.5 ms at 720p, 7 to 8 ms at 1080p and 30 ms at 4K. That makes the software encoder's cost about 3.7, 8 and 30 ms per frame.
- **Output fps does not change the cost per output frame.** It moves only the real-time factor. In the product, 30 fps costs two simulation ticks per output frame (section 7).
- **Under SwiftShader the capture is the bottleneck.** `VideoFrame(canvas)` costs 12.9 ms at 720p, 24 ms at 1080p and 90 ms at 4K, which is a CPU readback.
- REC-112 (software 1080p30) is recorded as 124 frames/s on this scene.
- REC-110 asks for hardware H.264, which this machine cannot provide in Chrome on Linux (see changes below). With software H.264 the spike scene runs at 2x the 60 frames/s target, before the engine's own per-frame cost: 1.7 ms CPU per tick on git/git, so about 3.2 ms per 30 fps output frame. That cost runs on the main thread while the encoder works on its own threads.

**Capture cost** (task 1). These are baseline runs: draw plus capture, no encoder. "Draw" also forces a 1-pixel `readPixels` sync, so the GPU work is counted.

| Size | GL: path A `VideoFrame(canvas)` ms | GL: path B `readPixels`+flip+`VideoFrame` ms | default: path A ms | default: path B ms |
|---|---|---|---|---|
| 1280x720 | 0.12 | 2.2 | 11.4 | 9.2 |
| 1920x1080 | 0.18 | 4.1 (6.0 while encoding) | 20.7 | 16.3 |
| 3840x2160 | 0.12 | 14.4 (21.2 while encoding) | 82.1 | 60.6 |

Path B's readback overlaps the encoder. With encoding, path B ran at 109 frames/s at 1080p60 and 32 at 4K60, within the run-to-run spread of path A. At 720p60 it was slower: 170 against 260.

**Detached canvas** (REC-43). `getContext('webgl2')` and `new VideoFrame(canvas)` both work on a canvas that is never attached to the document. Throughput is the same as on an attached canvas: 112 against 117 frames/s under GL, 39.6 against 36.6 under default. Keep the canvas detached; the off-screen-attach fallback is not needed.

**Hidden tab** (task 4). Not settled. A second page brought to the front in headless Chrome leaves the recording page `visible` (0 hidden frames). Throughput was unchanged under GL (105 and 103 frames/s in two runs). Under SwiftShader one run gave 21 and a rerun 39.5, the same as a visible page. The REC-83 notice decision needs a headed run in M1 (T-13).

**Encoder queue depth.** Awaiting one `dequeue` at `encodeQueueSize >= 1, 2, 4, 8, 16` gave 109, 110, 121, 114 and 103 frames/s under GL, and 35, 38, 39.5, 39 and 39 under default. 4 is the best value or as good as any; keep it.

#### Colour (task 2, REC-47, REC-48)

All figures are per channel on 0..255. "Patch error" is the mean absolute error over the inner region of each flat patch, with a margin of a sixth of the patch and at least 8 px, taking the worst patch and channel. "Worst pixel" is the largest single-pixel error inside a patch. "Frame MAE" is the mean absolute error over R, G and B of the whole frame. "4:2:0 bound" is the error of an ideal 8-bit BT.709 4:2:0 round trip of the same source, the floor for any encoder. GL and default gave the same numbers to ±0.2.

| Capture | Size | Scene | Patch error | Worst pixel | Frame MAE | 4:2:0 bound | Out Mbit/s (target) | Stream colour (SPS VUI range/primaries/transfer/matrix) |
|---|---|---|---|---|---|---|---|---|
| A | 1280x720@60 | moving | 4.00 | 4 | 16.20 | 4.04 | 14.9 (7.5) | full/1/13/1 |
| A | 1920x1080@60 | moving | 4.00 | 4 | 13.21 | 3.01 | 28.8 (12) | full/1/13/1 |
| A | 1920x1080@30 | moving | 4.00 | 5 | 13.34 | 3.01 | 15.6 (8) | full/1/13/1 |
| A | 3840x2160@60 | moving | 4.00 | 4 | 8.87 | 1.65 | 81.4 (60) | full/1/13/1 |
| A | 1280x720@60 | still | 4.00 | 4 | 6.86 | 3.93 | 1.5 (7.5) | full/1/13/1 |
| A | 1920x1080@60 | still | 4.00 | 4 | 6.18 | 2.92 | 2.7 (12) | full/1/13/1 |
| A | 3840x2160@60 | still | 4.00 | 4 | 3.75 | 1.61 | 6.0 (60) | full/1/13/1 |
| B, tagged BT.709 RGB | all three sizes, moving and still | | same as A to 0.01 | same | same | | same | full/1/1/1 |
| B, tagged sRGB RGB | all three sizes, moving | | same as A | same | same | | same | full/1/13/1 |
| B, tagged BT.709 limited Y'CbCr | 720p / 1080p / 4K, moving | | 16.00 / 14.00 / 16.00 | 16 / 14 / 16 | 17.74 / 14.52 / 9.81 | | 11.8 / 23.5 / 69.2 | limited/1/1/1 |
| A, 1080p60, bitrate 30 / 60 Mbit/s | moving | | 3.94 / 3.94 | 4 / 4 | 13.10 / 10.19 | 3.01 | 30.0 / 59.8 | full/1/13/1 |

- **Capture is lossless.** `new VideoFrame(canvas)` drawn straight back into a 2D canvas, with no encode, matches the WebGL pixels exactly (error 0) on every run and size. The darker greens of https://github.com/w3c/webcodecs/issues/598 do not reproduce here.
- **Path A and path B decode to identical pixels.** Only the tag differs: path A frames carry transfer sRGB (13), path B frames carry the transfer of the tag. The encoder converts to **full-range** BT.709 Y'CbCr in every case unless asked otherwise, and writes that into the SPS VUI and `decoderConfig.colorSpace`.
- **Asking for limited range is worse.** A path B frame tagged BT.709 limited range does produce a limited-range stream, but saturated colours come back off by 14 to 16: saturated blue `0000FF` decodes as `0100EF`, and create green `00FF00` gains 10 in blue.
- **Patch errors are at most 4 in every configuration.** The largest are on keyframes, on the `.json`, `.html` and modify-orange colours. Later P-frames of a still card refine them to 1 or less.
- **Frame MAE is set by the encoder, not by the capture.** It is 1.7 to 2.3 times the 4:2:0 bound on a still card, and 4 to 5.4 times on this deliberately busy moving scene at the `standard` bitrate.

**Chosen capture path: A** (REC-48). It passes the colour test with the same errors as B, costs 0.1 to 0.2 ms per frame where B costs 4 to 21 ms, and needs no readback buffer.

- Path B tagged BT.709 RGB is the fallback if the player checklist (task 6) shows that a player mishandles transfer 13. It gives the same pixels with the conventional 1/1/1 tag, at no end-to-end cost at 1080p and above on this machine.
- The tagged-sRGB experiment of REC-48 is unnecessary.

**REC-48 thresholds (final; measured plus 25 %).** For T-3 on the Gource test card, decoded through `drawImage`:

| Check | Measured (worst over sizes, paths A/B, both flag sets) | Threshold |
|---|---|---|
| Capture only: `VideoFrame(canvas)` drawn back, any pixel (cheap check that would catch issue 598) | 0 | 0 (exact) |
| Flat patch, mean absolute error per channel, inner region | 4.00 | **5.0** |
| Flat patch, worst single pixel per channel, inner region | 5 | **6** |
| Whole frame MAE, still card, 1280x720 | 6.86 | **8.6** |
| Whole frame MAE, still card, 1920x1080 | 6.19 | **7.7** |
| Whole frame MAE, still card, 3840x2160 | 3.76 | **4.7** |
| Whole frame MAE for any other still card, as a multiple of its own 4:2:0 bound | 2.32 x | **2.9 x** |

The T-3 row in section 8 says "per-patch error at most 4/255"; the measured worst is exactly 4, so the threshold becomes 5.0 as above. Moving content gets no frame threshold, because it depends on the scene and the bitrate.

#### Encoder behaviour (task 3, REC-13, REC-51, REC-52, REC-53)

- **`isConfigSupported`.** Results were identical under both flag sets.
  - Every codec string of the level table in section 5.6 was accepted, in High, Main and Baseline, with `no-preference` and `prefer-software`, and with both latency modes: 1280x720, 1920x1080, 2560x1440, 3840x2160, 1080x1080 and 1080x1920 at 30 and 60, plus 4096x2304 at 30 (`avc1.640034`).
  - `prefer-hardware` was rejected for all of them, and also for VP9 (`vp09.00.40.08`, which is otherwise accepted at 1080p and 4K).
  - So `prefer-hardware` is **strict** in `isConfigSupported` on this machine. `prefer-software` is accepted wherever `no-preference` is. Profile and latency never change the answer.
- **REC-51.** 1920x1080 is accepted as is. The SPS carries a crop of 8 rows (1088 to 1080), and decoded frames report `displayWidth`/`displayHeight` 1920x1080. The decoder's `codedHeight` is 1090 at 1080p, 738 at 720p and 2178 at 4K (padding), so T-7 must check the display size or `visibleRect`, not `codedHeight`.
- **B-frames (REC-53).**
  - No out-of-order output in any run: more than 60 encoder runs per flag set, both latency modes, every size and fps, both flag sets.
  - The SPS has `pic_order_cnt_type` 2, `max_num_reorder_frames` 0 and one reference frame, so this encoder cannot emit B-frames. M1 can write `dts == pts` with no `ctts`.
  - Keep `record_mux_reorder` as a guard for other platforms' encoders. The `realtime` retry never triggers here.
- **Keyframe cadence.** Without forced keyframes, the encoder emitted one keyframe (frame 0) in 900 frames, in both latency modes. Forced keyframes arrived exactly on schedule: gaps of 120 frames at 60 fps and 60 at 30 fps. Forcing every `round(record-keyframe-seconds * fps)` frames is therefore required, as specified.
- **Bitrate.** The software encoder does not go below a quality floor.
  - On the dense moving scene the output exceeded the `standard` target 1.4 to 2.4 times:
    - 720p60: 14.9 against 7.5 Mbit/s;
    - 1080p30: 15.6 against 8;
    - 1080p60: 27.5 to 28.8 against 12;
    - 4K60: 78 to 81 against 60.
  - `bitrateMode: 'constant'` behaved the same: 27.5 Mbit/s at 1080p60.
  - Targets above the floor are met: 30.0 for 30 and 59.8 for 60.
  - A still card used 1.5 to 6 Mbit/s whatever the target.

#### Recommended default configuration and ladder

| Size @ fps | Codec | `standard` bitrate |
|---|---|---|
| 1280x720 @ 30 / 60 | `avc1.64001f` / `avc1.640020` | 5 / 7.5 Mbit/s |
| 1920x1080 @ 30 / 60 | `avc1.640028` / `avc1.64002a` | 8 / 12 Mbit/s |
| 2560x1440 @ 30 / 60 | `avc1.640032` / `avc1.640033` | 16 / 24 Mbit/s |
| 3840x2160 @ 30 / 60 | `avc1.640033` / `avc1.640034` | 40 / 60 Mbit/s |
| 1080x1080 @ 30 / 60 | `avc1.640020` / `avc1.64002a` | by the formula of section 5.6 |
| 1080x1920 @ 30 / 60 | `avc1.640028` / `avc1.64002a` | by the formula of section 5.6 |

The rest of the encoder configuration:

- `hardwareAcceleration: 'no-preference'` by default.
- `latencyMode: 'quality'`. `realtime` is neither faster nor different here.
- `bitrateMode: 'variable'`.
- `avc: {format: 'avc'}`.
- Keyframes forced every 2 s.
- Await one `dequeue` at `encodeQueueSize >= 4`.
- Capture with path A.

**Ladder order.** Keep the REC-52 order:

1. High at the requested size with the requested preference.
2. The other preferences.
3. Main.
4. 75 % and 50 % of the size.

On this machine step 1 always succeeds with `no-preference`. A user who asks for `prefer-hardware` falls to step 2 (`prefer-software`), with the REC-13 note "H.264 hardware encoding is not available at <size> here; using software". Steps 3 and 4 are never reached, because `isConfigSupported` does not discriminate by profile or size up to 4096x2304@30; T-10's stubbed ladder test remains their only coverage.

#### What this changes in the design

1. **REC-47 (colour tag).**
   - Do not force limited range. Chrome's encoder produces full-range BT.709 Y'CbCr, and asking for limited range through the frame tag costs up to 16/255 on saturated colours.
   - The muxer writes `colr` `nclx` from `decoderConfig.colorSpace` (path A: primaries 1, transfer 13, matrix 1, full range), so the container agrees with the SPS VUI. It never hard-codes 1/1/1 limited.
   - The player checklist (task 6) must include this tag. If a target player mis-renders transfer 13, switch to path B tagged BT.709 RGB (1/1/1 full, same pixels).
2. **REC-48.** Path A is the default. The thresholds are in the table above, and T-3 adds the zero-error capture-only check.
3. **REC-110 and REC-111.** "Hardware H.264" does not exist on the reference machine in Chrome on Linux. Restate both budgets for the encoder the ladder actually selects (software here). The spike scene runs 1080p at 123 frames/s, so the 60 frames/s budget at 1080p30 is plausible with software encoding; M1 measures it with the real engine on git/git.
4. **REC-53 and the risk row "Encoders that reorder frames".** No reordering is possible from Chrome's software H.264 encoder (POC type 2, zero reorder frames). M1 keeps `dts == pts` and the guard; `ctts` stays out of M1.
5. **Size estimate (section 5.9).** The output bitrate can be up to 2.4 times the `standard` target on dense, moving content, and far below it on quiet content. An upper-bound estimate that multiplies the target by duration under-estimates. Use 2.5 times the target for the upper bound until the M2 dry run measures the real size.
6. **CI and e2e timeouts.** Under the default headless renderer (SwiftShader), `VideoFrame(canvas)` is a CPU readback of 24 ms at 1080p and 90 ms at 4K. End-to-end throughput drops to 39 frames/s at 1080p and 11 at 4K, so real-pipeline e2e tests should record short clips or use the GL flags.
7. **Detached canvas (REC-43).** Confirmed; no attach fallback is needed.

**Not covered by the spike** (left for M1, unchanged in scope):

- Task 4: the hidden tab needs a headed browser.
- Task 5: File System Access staging needs a native picker and a user gesture, which headless automation cannot drive.
- Task 6: the player checklist needs a real MP4, so it moves to the first output of `gource-mux` (workstream f3-mux) and must include the 1/13/1 full-range `colr` tag.

### M1: Chrome desktop, H.264/MP4, main thread (smallest shippable slice, in two steps)

M1 lets a viewer save a video from their own settings including speed: size presets (720p, 1080p, 1440p, 4K, match window), frame rate 30 or 60, quality, the speed fields, Fit to N seconds, range whole history or from the current position, tail, Record, progress, Cancel with Stop and save, and a finished screen with a reproduce link and a sidecar `.json`.

Scope of M1 as a whole: REC-1, REC-2, REC-4 to REC-6 (presets above), REC-8 to REC-11, REC-12 (upper bound), REC-13 to REC-23, REC-30, REC-31 (snapshot exactness), REC-32 to REC-58, REC-61 to REC-65, REC-70, REC-72, REC-73 (without `record=1`), REC-74, REC-75, REC-80 to REC-83, REC-86, REC-87, REC-89, REC-94, REC-95, REC-100 to REC-104, REC-105 and REC-106 (the parts that apply without `record=1`), REC-110, REC-111, REC-113, REC-114 (dialog open; no preview), REC-116 to REC-120, REC-122 (plan-based), tests REC-90 to REC-93. Muxer: fragmented MP4 with patches (no embedded recipe). Sinks: File System Access and Blob. Estimates: upper bound, progress and ETA by the expected-time table. No preview, no dry run, no WebM, no worker, no MediaRecorder fallback, no stills, no flat MP4 unless M0 task 6 says so.

**M1a: engine and file path (no dialog).** Order of work, each with its own exit: (1) confirm that the M0 results and thresholds are committed to this document. (2) `gource-mux` and its tests; exit: T-18. (3) `record/` Rust module (options, plan, level and bitrate, driver, `LogSource`, `frame_hash`, empty-run rule) and the `hash_view` move (this step precedes F1 M1b); exit: T-4, T-6, T-8, T-9a (M1 part), T-20, T-21, T-22, T-26 (snapshot part), T-27 (without preview) pass natively and no golden file changes. (4) wasm classes, controller, encoder ladder, the two sinks, and the `window.__record` hook (test builds only); exit: a headless e2e run through the hook produces a file that passes T-2, T-7 and T-19.

**M1b: dialog and polish.** Dialog and viewer integration (M1 requirements of REC-1 to REC-24), upper-bound estimate, errors, accessibility, security items, mock factory and e2e. Exit criterion for M1 as a whole: on the reference machine, a 1080p30 recording of the git/git log restricted to about 5 minutes of video (`seconds-per-day` 1 with a roughly 300 day `start-date`/`stop-date` window set through the URL) completes at REC-110 throughput, plays in Chrome, VLC, QuickTime, Windows Films and TV and an upload target such as YouTube (checklist recorded in the PR), has the plan's frame count exactly and the right duration in `ffprobe`; passes T-1 to T-8, T-9a (M1 part), T-10 to T-15, T-16a, T-17 (REC-103 and F2 PR-108 are covered here), T-19 to T-22, T-24, T-25 (without the `localStorage` part), T-26 (snapshot part), T-27 (without preview), T-28 (without the hostile-URL case), T-30 and T-31 (T-29 when F1 M1b and M2 have shipped), T-3 within the thresholds committed after the spike (not placeholders), REC-119 is measured, and the existing test suites and frame hashes are unchanged. If M0 task 6 found a player that rejects or mis-times the fragmented file, the flat MP4 layout (`moov` at the end, written for seekable sinks) is part of M1 from the start. Fragmenting also keeps the truncation property only for the Blob sink and the M2 checkpoints (REC-64); the copy says so.

**M1 as the maintainer will see it.**

1. Open a loaded run and press **Record video** (or `Alt+R`): the dialog opens with 1920x1080, 30 fps, High quality, the viewer's speed and an "up to" length.
2. Change speed to 2 seconds per day, choose "From current position" or leave "Whole history", and press **Record...**.
3. Pick a file name in the browser's save dialog; a progress panel shows frames, elapsed time, ETA and the file size so far, while a banner says the live view is paused.
4. When it finishes: "Saved ... Stored on this computer. Nothing was uploaded." with **Copy reproduce link** and **Download recipe (.json)**.
5. Open the `.mp4` in any player: it plays, seeks and shows the run as it looked in the viewer, at the chosen size.
Not in M1: preview thumbnail, square and vertical sizes, date range, loop, the embedded recipe, the "about" estimate.

### M2: embedded recipe, preview, exact length, more formats, resilience

Scope: the embedded recipe and `record-metadata` (REC-71, REC-107, `udta/meta` in the muxer, T-16b); preview thumbnail (REC-7), `record=1` with the blocking confirmation (REC-3, REC-124); square, vertical and Custom sizes, 24 and 25 fps, Dates range and Loop (REC-6, REC-9, REC-10); the `about X` estimate and its accuracy test (REC-12, REC-69); dry run and exact progress (REC-12 second half, REC-67 to REC-68, REC-115); remembered values (REC-88); wake lock and tab title (REC-123, REC-24); `REC-125` and the REC-31 enumeration test; `faststart` layout (REC-59); WebM/VP9 (REC-60, ladder step 5); OPFS sink (REC-63 part 2); worker and `OffscreenCanvas` export (section 5.13, REC-84); real-time MediaRecorder fallback (ladder step 6) driven by the recorder's canvas in real time with an honest warning ("Real-time recording: the video is as long as the run takes to play, the tab must stay visible, and frames can be dropped"; `captureStream` timestamps are wall-clock, https://developer.mozilla.org/en-US/docs/Web/API/HTMLCanvasElement/captureStream; MP4 via `MediaRecorder.isTypeSupported`, Chrome 126+, https://developer.chrome.com/release-notes/126); optional crash-safe checkpoints (REC-66); "Save frame as PNG" at the output size (US-13); saved presets and recipe-file import (US-15); `gource-rs record-plan`; camera-path capture (Q6) as a candidate. The worker path adds a `LogSource` implementation; it does not rewrite `GourceRecorder::create`. The worker's `export_log` writes F1's batch v2 and sends multi-value options as one string joined by `\n`, so F1 M1a precedes M2.

Exit criterion: the planner's frame count equals the driver's on all fixtures (T-9b); a 20 minute hidden-tab recording in the worker completes with throughput within 20 % of the foreground run; faststart output has `moov` before `mdat` (box walk) and the right duration without patches; VP9/WebM plays in Chrome and VLC; the fallback recording of a 30 second run plays with the warning shown; the embedded recipe passes T-16b.

### M3: server rendering, stills and animated formats, audio

Prerequisite: F2 M2a (sealed or plain private cache, webhooks) and the PR-110 ticket for private renders; schedule PR-110 with this milestone.

Scope: section 5.14 (render queue, quotas, private repositories through F2, time-zone override, cache); animated GIF export (a small Rust LZW writer, 256-colour palette from the frame, up to 15 fps, 640 px) and a poster frame; an optional silent audio track only if a target platform requires one; camera path recording (Q6) if wanted.

Exit criterion: a recipe produced in the browser renders on the server to a file that has the same frame count and duration, a camera within tolerance of the browser's and passes the colour test; quotas and cancellation work; a private-repository render is possible only for its owner (F2 tests).

**Cross-feature ordering (identical in F1, F2, F3 and the README).** Hard dependencies: F3 M1a step 3 (the `hash_view` move to `gource-core`) before F1 M1b, or F1 M1b applies its DECO section at the new path; F1 M1a (batch v2) before F3 M2 (`export_log` must write v2 and send multi-value options as one string joined by `\n`); F1 M1b before F3 T-29 (and F1 M2 for `user-style` and `agent-key`); F2 M1b (CSP) and F3 M1b: whichever ships second adds F3 T-2, T-16 and T-19 (resp. the recording flows) to F2's `csp-clean` run; F2 M2a and PR-110 before F3 M3 private renders; the later spec "claimed styles" (F4) after F1 M3 and F2 M2a; the two `gource_web_bg.wasm` size budgets are cumulative (REC-119: at most +210 KiB gzip together with F1).

## 10. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Canvas capture colour is unspecified and can look darker (https://github.com/w3c/webcodecs/issues/598) | Wrong colours in every video | Spike T-S1 and test T-3 pick between capture paths A and B before anything else is built; REC-47/48 |
| Main-thread loop is throttled in a hidden tab | Long exports stall when the viewer switches tabs | REC-83 test T-13, visible-tab notice, worker export in M2 |
| File System Access output only appears at `close()` and is lost on a crash (REC-64) | A 30 minute render lost to a tab crash | Honest copy, `beforeunload`, optional checkpoints (REC-66) |
| Chromium builds without H.264, and encoder limits (Windows hardware encoder limits changed by version, macOS 4096x2304, Linux VA-API 4096x4096 at 30 fps; `docs/features/research/video-research.md` section 6) | `record_no_encoder` on some machines | Ladder (REC-52), VP9/WebM in M2, real-time fallback |
| Encoders that reorder frames (B-frames) | Muxer needs DTS and `ctts` | M1 treats it as an error with a `realtime` retry (REC-53); implement `ctts` if T-7 shows it on real encoders |
| Fork doubles log memory in the wasm heap, which never shrinks | About +133 MiB at 100k files; page memory stays high after a recording | Acceptable under wasm32's 4 GiB; documented; M2 worker has its own instance and exits with it |
| Glyph atlas eviction in a very long run with many scripts (2048 x 2048 cap) | Labels drawn at fallback metrics | REC-42 counter and warning; larger atlas if observed |
| Gource's simulation spikes (about 40 ms on a frame with hundreds of new files, PERFORMANCE.md) | Irregular wall-time per frame, not video time | None needed: video time is frame-indexed |
| Output differs between machines in tiny rasterisation details | "Same recipe, different bytes" | Spec guarantees same simulation, camera and timestamps, not identical bytes |
| The recipe URL grows with long filter lists or style rules | Link too long to share | The link omits nothing; above `SHARE_URL_SOFT_LIMIT` (2,000 characters) the UI offers the recipe JSON instead and warns that sign-in may drop options (REC-73, F2 PR-10) |
| Time zone not part of the URL state (REC-74) | A video made in another zone prints different times | Recorded in the recipe; server rendering overrides the zone (M3); Q4 |
| Browsers other than Chrome/Edge | No export, or untested export | Gated by feature detection only; Chrome and Edge are listed as tested; T-7 and T-10 run non-blocking on Firefox and Safari; MediaRecorder fallback in M2 helps partly |
| Hidden tab frozen or discarded, or the laptop sleeps, mid-recording | Whole file lost (REC-64) | Screen Wake Lock (REC-123), visible-tab notices (REC-83), REC-14 warning above 10 minutes, checkpoints in M2 (REC-66); T-13 slow variant documents the limit |
| Fragmented MP4 is mishandled by some players or editors | Unplayable or mis-timed video | M0 task 6 runs the checklist (Chrome, VLC, QuickTime, Windows Films and TV, an upload target) on a synthetic fragmented file before the dialog work and decides the layout; M1 exit repeats it |
| Embedded metadata or sidecar discloses names, filters or location | Privacy leak when a video is shared | Embedded copy omits time zone, creation time and name-bearing options (REC-71); private and unknown sources default to `none` and redacted sidecars (REC-103); Details popover previews the exact text |
| M3 server H.264 (headless Chromium builds without H.264, licensing) | Server renders fail or raise licensing questions | Image ships no H.264 encoder; VP9/WebM default on the server; branded Chrome only where the operator accepts the licence (section 5.14) |
| Heuristic estimate is off for strange logs | Misleading "about" figure | Always shown with the upper bound; exact dry run offered; REC-69 test on fixtures |

## 11. Open questions

| # | Question | Recommended answer |
|---|---|---|
| Q1 | Default frame rate: Gource's exporter defaults to 60 (`src/core/settings.cpp:88-89`); the dialog default is 30. | Keep 30: half the encode time and size, lower level (4.0 at 1080p), and sharing platforms handle 30 best. 60 is one click. Document the deviation in PARITY.md. |
| Q2 | Include 24 fps (not a Gource value)? | Yes. It costs nothing (3 ticks at 72 Hz) and matches common video; keep the C++ values 25, 30, 60 as the Gource-compatible ones. |
| Q3 | UI scale for presets: relative to 1080p (`auto`) or Gource's own `font_scale` rule? | `auto = min(width, height)/1080` so the image is resolution and orientation independent (a vertical export keeps its label proportions); `font_scale` is pinned to 1; "match window" uses the device pixel ratio (scaled with the size) so it equals the live view. Document in PARITY.md. |
| Q4 | Add a `time-zone` option to `Settings` so dates are reproducible across machines? | Not in M1 (the engine uses the browser zone and the offset table, `wasm.rs:40-55`). In M3 the server applies the recipe zone with a browser timezone override; reconsider a `Settings` field if the native CLI needs it. |
| Q5 | Should the live viewer keep running (real time) during a recording? | Not in M1: it is paused (REC-20) to leave the main thread to the export; M2's worker lets it run. |
| Q6 | Record manual camera moves as a replayable path? | Not in M1 (N3, REC-45; the dialog says so, US-16). An M2 candidate if that line draws complaints, otherwise M3: add an option `camera-path` (timestamped pan/zoom/rotate keyframes in simulation time); it fits the recipe because it is plain text. |
| Q7 | Do we need audio? | No. No video-codec constraint requires it; add a silent track only when a specific target rejects video-only files (the claim that YouTube or X require audio was not verified). |
| Q8 | Keyframe interval: 2 s (the feature request) or 0.5 s (YouTube's GOP recommendation)? | Default 2 s; `record-keyframe-seconds` is exposed. Platforms re-encode uploads, so the GOP recommendation does not bind the file we make. |
| Q9 | Flat faststart MP4 as the default in M2? | Yes for seekable sinks (YouTube recommends `moov` first); fragmented otherwise. Players handle both; the fragmented file survives truncation. |
| Q10 | Allow sizes above 4096 per side or 36,864 macroblocks (8K)? | No. Above level 5.2's frame size most encoders fail, and `MAX_RENDERBUFFER_SIZE` varies. Reconsider with M3 server rendering. |
| Q11 | Hardware preference default: `no-preference` or `prefer-hardware`? | `no-preference`; some hardware encoders ignore bitrate and GOP settings, so the ladder tries `prefer-software` second. The UI reports the requested preference and the accepted candidate, never "actually used", because WebCodecs does not expose that. |
| Q12 | Should the recipe be in the URL path for sharing recordings (a "recipe id")? | No. The URL parameters already are the recipe; an id would need server storage (M3 may add one for render jobs). |
| Q13 | Promote the tree runtime config (`z` gravity, `[` / `]` force gravity) to Gource-style options so the snapshot, URL and recipe carry them? | Not in M1: `z` is read nowhere and `[` / `]` are rarely used; REC-125 warns instead. If a viewer needs it, add `force-gravity` as an option (restart-class) in a later slice; it then flows through `set_option`, the URL and the snapshot with no format change. |
| Q14 | Default for `record-metadata` on public sources: `embed`, link only, or `none`? | `embed` of the reduced copy of REC-71 (no time zone, creation time or name-bearing option values), with the exact text shown in the Details popover and a one-click `none`. The sidecar always offers the full recipe, which satisfies D9 without silent disclosure. |
| Q15 | Keep a flat (non-fragmented) MP4 for seekable sinks out of M1? | Yes unless the M1 compatibility checklist fails (section 9). Fragmented MP4 is supported by the target players and keeps one writer in M1; the gate moves the flat layout in if it is not. |

## 12. References

Specifications and platform documentation

- WebCodecs: https://www.w3.org/TR/webcodecs/ (`VideoEncoder`, `encodeQueueSize`, `dequeue`, `EncodedVideoChunkMetadata.decoderConfig`)
- WebCodecs AVC registration (`avc` and `annexb` formats, `AVCDecoderConfigurationRecord` in `description`): https://www.w3.org/TR/webcodecs-avc-codec-registration/
- WebCodecs VP9 and AV1 registrations (no `description`): https://www.w3.org/TR/webcodecs-vp9-codec-registration/ , https://www.w3.org/TR/webcodecs-av1-codec-registration/
- WebCodecs codec registry: https://www.w3.org/TR/webcodecs-codec-registry/
- `VideoEncoder`: https://developer.mozilla.org/en-US/docs/Web/API/VideoEncoder ; `configure`: https://developer.mozilla.org/en-US/docs/Web/API/VideoEncoder/configure
- `VideoFrame` constructor (canvas init: `timestamp`, `duration`; buffer init requires `colorSpace`): https://developer.mozilla.org/en-US/docs/Web/API/VideoFrame/VideoFrame
- Chrome WebCodecs best practices (`isConfigSupported`, frame close, queue size): https://developer.chrome.com/docs/web-platform/best-practices/webcodecs ; Chrome 94: https://developer.chrome.com/blog/new-in-chrome-94
- Colour of canvas-sourced VP9 frames looks darker (open issue): https://github.com/w3c/webcodecs/issues/598
- `showSaveFilePicker`: https://developer.mozilla.org/en-US/docs/Web/API/Window/showSaveFilePicker ; File System Access (writes committed at `close()`, `seek`/`truncate`, Chrome 86): https://developer.chrome.com/docs/capabilities/web-apis/file-system-access ; `FileSystemWritableFileStream.seek`: https://developer.mozilla.org/en-US/docs/Web/API/FileSystemWritableFileStream/seek
- Origin private file system (workers' sync access handles, quota, cleared with site data): https://web.dev/articles/origin-private-file-system
- Blob size and `createObjectURL` limits: https://issues.chromium.org/issues/375385420
- Background tabs, rAF and timer throttling: https://developer.chrome.com/blog/background_tabs ; page lifecycle (freeze, discard exemptions): https://developer.chrome.com/docs/web-platform/page-lifecycle-api
- `OffscreenCanvas`: https://developer.mozilla.org/en-US/docs/Web/API/OffscreenCanvas
- Screen Wake Lock API: https://developer.mozilla.org/en-US/docs/Web/API/Screen_Wake_Lock_API
- `HTMLCanvasElement.captureStream`: https://developer.mozilla.org/en-US/docs/Web/API/HTMLCanvasElement/captureStream ; MediaRecorder MP4 in Chrome 126: https://developer.chrome.com/release-notes/126 , https://groups.google.com/a/chromium.org/g/blink-dev/c/p1OMVj1FrMI
- Chrome's fragmented MP4 with zero durations: https://blog.addpipe.com/duration-in-mp4-files-produced-by-chrome-safari/
- H.264 levels table (Main limits; High is 1.25x, High 10 is 3x): https://en.wikipedia.org/wiki/Advanced_Video_Coding ; ITU-T H.264 Table A-1
- YouTube upload recommendations (High profile, 4:2:0, BT.709, closed GOP of half the frame rate, `moov` first, bitrates): https://support.google.com/youtube/answer/1722171
- MP4 box registry: https://mp4ra.org/ ; Matroska notes: https://www.matroska.org/technical/notes.html
- Reference implementations to read, not copy: https://mediabunny.dev/guide/introduction , https://github.com/Vanilagy/mp4-muxer ; a minimal WebCodecs canvas-to-MP4 example: https://devtails.xyz/adam/how-to-save-html-canvas-to-mp4-using-web-codecs-api

In-repo sources

- `CLAUDE.md`, `docs/web-mvp/PLAN.md` (contracts, constraints), `docs/web-mvp/PERFORMANCE.md` (per-frame costs, memory, `PERFORMANCE.md:137-141`, `178`), `docs/web-mvp/PARITY.md`.
- C++ exporter: `src/gource.cpp:248-256`, `265-276`, `505-522`; `src/core/settings.cpp:88-89`, `452-467`; `src/gource_shell.cpp:221-226`.
- Research notes: `docs/features/research/playback-record.md`, `docs/features/research/video-research.md`, `docs/features/research/user-render.md`.
