# gource-render

Turns a scene snapshot from `gource-core` into vertex streams and draw commands: Gource's per-frame VBO rebuild
and its draw-pass order. It makes no GPU calls; a backend (`gource-render-webgl2`) executes the frame.

## What it ports

| File | C++ source | Notes |
|---|---|---|
| `builder.rs` | `gource.cpp`: `draw`, `drawScene`, `updateVBOs`, `updateAndDrawEdges`, `drawFileShadows`, `drawActions`, `drawFiles`, `drawUserShadows`, `drawUsers`, `drawBloom`, and the name and HUD passes | The pass order is Gource's: edges, file shadows, actions, files, user shadows, users, bloom, text |
| `emit.rs` | `core/vbo.cpp` (`quadbuf::add`), `bloom.cpp` (`bloombuf::add`) | Vertex layouts: position, colour, texcoord |
| `spline.rs` | `spline.cpp`: `SplineEdge::update`, `getLabelPos`, `drawToVBO` | Directory edges in screen space |
| `key.rs` | `key.cpp` (`FileKeyEntry`, `FileKey`) | The file-extension key |
| `textbox.rs` | `textbox.cpp` | The hover tooltip |
| `text.rs` | `core/fxfont.cpp` (`FXFont`) | Text layout; the glyphs come from the backend's atlas |

Per-entity drawing that the C++ keeps in `dirnode.cpp`, `file.cpp`, `user.cpp` and `action.cpp` is here too,
since `gource-core` has no drawing.

## What is new

| File | What |
|---|---|
| `lib.rs` | The frame contract and the backend trait |
| `deco.rs`, `deco_atlas.rs` | AI decorations around avatars, from `data/decorations.png` (made for this project) |
| `agent_key.rs` | The agent key and the AI filter line in the HUD |

    cargo test -p gource-render

GPL-3.0-or-later, derived from Gource. See `LICENSING.md`.
