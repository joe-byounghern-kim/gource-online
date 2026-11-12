<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Visual parity with the C++ Gource

How closely the Rust/WebAssembly port reproduces the reference renders of
unmodified C++ Gource 0.57 (`scripts/reference/`), what was fixed to get
there, what still differs and why, and how to rerun the comparison.

Summary: with the hidden option `rand-mode=glibc` the port draws the
frames of the C++ build for the whole of every scenario, including 67 s of
Gource's own first year of history (up to 9 users, about 1200 file
events). The layout, entity positions, colours, sizes, bloom, edges,
beams, shadows, label positions and fades, date, title and file key all
line up to the pixel. What is left is text rasterisation (canvas 2D instead
of FreeType's hinted outlines), a one- or two-level per-pixel noise in the
bloom (its hash `fract(sin(x) * 2351.37)` depends on each GPU's `sin`),
and small texture-filtering differences at sprite edges. In the default
keyed mode the port is the same visualisation with a different arrangement
(the tie-breaks that `rand()` decided in Gource come from keyed hashing).

## Method

1. **Reference.** `target/reference/<scenario>/` from
   `scripts/reference/render-all.sh`: 1280x720, `--output-framerate 30`, so
   saved frame k (every 15th) is the state after 2k + 1 ticks of 1/60 s
   (`manifest.json` lists the ticks), Mesa llvmpipe, `TZ=UTC`.
2. **Port render** (`scripts/parity/render.mjs`). Headless Chrome through
   Playwright, one page per scenario: a 1280x720 canvas at device pixel
   ratio 1, time zone UTC, FreeSans loaded as `GourceSans`, the scenario's
   `command.txt` options applied by Gource name (`stop-at-end`,
   `hide=mouse`, `disable-input`, and for `gource-history` the start/stop
   dates and `seconds-per-day 0.15`) plus `rand-mode`, the whole
   `input.log` parsed with `LogParser` and loaded before the first tick
   (the C++ loads the log on a thread before its first update), then
   `GourceApp.step` at a fixed dt of 1/60 s up to each reference frame's
   tick count and `canvas.toDataURL()` in the same task. The harness page is
   served by the script itself from `web/src/wasm/`; it does not use the web
   app, its panel or the clone service. Default GPU: ANGLE on the NVIDIA
   OpenGL driver (`--use-angle=gl`); SwiftShader gives the same numbers to
   within 0.01.
3. **Comparison** (`scripts/parity/compare.py`, Pillow only), per frame:
   - **MAD**: mean absolute difference over all pixels and channels (0-255),
     also per channel;
   - **>24 levels**: share of pixels whose largest channel difference
     exceeds 24;
   - **MAD 1/8**: MAD after box-filtering both frames to 160x90, which keeps
     structure and colour and drops sub-pixel and glyph-shape differences;
   - **block SSIM**: mean SSIM of the luma over 8x8 blocks;
   - plus side-by-side sheets (reference | port | difference x4) that were
     read by eye at early, middle and late frames, and full-resolution crops
     of every region the difference images flagged.

## rand-mode glibc

Gource calls `rand()` in four places: overlap tie-breaks in
`RDirNode::applyForceDir`, `RUser::applyForceUser` and
`RUser::applyForceAction` (`vec2((rand() % 100) - 50, (rand() % 100) - 50)`,
which g++ evaluates y first), and `Gource::changeColours`. It never calls
`srand` (only `--start-position random` does), so every run sees glibc's
default sequence. The port replaces those calls with keyed hashing, one
stream per call site, as `CLAUDE.md` requires; frame-0 layouts hit the
tie-breaks at once (every node starts at its parent's position), so keyed
layouts diverge from the reference within a few frames.

`rand-mode` is a port-only option (`Settings::rand_mode`, values `keyed`,
the default, and `glibc`; effect `restart`; it is not in the web panel's
option list). With `glibc`, `Simulation::new` builds one `GlibcRand`
(`crates/gource-core/src/rand.rs`: glibc's `random_r` TYPE_3 additive
feedback generator with `srandom_r`'s Park-Miller initialisation and 310
discarded outputs, output `x >> 1`) shared by every call site, in the C++
call order, never rewound by a reset (Gource never reseeds). Keyed mode is
unchanged: every existing golden still matches.

Checks:

- `rand::tests::matches_glibc_rand`: the first 1000 values equal glibc 2.39's
  (`scripts/parity/glibc-rand.c`, output in
  `crates/gource-core/tests/golden/glibc_rand_1000.txt`).
- `sim::tests::simulation_matches_cpp_golden_with_glibc_rand`: the
  simulation golden harness (`tests/golden/sim/`, Gource's own `dirnode.cpp`,
  `user.cpp`, `action.cpp`, `zoomcamera.cpp`, `file.cpp`, `pawn.cpp` driven by
  a transcription of `gource.cpp`'s logic) is now also linked against
  glibc's real `rand()` (`sim_golden_glibc.tsv`), and the port in glibc mode
  matches its 2490 rows bit for bit. The scenario draws 34 values in
  `applyForceDir` and 12 in `applyForceUser`, interleaved, so the shared
  call order is covered.
- The renders below: with `glibc` every scenario stays on the reference
  layout to its last frame.

## Results

Means over every saved reference frame (`frames` per scenario). Before: the
port as merged on `web-mvp`, rendered with `rand-mode=glibc`. After: this
branch.

Before (glibc):

| scenario | frames | MAD | >24 levels | MAD 1/8 | block SSIM |
|---|---:|---:|---:|---:|---:|
| custom-dir-delete | 29 | 1.187 | 0.153% | 1.041 | 0.9957 |
| deletion-recreation | 28 | 1.220 | 0.176% | 1.049 | 0.9953 |
| dense-contributors | 28 | 1.240 | 0.200% | 1.050 | 0.9947 |
| file-removal | 30 | 1.190 | 0.154% | 1.041 | 0.9957 |
| file-to-dir | 29 | 1.197 | 0.153% | 1.040 | 0.9954 |
| gource-history | 136 | 1.545 | 0.497% | 1.108 | 0.9831 |
| utf8 | 28 | 1.200 | 0.168% | 1.046 | 0.9956 |
| visual-hierarchy-activity | 28 | 1.256 | 0.217% | 1.056 | 0.9944 |

After (glibc):

| scenario | frames | MAD | >24 levels | MAD 1/8 | block SSIM | worst MAD 1/8 (frame) |
|---|---:|---:|---:|---:|---:|---:|
| custom-dir-delete | 29 | 0.072 | 0.087% | 0.020 | 0.9984 | 0.033 (75) |
| deletion-recreation | 28 | 0.071 | 0.091% | 0.022 | 0.9986 | 0.033 (75) |
| dense-contributors | 28 | 0.101 | 0.111% | 0.030 | 0.9980 | 0.050 (75) |
| file-removal | 30 | 0.072 | 0.088% | 0.019 | 0.9984 | 0.035 (90) |
| file-to-dir | 29 | 0.088 | 0.089% | 0.021 | 0.9981 | 0.028 (75) |
| gource-history | 136 | 0.751 | 0.398% | 0.176 | 0.9860 | 0.336 (90) |
| utf8 | 28 | 0.077 | 0.098% | 0.022 | 0.9983 | 0.045 (75) |
| visual-hierarchy-activity | 28 | 0.126 | 0.127% | 0.035 | 0.9977 | 0.066 (75) |

The remaining ~0.09% of pixels over 24 levels in the small scenarios is the
date label and the user and file names (an empty late frame, date only,
scores 0.08-0.09%). `gource-history` has hundreds of labels on screen and
large blooms, hence its higher numbers; its MAD is dominated by the bloom
noise (about 30% of the pixels of frame 90 differ by 1 or 2 levels; NVIDIA and
SwiftShader renders of the port differ from each other by MAD 0.53 there).

Progression on `gource-history` (MAD / >24 levels / MAD 1/8): 1.545 / 0.497%
/ 1.108 before; 0.781 / 0.431% / 0.219 after the clear colour, the colour
clamp and the font ascender; 0.771 / 0.417% / 0.217 after the glyph
heights; 0.751 / 0.398% / 0.176 after the glyph coverage.

Two extra C++ renders exercise what the default scenarios do not (rendered
with `scripts/reference/render.sh` at `REF_FPS=60 REF_EVERY=30` into
`target/parity/reference-extra/`, see "Rerunning"):

| scenario (C++ flags) | MAD | >24 levels | MAD 1/8 | block SSIM |
|---|---:|---:|---:|---:|
| bloom-intense (`--bloom-intensity 2.5 --bloom-multiplier 1.5`), before the clamp | 1.426 | 2.676% | 1.362 | 0.9885 |
| bloom-intense, after | 0.225 | 0.117% | 0.047 | 0.9946 |
| key-title (`--key --title Parity`, dense-contributors log), after | 0.115 | 0.130% | 0.033 | 0.9979 |

Default keyed mode (no `rand-mode`), same renders:

| scenario | MAD | >24 levels | MAD 1/8 | block SSIM |
|---|---:|---:|---:|---:|
| custom-dir-delete | 0.413 | 0.608% | 0.343 | 0.9930 |
| deletion-recreation | 0.330 | 0.450% | 0.270 | 0.9935 |
| dense-contributors | 0.839 | 1.177% | 0.750 | 0.9844 |
| file-removal | 0.321 | 0.490% | 0.264 | 0.9949 |
| file-to-dir | 0.362 | 0.495% | 0.283 | 0.9933 |
| gource-history | 10.579 | 12.154% | 9.632 | 0.8499 |
| utf8 | 0.321 | 0.415% | 0.265 | 0.9937 |
| visual-hierarchy-activity | 0.871 | 1.110% | 0.718 | 0.9821 |

Keyed mode looks like Gource: the same tree, node sizes, file rings,
colours, bloom, beams, labels, camera framing and timing, with branches
leaving their parents in other directions and users that started on top of
each other drifting apart in other directions (`history-1020-keyed.jpg`).
That is the intended consequence of replacing `rand()`, not a defect.

Illustrations (in `docs/web-mvp/parity/`):

- `history-1020-glibc.jpg`: `gource-history` frame 1020, reference | port |
  difference x4, glibc mode. Only glyph outlines show in the difference.
- `history-1020-keyed.jpg`: the same frame in keyed mode.
- `dense-75-glibc.jpg`: `dense-contributors` frame 75 at 1:1.
- `labels-before-after.jpg`: the date and a user name, reference / before /
  after, 3x.
- `bloom-clamp.jpg`: `bloom-intense` frame 150, reference / without the
  colour clamp / with it.

## Findings

Fixed:

1. **Vertex colours were not clamped** (`gource-render-webgl2/src/shaders.rs`).
   Legacy GL clamps `gl_FrontColor` and fixed-function colours to [0, 1]
   (`GL_CLAMP_VERTEX_COLOR`, on by default) before interpolation; WebGL
   does not. Bloom colours (`node colour * bloom_intensity`) above 1 made
   the glow wider and whiter, and labels with alpha up to 3 (the file name
   fade-in) drew heavier. Both vertex shaders now clamp. `bloom-intense`:
   2.68% of pixels over 24 levels down to 0.12% (the commit message quotes
   2.76%, measured against a first render of that reference that had hit the
   loader race described below).
2. **Background one level dark on NVIDIA** (`backend.rs`). The default
   background 0.1 is exactly 25.5 levels; Mesa clears it to 26, NVIDIA's GL
   (clear and shader output alike) to 25. The clear colour is now snapped to
   a quarter level above its round-to-nearest level, which every driver
   maps to the same value. This was the 1.0 floor of every "before" MAD.
3. **Text one pixel high or low** (`atlas.rs`). Canvas rounds
   `fontBoundingBox*` and `actualBoundingBox*` to whole pixels. `FXFont`
   uses FreeType's unrounded face ascender/descender times `y_ppem / em`
   (`getHeight = ceil(asc + desc)`: 9 px at 12 px, 12 px at 16 px; canvas
   gave 8 and 11, lifting top-aligned text such as the date by a pixel), and
   `getMaxHeight` is the tallest hinted glyph height of the precache set
   (15 px at 14 px; canvas gave 14, so user names sat a pixel low). The
   ascender and descender are now measured at 1000 px and scaled like
   FreeType's `unit_scale`, and glyph heights are estimated as the
   unhinted ink ascent and descent each rounded up, which equals
   FreeType's hinted maximum for FreeSans at 33 of the sizes 8-48,
   including 10-14, 16, 18, 20 and 24 (verified with FreeType in the
   reference image).
4. **Text about 10% heavier** (`atlas.rs`). Skia's contrast/gamma preblend
   gives white glyphs about 13% more coverage than FreeType's linear
   grayscale (black glyphs about 4%). Glyphs are rasterised in black and
   recoloured white with a clipped `source-in` fill (the text shader reads
   alpha only). The date's ink is now within 2% of the reference.
5. **Port could not be rendered at the reference frames with the reference
   `rand()`**: added `rand-mode=glibc` (above) and the comparison scripts.

Open / won't fix:

6. **Glyph shapes** (wontfix): FreeType hints FreeSans to the pixel grid
   (crisper stems, slightly different x-heights); Chrome's canvas rasteriser
   does not. Positions, advances, sizes, shadows and fades match; the
   outlines differ by a pixel here and there. Matching would need a
   FreeType rasteriser in the wasm, which the plan rules out.
7. **Bloom noise** (wontfix): `bloom.frag` dithers with
   `fract(sin(dot(pos, ...)) * 2351.3718)` at world-space arguments in the
   thousands, where every GPU's `sin` returns different low bits. The
   result is the same procedural glow with a different 1-2 level grain
   (also between NVIDIA and SwiftShader). The maths is Gource's and stays.
8. **Sprite edges** (wontfix): file, user and beam edges differ by a few
   levels (mipmaps from `generateMipmap` vs `gluBuild2DMipmaps`, and
   rasteriser coverage rules).
9. **Headless Vulkan loses every WebGL context** (open, environment): on the
   test machine (RTX 3080 Ti, Chrome with `--use-angle=vulkan`) even a bare
   `getContext('webgl2')` is lost within a second, so no frame can be read.
   `--use-angle=gl` (NVIDIA OpenGL through ANGLE) and SwiftShader work and
   are what the scripts use. Not a port issue; worth checking on a desktop
   Vulkan Chrome before calling it one.
10. **C++ loader race** (documented, `scripts/reference/README.md`): a C++
    render can be sampled one tick late. It happened once here (the first
    `bloom-intense` render at 30 fps; the port matched it with
    `--tick-offset 1`). All eight committed scenarios match best at offset
    0. Extra references should be rendered at `REF_FPS=60`, which is immune.
11. **Not compared**: mouse hover/selection text boxes, captions, logo and
    background images, avatars (no C++ scenario uses them, or the port does
    not implement them); HiDPI (the references are DPR 1).

## Rerunning

```sh
export PATH="$HOME/.cargo/bin:$PATH"
scripts/reference/render-all.sh                 # once: target/reference/ (Docker)
(cd web && pnpm install --frozen-lockfile)      # Playwright
scripts/build-web.sh                            # web/src/wasm/
scripts/parity/run.sh                           # glibc and keyed renders + comparison
# results: target/parity/port-{glibc,keyed}/summary.md, <scenario>/metrics.{csv,json},
#          <scenario>/sheet-*.jpg (first live, middle, last frame)
```

Pieces, for other setups:

```sh
# reference root elsewhere (for example the main checkout's)
PARITY_REF=/path/to/target/reference scripts/parity/run.sh
# one scenario, every 4th reference frame, on SwiftShader
node scripts/parity/render.mjs --gpu swiftshader --every 4 --out target/parity/x gource-history
python3 scripts/parity/compare.py target/parity/x --sheets 5
# extra reference renders with other flags (60 fps avoids the loader race)
REF_FPS=60 REF_EVERY=30 scripts/reference/render.sh \
    tests/fixtures/custom-log/visual-hierarchy-activity.log \
    target/parity/reference-extra/bloom-intense --bloom-intensity 2.5 --bloom-multiplier 1.5
PARITY_REF=target/parity/reference-extra node scripts/parity/render.mjs --out target/parity/extra
python3 scripts/parity/compare.py target/parity/extra --ref target/parity/reference-extra
```

`render.mjs --help` lists its flags (`--rand-mode`, `--gpu`, `--every`,
`--option name=value`, `--tick-offset`). The full run (8 scenarios, both
modes) takes about a minute, mostly the comparison. Everything is written
under `target/parity/` (ignored by git).

To use the option elsewhere: `set_option("rand-mode", "glibc")` and then
`restart()` (the core and `GourceApp`); the web page does not read it from
its URL, by design.
