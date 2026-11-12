<!-- Research notes gathered on 2026-10-02 (branch web-mvp, HEAD 8a3ecec) for the feature specs in docs/features/. Working notes: claims are marked verified or inferred; file:line anchors describe that commit. -->

# Fact notes: user simulation, drawing and options, for AI-contributor decorations

**Verified** means I read the cited code. **Inferred** means my reasoning from it. Nothing was edited. No external URLs were consulted, so every claim is from this repo.

## 1. How a user is simulated (verified)

- **State.** `RUser` (`crates/gource-core/src/user.rs:64-85`) embeds a `Pawn` (`pawn.rs:26-53`). The `Pawn` holds `pos`, `accel`, `size`, `dims`, `elapsed` and `selected`. `RUser` adds `usercol`, `highlighted`, the action queues and `last_action`.
- **Pawn fields that could matter for a decoration.** `pawn.rs` has no heading, rotation or facing field.
  - `accel` doubles as velocity. It is capped at `speed`, applied as `pos += accel*dt`, then damped by `user_friction` (`user.rs:427-434`).
  - `accel` is private to the simulation and does not reach the view.
- **Active state.** Three signals exist today, none exported.
  - `is_idle` is true when both action lists are empty (`user.rs:184`).
  - `is_fading` is true when idle for longer than `user_idle_time`.
  - `active_actions` holds beaming actions. Actions have a `kind` in `action.rs`; I did not read its details.
- **Colour.** `colour_hash(name, seed)`, mixed `*0.6 + 0.4 white`, then `*0.9` (`user.rs:134-139`).
  - `colourize()` on a reseed gives the raw hash, which is more saturated (`user.rs:142`).
  - `colour()` returns white when selected (`user.rs:212`).
- **Alpha.** `alpha()` is `min(elapsed/1.0, 1)`, then a 1 s fade-out after `user_idle_time` (`user.rs:232-239`). Users are deleted after 10 s idle (`user.rs:61`).
- **Size.** The base is `20 * user_scale`, and `dims = (size, size*512/384)` (`user.rs:99`, `pawn.rs:85-91`). The art is 384x512 (`user.rs:53`, confirmed by `file data/user.png`). `extract_users` multiplies `dims` by `-camera.z/300` when `fixed_user_size` is set (`sim/extract.rs:323-334`).
- **Creation.** `add_user(username)` runs the first time a commit change references a user (`sim.rs:1300-1315`). The key is the username string only, in `user_map: BTreeMap<String, UserId>` (`sim.rs:247`).
  - That makes draw order alphabetical by name. Decorations therefore inherit a stable order.
  - The user is created inside `add_file_action`, which does not look at commit metadata.
- **Commit event model.** `CommitRecord` is `{timestamp, user: StrId, first_change, change_count, byte_offset}` (`events.rs:67-76`).
  - There is **no author-kind, trailer or co-author field** anywhere in `events.rs`, the custom log format (`custom.rs:1-60`, which is `timestamp|user|A/M/D|path[|RRGGBB]`) or the git parser (`git --raw --pretty=format:user:%aN%n%ct`, per CLAUDE.md). A grep for "co-author" and "trailer" in `gource-sources` and `gource-core` found nothing.
  - **Consequence (inferred):** AI-assisted detection needs new source data. The options are a trailer column in the log, a `%(trailers)` format in `git log`, or a side table of user-to-style. The custom log's fifth column is already used for the file colour.

## 2. SceneView and the render contract (verified)

- **`UserView`** (`view.rs:163-171`) is `{pos: Vec2, size: Vec2, colour: Vec3, alpha: f32, texture: UserTexture}`. It is `Copy` plain data, filled in `extract_users` (`sim/extract.rs:331-337`) and reused each frame.
- **`UserTexture`** (`view.rs:154-159`) is an enum with one variant, `Default`. The builder ignores it: `builder.rs:293-302` always uses `TextureId::User` and `DEFAULT_TEXCOORD`. This is a ready-made extension slot.
- **User labels** are `LabelView`s anchored `WorldCentredAbove` at `pos.y - dims.y/2` (`user.rs:265`, `view.rs:227`).
- **Quad emission** (`builder.rs:290-303`).
  - Users go into `frame.quads`, one `quad()` per user (`emit.rs:27`).
  - Each quad has 4 `QuadVertex` (32 bytes: pos, rgba, uv), indexed as 2 triangles from a shared index buffer.
  - The pass order is in `builder.rs:343-408`. Draw commands are merged when the state matches and the ranges are contiguous (`emit.rs:96-108`):
    1. edge shadow, 2. edges, 3. file shadows, 4. action beams (Beam texture), 5. files, 6. **user shadow** (`Program::Shadow{0.5}`, `TextureId::User`, offset `2*user_scale`), 7. **users** (`Textured`, `User` texture), 8. bloom (additive).
  - The shadow command re-draws the same `users` quad range with the shadow shader. Names are drawn afterwards in the text pass.
- **Draw-call budget.** `docs/web-mvp/PERFORMANCE.md:178` says "Always 10 draw calls per frame (9 when a pass is empty)". The 10 are 8 scene commands plus 2 text commands. PLAN.md:66 says "about a dozen". There is headroom of about 2 calls.
- **Shadow shader.** It is `black * colour.a * tex.a * strength` (`shaders.rs:57-68`). It samples the bound texture's alpha. A decoration texture with alpha would cast a shadow if its quads are in the shadowed range, at no extra cost.
- **Textures** (`textures.rs:30-36`, `build.rs:15`).
  - Five textures exist: `white`, `file`, `user`, `beam` and `glyphs`.
  - The three PNGs are decoded at build time from `data/*.png` into RGBA8 and embedded in the wasm.
  - They are mipmapped, trilinear and clamp-to-edge (`textures.rs:96-118`).
  - `TextureId` is matched exhaustively in `backend.rs:306-312`. Adding a texture means a `TextureId` variant, a `Textures` field, a `build.rs` `TEXTURES` entry and one match arm.
  - `uv_scale` is `[1,1]` for every non-glyph texture (`backend.rs:264-268`).
- **Programs** (`shaders.rs`).
  - There are four (`Textured`, `Shadow`, `Bloom`, `Text`), in a fixed array indexed by `ProgramKind` (`shaders.rs:151-158`, `PROGRAM_COUNT = 4`).
  - Adding a procedural program means: a GLSL pair, a `ProgramKind` variant, a `Program` variant in `gource-render/src/lib.rs:81`, a `link()` call and one match arm in `backend.rs:~246`.
  - `Bloom` is the precedent: its frag derives the look from a texcoord packed with `(radius, centre.x, centre.y)`, with no texture (`builder.rs:316-323`, `shaders.rs:129-143`). It also uses a separate 40-byte vertex stream.
  - Vertex colour is clamped to [0,1] in the vertex shader (`shaders.rs:20-34`). This matters for a glow that wants HDR values.
- **No per-frame allocation.** The streams are reused `Vec`s and the GPU buffers only grow (`streams.rs:1-6`). New quads fit that rule as long as they are pushed into the existing `frame.quads`.
- The `UserView` struct literal appears in `gource-render/tests/common/mod.rs:85`, `builder/tests.rs:77` and `gource-cli/src/hash.rs:298`. All use `..UserView::default()`, so new fields with a default will not break them.

## 3. Options (verified)

- **Names and aliases.** `options.rs:50-148` is a name list and `canonical_name` (`options.rs:152-168`) holds the aliases. `Simulation::set_option(name, value: &str)` calls `Settings::apply_option` and then switches on the canonical name for live side effects (`sim.rs:456-505`, e.g. `user-scale` calls `RUser::set_scale`, `hash-seed` calls `apply_hash_seed`).
- **Value syntax** (`options.rs:5-48, 179-256`).
  - Booleans accept `true/1/yes` and so on, and an empty value means true.
  - Colours are `RRGGBB` or `vec3(r,g,b)`.
  - Multi-value options are newline-separated strings. Empty lines are dropped and the list replaces the old one.
  - `Ctx::regex_lines` validates every line, and `OptionError::InvalidValue{option,value,reason}` carries a message.
  - Validation is strict: numbers must parse fully, and an empty value resets to the default.
- **Restart vs live.** `effect_of` (`options.rs:171-176`) lists `file-filter`, `file-show-filter`, `user-filter`, `user-show-filter`, `start-date`, `stop-date`, `start-position`, `stop-position` and `rand-mode` as `Restart`. Everything else is `Live`. Style rules that change user identity would be `Live` if they only affect view fields.
- **Existing user list-valued options.** `highlight-user` and `follow-user` are plain name lists (`options.rs:652-653`). `user-filter` is a regex list. There is a **round-trip test** (`options.rs:904`, "every option value applies back unchanged") that any new option must satisfy.
- **The `set_option` path.** It is string-in, string-out all the way.
  - It goes from `web/src/screens/viewer.ts:217` to `gource_web.d.ts:75` (`set_option(name, value): string`) to `crates/gource-web/src/wasm.rs:166` (throws `JsError`) to `engine.rs:487` to `sim.set_option`.
  - So a structured value has to be a string. Two options fit the existing conventions.
    - **Line-per-rule text**, with one rule per line, mirroring the multi-value lists. For example `match=<regex-or-name>;cape=gold;glow=ff0` or a `|`-separated line like the custom log.
    - **A JSON string**, with its own validator.
  - **Inferred:** line-per-rule matches `Ctx::lines` and the web `ListDef` kind (`options.ts:48-52`, textarea editing) with the least new machinery.
- **Web side** (`web/src/options.ts`, `url-state.ts`, `ui/panel.ts`).
  - `OPTION_DEFS` is the single table. Kinds are `colour | range | bool | select | text | list`, each with a `section`, a `default` and a `hint`. `SECTIONS` has 4 ids: `look`, `playback`, `visibility` and `filters`.
  - `RESTART_OPTIONS` in `options.ts` is duplicated by hand from Rust and must stay in sync.
  - The URL uses option names as query keys (`url-state.ts:41-65`). Only keys in `OPTIONS_BY_NAME` are read and written, and only non-default values are serialised.
  - List options become repeated parameters, one per line (`url-state.ts:81-85`, parse at `:46-53`), with `%2C` and `%2F` unescaped for readability (`url-state.ts:90`).
  - A rule list would therefore round-trip as `?user-style=...&user-style=...` with no change to the URL code, provided each line avoids `\n`.
  - `panel.ts` builds controls from the kind (`build(def, section)`, `panel.ts:~88`). A new kind (for example a rule editor) means a new builder branch.
  - Panel changes go through `queueFrame` (one rAF batch) and `send` (`panel.ts:289-307`).
- **PLAN scope.** PLAN.md:50 puts "private repositories and tokens, ... video export, user avatars" out of scope for the MVP, so none of this exists. The MVP option list is at PLAN.md:236-246.

## 4. Existing per-user colour and image notions

- **C++** (`src/user.cpp:178-214`, `gource_settings.cpp:421-424, 755, 796-860`).
  - `--user-image-dir` maps `<name>.png|jpg|jpeg` (the extension stripped, lower-cased) to an image, kept in `user_image_map`.
  - `--default-user-image` and `--colour-images` (`colour_user_images`) also exist. Without `--colour-images`, `usercol` is forced to white when a custom image is used.
  - Each user holds its own `graphic` (texture), and `Gource::updateVBOs` (`gource.cpp:2255-2281`) adds the user to a **texture-batched** `user_vbo` using `user->graphic->textureid`. This is the C++ precedent for per-user texture variation.
  - Colour is always `colourHash(name)` mixed 60/40 with white. There is no per-user colour override, apart from white when a custom image is used.
- **Rust.** There are **none** of these. `user.rs:27` says "No avatars: every user has the `user.png` graphic". A grep for `user_image` in `crates/` finds only the C++ golden-test stubs (`crates/gource-core/tests/golden/sim/stubs/gource_settings.h`). Per-user colour is only the hash colour, and `UserTexture::Default` is the placeholder for the avatar hook. The one existing per-entity colour override is the file colour: the custom log's fifth column, `FileChange.colour: Option<[u8;3]>` (`events.rs:62`).
- **Inferred:** a style system should generalise "name → (texture, tint)" into "predicate on name/metadata → style record". The rule that user-images only match by exact name is what a rule system would generalise. The `colour_user_images` flag maps to a style's `tint: hash|fixed|none`.

## 5. Hashing for variants (verified)

- `string_hash(bytes, seed)` is bit-exact with C++ (`hash.rs:42-55`, signed-char quirk, wrapping). The `seed` is `Settings::hash_seed`, which users can change (default 31, a restart option in the web `RESTART_OPTIONS`; `sim.rs:1602-1617` reseeds on `S`).
- `vec2_hash`, `colour_hash` and `component(hash, d)` give `(hash/d) % 255` (`hash.rs:65-69`).
- **Deterministic keyed values.** `keyed_u64(seed, key, counter)` is a stateless splitmix64-based hash (`hash.rs:106-132`). `stream_key(bytes)` is a `const fn` FNV-1a key, so `keyed_u64(seed, stream_key(b"deco.variant"), stream_key(name))` is a pure, platform-stable value for picking a variant.
- **Caveat.** `mix64`, `stream_key` and `keyed_u64` are `pub(crate)` in `gource-core`, so a variant picker has to live in core (for example computed at `RUser::new`) rather than in the renderer.
- Using `colour_hash`'s own components for variant choice would couple variants to the hash seed, so a reseed would also reshuffle them. A separate key does not do that. That is a design decision for the spec.

## 6. Answers to the six questions

1. **Cheapest decoration path (inferred, grounded in 2).** Add decoration quads to the existing `frame.quads` stream after (or around) the user quads.
   - Use a new `TextureId::Deco` atlas texture, built at compile time like `file/user/beam` (new `TEXTURES` entry in `build.rs`) with a sprite sheet of cape, halo, hat and badge frames.
   - This adds about 2 draw calls (one per behind or front group, plus shadow if wanted), keeping the total at about 12. It allocates nothing new, because the quads are `extend_from_slice` into reused `Vec`s.
   - Quads are cheap: 4x32 bytes each.
   - A new pass or program is only needed for procedural effects, such as an animated aura. The `Bloom` stream and program (`builder.rs:306-329`) is the pattern: an additive pass, one quad per user, glow computed in the fragment shader. That costs one program, one `Program` variant and one draw call, but no texture.
   - **Recommendation direction:** sprite quads for cape, hat and badge; the existing additive path (or a tinted textured quad with additive blend) for glow and aura.
2. **Draw order and shadows.**
   - Current order is `user shadow → users → bloom`. A cape needs to go **before the avatar quad inside the user range**, and a hat, halo or badge **after it**.
   - Contiguity matters, because `push_command` only merges adjacent ranges (`emit.rs:96`). Interleaving per user would break merging and create about 2 commands per user.
   - So emit three batches in this order: `cape quads, user quads, front-deco quads`, each as its own range.
     - Shadow: one command over the cape and user ranges (`Program::Shadow`, texture varies, so two commands unless capes and avatar share an atlas).
     - Body: one command per texture.
     - Front: one for hats and badges.
   - If decorations share an atlas with the avatar (one combined `User` sheet), a shadow pass over all three ranges is one call, because the texture is the same.
   - Alpha: copy `u.alpha` onto every decoration quad, so fade-in and fade-out stay in step with the avatar.
   - Glow, as an additive pass, belongs after users and before or with bloom (additive bloom is last, `builder.rs:397-408`).
3. **Per-user data the SceneView needs.** The minimum is an extension of `UserView`. Keep it `Copy` and `Default`.
   - `style: u16` (an index into a style table; 0 means none)
   - `variant: u8` (hash-picked)
   - `facing/velocity: Vec2` or an angle, taken from `pawn.accel` for cape tilt (not currently exported; note it is damped and capped, so it is smooth)
   - an `active` or `beaming: bool`, or the action count, for a glow that pulses while working
   - Also, `SceneView` would need a **style table** (rules resolved to quad specs: texture rect, anchor offset in `dims` units, tint mode, blend) and a per-frame time (`pawn.elapsed`) for animation.
   - The renderer already receives `Settings`, so static style config can live there if it is not per-user.
4. **Options and validation.** See section 3. Following Gource style means kebab-case names (for example `user-style`, `ai-user-style`, `hide=decorations` using the existing comma list in `options.rs:336-356`).
   - Add the value to `Settings`.
   - Handle it in `options.rs` `apply`/`value`. Because `Ctx::lines` gives one rule per line, a rule parser returns `OptionError::InvalidValue` with a reason, so the panel can show the error (`panel.ts` `showError`).
   - Add a `sim.rs` set_option arm to refresh the per-user style after a change, as `user-scale` does.
   - Add an `OptionDef` entry of kind `list`.
   - The URL works as repeated `user-style` parameters at no extra cost.
   - The round-trip test requires `option_value` to emit exactly what `apply` accepts.
5. **Existing notion to generalise.** The C++ `user-image-dir` name-to-image map, `default-user-image` and `colour-images` (section 4). Rust has only the `UserTexture::Default` stub. The file colour column in the custom log is the one existing per-entity override.
6. **Hash-picked variants.** See section 5. Use `keyed_u64(hash_seed, stream_key(b"<purpose>"), stream_key(name))`, computed once at `RUser::new` and again in `apply_hash_seed` only if variants are meant to follow the seed. This needs the helper functions exposed within core. Cape colour and hat choice then agree between native runs, browser runs and cloud runs.

## 7. Risks and gaps to flag in the spec

- **No source signal for AI co-authorship.** Nothing in the parser, event log or server carries it (section 1). The git format string `%aN%n%ct` ignores trailers, so this needs a decision on where the flag comes from. It also changes the interchange format, which CLAUDE.md says "must stay supported".
- **Identity is the username string.** Co-authorship is a property of the **commit**, not the user. A user who is sometimes AI-assisted would need a state that is per-commit and decays, or one that sticks for the session, or a separate "AI actor" user. The spec has to pick one.
- **Clamping.** Vertex colours clamp at 1 (`shaders.rs:11-14`), so an HDR glow needs a separate program or the additive path.
- **Parity tests.** `docs/web-mvp/parity` and the golden tests compare the plain scene. Decorations must be off by default, or the frame hashes (`gource-cli/src/hash.rs`) change.
- **Private repos and recording are separate surfaces** and appear nowhere in the code today (PLAN.md:50 marks both out of scope). For recording, the only relevant hooks I verified are the deterministic `?fixed-dt` and `step(n)` mode (`engine.rs:145, 302-330`, CLAUDE.md "Page URLs") and the `ui_scale` / viewport on `SceneView` (`view.rs:282-285`).