# @gource-online/viewer

Gource's renderer for a web page, as the custom element `<gource-viewer>` (GPL-3.0-or-later). It
holds the canvas with the WebAssembly engine on it, forwards pointer, wheel and key input to the
engine, keeps it sized to the canvas, and runs the render loop. The log is parsed off the main
thread by a worker. Controls, panels and recording belong to the page around it.

Importing the package defines the element. The engine module is generated into `src/wasm/` by
`scripts/build-web.sh`; `mock` uses the Canvas2D mock engine instead.

## What comes from Gource

| Folder | What | Gource source |
|---|---|---|
| `src/wasm/` (generated) | The engine: Gource's simulation and drawing, ported | `crates/`, mapped to the C++ in `crates/README.md` |
| `src/options.ts` | The options a page can offer: names, value syntax and defaults | `gource_settings.cpp` (`setGourceDefaults`, `importGourceSettings`) |
| `src/element.ts` | The canvas, its input and the render loop | The SDL window and event loop of `gource_shell.cpp`, as a custom element |
| `src/log-loader.ts`, `src/worker/` | Parsing the log off the main thread | `logmill.cpp`, which reads the log on its own thread |
| `src/engine/` | Loading the engine, and a Canvas2D mock for tests | None |

## Playing a log by itself

```html
<gource-viewer src="/history.log" font-src="/fonts/FreeSans.ttf"></gource-viewer>
```

The element creates the engine, loads `src` (a custom log or the output of Gource's git command)
and starts playing at the first commit, without taking keyboard focus from the page. Or call
`viewer.play(source, options, focus)` with a `{ kind: 'url', url }` or `{ kind: 'file', file, name }`
source and Gource option values.

| Attribute | |
|---|---|
| `src` | The log URL, set before or after the element is on the page. One log per element: a later change is ignored with a warning. |
| `font-src` | The label font, loaded before the engine starts (at most 4 s). |
| `mock` | Use the mock engine. |

## With an engine the page creates

```ts
const viewer = document.createElement('gource-viewer');
container.append(viewer);
const factory = await loadEngine(false);
viewer.attach(factory.createApp(viewer.canvas));
viewer.load({ kind: 'url', url });   // or stream it yourself with LogLoader
viewer.start();                      // the render loop, when the page shows it
```

## Properties and methods

- `canvas`: the canvas; the page may label it.
- `engine`: the `GourceApp`, for options (`set_option`, `get_option`, `restart`), playback
  (`set_paused`, `seek`, `date_at`), and recording.
- `status()`: the parsed engine status, or null.
- `playing`: whether the render loop runs.
- `keyScope`: one more element whose key presses count as aimed at the visualisation, besides the
  canvas and the page body.
- `start(focus = true)`: the render loop; it focuses the canvas unless `focus` is false.
- `resize()`, `dispose()`: dispose stops the loop, the input and the download, and frees the
  engine. Removing the element from the page disposes it (moving it within the page does not),
  and a disposed element stays disposed.

## Events

| Event | `detail` |
|---|---|
| `gource-progress` | `{ bytes, commits, playable }` as the log arrives |
| `gource-done` | `{ bytes, commits, skipped, format }`: the whole log is in, seeking works |
| `gource-error` | `{ code, message }`: `engine_missing`, `engine_failed`, `log_failed`, `parse_failed`, `empty_log` |
| `gource-render-error` | `{ message }`: a frame failed and the loop stopped |
| `gource-key` | `{ key }`: the engine handled a key, which may have changed its options |

Escape never reaches the engine: it is the page's (to close a panel or leave the canvas). Tab is
Gource's "next user" only on the focused canvas, and Shift+Tab always moves focus.
