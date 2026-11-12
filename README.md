# Gource Online

Watch a software project's history grow in your browser. This is a Rust and WebAssembly port
of [Gource](https://gource.io) by Andrew Caudwell: the same simulation and look, drawn with
WebGL2, plus pull requests and AI co-authors in the same tree.

**Beta.** Try it at [gource.online](https://gource.online), or on your own machine, inside any git checkout:

    npx gource-online@beta        # nothing is uploaded

The command reads the history with git, keeps it in memory and opens the player in your browser. It
works with any repository, private or public.

## Ways in

- **A public GitHub or GitLab URL** on [gource.online](https://gource.online).
- **Your own checkout:** `npx gource-online@beta` (`apps/cli`).
- **A log file:** drop a Gource custom log or `git log` output on the player, or link one with
  `#src=<url>` (`apps/player`).
- **Your own page:** the `<gource-viewer>` element (`packages/viewer`).

## A port of Gource

Gource is a C++ and OpenGL program. This repository is a fork of it: upstream's tree is kept
unchanged as the reference, and the port lives beside it.

| Path | What | Where it comes from |
|---|---|---|
| `src/`, `data/`, `cmd/`, `contrib/`, `dev/`, `resources/`, `tests/`, the autotools files, `README.gource.md` | Gource 0.57, C++ | Upstream at commit `70db547`, unchanged. `README.gource.md` is upstream's README. Added: `data/decorations.png` and `tests/fixtures/custom-log/` |
| `crates/gource-core` | The simulation | Ported from `gource.cpp`, `dirnode.cpp`, `file.cpp`, `user.cpp`, `action.cpp`, `zoomcamera.cpp`, `gource_settings.cpp` and parts of Core |
| `crates/gource-sources` | Log readers and the custom log writer | Ported from `formats/custom.cpp`, `formats/git.cpp`, `formats/commitlog.cpp` |
| `crates/gource-render` | Frame building and draw order | Ported from `gource.cpp` (`draw`, `updateVBOs`), `spline.cpp`, `key.cpp`, `textbox.cpp` |
| `crates/gource-render-webgl2` | The WebGL2 backend | Shaders rewritten from `data/shaders/`; Gource's sprites |
| `crates/gource-web` | The wasm module | New; drives `update` and `draw` as `Gource` does |
| `crates/gource-cli` | `gource-rs`: log conversion, headless runs | New; `convert` is `gource --output-custom-log` |
| `crates/gource-mux` | MP4 writer for the recorder | New |
| `packages/viewer` | `<gource-viewer>` | New; its option list follows `gource_settings.cpp` |
| `apps/player` | The player: playback, options, recording, AI statistics | New |
| `apps/cli` | `npx gource-online` | New; it runs Gource's own git command |
| `packages/protocol` | The shared contract (MIT) | New; it pins the git command of `formats/git.cpp` |

[`crates/README.md`](crates/README.md) and each crate's README map the port file by file to the C++
it follows, and say what was added and what was left out. `docs/web-mvp/PARITY.md` records how
closely the frames match.

The branch `master` is upstream Gource's history, untouched. This project's work is on `main`.

## Documentation

- Build and test: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- How the parts fit: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
- Features beyond Gource: [`docs/features/`](docs/features/) (AI co-authors, recording, the player's handoff)
- CI and security checks: [`docs/ci-readiness.md`](docs/ci-readiness.md)
- Reporting a vulnerability: [`SECURITY.md`](SECURITY.md)

## Licence

GPL-3.0-or-later, as Gource is, except `packages/protocol` (MIT). See [`LICENSING.md`](LICENSING.md).
Gource is © Andrew Caudwell and contributors; `THANKS` lists upstream's contributors.
