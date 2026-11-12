# gource-web

The WebAssembly module and its JavaScript API (`wasm-bindgen`). `scripts/build-web.sh` builds it into
`packages/viewer/src/wasm/`.

## What it ports

| File | C++ source | Notes |
|---|---|---|
| `engine.rs` | `gource.cpp`: one display frame is `Gource::update` then `Gource::draw` | Generic over the GPU backend and the clock, so it is tested natively |
| `input.rs` | SDL key and mouse events as `Gource` receives them | From DOM events |
| `record/timing.rs` | `gource.cpp`: `Gource::setFrameExporter` | The fixed time step of a video export: the smallest multiple of the frame rate that is at least 60 Hz |

It takes the place of `gource_shell.cpp` and `main.cpp`, which are not ported: the page owns the window, the
options and the life cycle.

## What is new

| File | What |
|---|---|
| `wasm.rs` | `GourceApp` (main thread: event log, simulation, frame and WebGL2 on one canvas) and `LogParser` (Web Worker) |
| `parser.rs` | Log bytes in, encoded event batches out, for the worker |
| `status.rs`, `ai.rs` | The engine status and the AI statistics as JSON |
| `record/` | Recording to MP4: a second engine that renders every frame at a fixed step, the encoder settings, the length estimate, the stop rules and the muxer (`docs/features/recording.md`). Gource writes PPM frames to a pipe instead |

    cargo test -p gource-web
    cargo build -p gource-web --target wasm32-unknown-unknown

GPL-3.0-or-later, derived from Gource. See `LICENSING.md`.
