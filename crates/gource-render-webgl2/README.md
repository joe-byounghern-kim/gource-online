# gource-render-webgl2

The backend: it executes a frame from `gource-render` with raw WebGL2 calls through `web-sys`. No rendering
engine or scene graph is used. On targets other than wasm the crate is empty.

## What it ports

| File | C++ source | Notes |
|---|---|---|
| `shaders.rs` | `data/shaders/bloom.{vert,frag}`, `shadow.{vert,frag}`, `text.{vert,frag}`, and the fixed-function `texture * colour` path | Rewritten in GLSL ES 3.00; the maths of the bloom and the text shadow is unchanged |
| `backend.rs` | `gource.cpp`: `Gource::drawScene` and the text and HUD passes of `Gource::draw` | Blend modes and pass order as in the C++ |
| `atlas.rs` | `core/fxfont.cpp` (`FXGlyphSet`, `FXGlyphPage`) | The browser's canvas rasterises the glyphs instead of FreeType |
| `textures.rs`, `build.rs` | `core/texture.cpp` (`TextureManager::grab`) | Gource's sprites `data/file.png`, `user.png`, `beam.png`, decoded at build time, with mipmaps and trilinear filtering as upstream |

## What is new

| File | What |
|---|---|
| `packer.rs`, `budget.rs` | A bounded glyph atlas with eviction and a per-frame rasterisation budget. Gource adds texture pages without limit |
| `streams.rs` | Vertex streams that grow geometrically and allocate nothing in a steady frame |
| `demo/` | A page that draws a hand-made frame exercising every program and blend mode |

    cargo clippy -p gource-render-webgl2 --target wasm32-unknown-unknown -- -D warnings

GPL-3.0-or-later, derived from Gource. See `LICENSING.md`.
