# The Rust port

These crates are a port of [Gource](https://gource.io) 0.57 (C++, in `src/` of this repository) to Rust and
WebAssembly. Each crate's README lists, file by file, which C++ source it follows and what it adds.

| Crate | Ports | README |
|---|---|---|
| `gource-core` | The simulation: `gource.cpp` (`logic`, the read side of `draw`), `dirnode.cpp`, `file.cpp`, `user.cpp`, `action.cpp`, `pawn.cpp`, `zoomcamera.cpp`, `gource_settings.cpp`, and parts of Core (`stringhash`, `quadtree`, `frustum`, `bounds`, `vectors`) | [gource-core](gource-core/README.md) |
| `gource-sources` | The log readers: `formats/custom.cpp`, `formats/git.cpp`, `formats/commitlog.cpp`, and Core's date and regex handling | [gource-sources](gource-sources/README.md) |
| `gource-render` | The frame: `gource.cpp` (`draw`, `drawScene`, `updateVBOs`), `spline.cpp`, `key.cpp`, `textbox.cpp`, `bloom.cpp`, Core's `vbo.cpp` and `fxfont.cpp` | [gource-render](gource-render/README.md) |
| `gource-render-webgl2` | The GL side: `data/shaders/*`, Core's `fxfont.cpp` and `texture.cpp`, the sprites in `data/` | [gource-render-webgl2](gource-render-webgl2/README.md) |
| `gource-web` | The application loop: `Gource::update` and `Gource::draw` per frame, `Gource::setFrameExporter` for recording | [gource-web](gource-web/README.md) |
| `gource-cli` | `gource --output-custom-log`, and headless runs for parity checks | [gource-cli](gource-cli/README.md) |
| `gource-mux` | Nothing in Gource (it writes PPM frames for an external encoder) | [gource-mux](gource-mux/README.md) |

## What "port" means here

- **Same behaviour first.** The simulation, the layout forces, the camera, the colour hash and the draw order are
  Gource's. Where the C++ depends on a compiler or library detail (float rounding, `char` signedness, glibc's
  `rand()`), the Rust reproduces that detail and a test compares it with values taken from the C++ build.
- **The C++ is the reference, and it stays.** `src/`, `data/` and the build files are upstream's, unchanged.
  `scripts/reference/` renders frames from the unmodified C++ in Docker for side-by-side comparison.
- **What is not ported:** the SDL application shell (`gource_shell.cpp`, `main.cpp`), the other version-control
  readers (`hg`, `bzr`, `svn`, `cvs`, `apache`), `logmill.cpp` (a browser cannot run a version-control
  program), the position slider (`slider.cpp`, replaced by the player's own) and PPM output. Captions
  (`caption.cpp`), the logo, the background image and user avatar images are not drawn yet.
- **What is new:** pull requests and AI co-authors as event kinds in the same stream, the contributor style
  rules, MP4 recording in the browser, and a deterministic replacement for `rand()`.

How far each behaviour matches is tracked in `docs/web-mvp/PARITY.md`.

## Licence

GPL-3.0-or-later: a derivative work of Gource, © Andrew Caudwell and contributors. See `LICENSING.md`.
