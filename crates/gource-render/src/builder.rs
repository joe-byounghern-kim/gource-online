// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! [`FrameBuilder`]: `Gource::draw` turned into vertex streams and commands.
//!
//! Ported from `gource.cpp` (`draw`, `drawScene`, `updateVBOs`,
//! `updateAndDrawEdges`, `drawFileShadows`, `drawActions`, `drawFiles`,
//! `drawUserShadows`, `drawUsers`, `drawBloom`), `dirnode.cpp`
//! (`updateFilesVBO`, `updateBloomVBO`, `updateEdgeVBO`, `calcEdges`,
//! `drawDirName`), `action.cpp` (`RAction::drawToVBO`), and the name drawing
//! of `user.cpp` / `file.cpp`.
//!
//! Port additions: the AI decoration blocks around the avatars and the user
//! glow in the bloom stream (`deco.rs`, at most two extra commands), and
//! the agent key and AI filter line with the date (`agent_key.rs`, text
//! only).

use glam::{Mat4, Vec2, Vec3, Vec4};

use gource_core::settings::Settings;
use gource_core::view::{FontRole, LabelAnchor, LabelView, SceneView, project_with};

use crate::agent_key;
use crate::deco::{self, DecoCtx};
use crate::emit::{DEFAULT_TEXCOORD, bloom_quad, push_command, quad, quads_in, vertex};
use crate::key::FileKey;
use crate::spline::{EDGE_HALF_WIDTH, Spline, label_pos, normalise};
use crate::text::{Fonts, Mode, SHADOW_STRENGTH};
use crate::textbox;
use crate::{
    Blend, DrawCommand, Frame, GlyphSource, Program, QuadVertex, Space, Stream, TextureId,
};

/// `shadow_shader->setFloat("shadow_strength", 0.5)`.
const SHADOW: Program = Program::Shadow { strength: 0.5 };
/// `Gource::max_tick_rate`.
const MAX_TICK: f32 = 1.0 / 60.0;
/// `date_x_offset` only follows the date width when it moves by more than
/// this many pixels (`Gource::updateTime`).
const DATE_WOBBLE: i32 = 5;

/// Builds a [`Frame`] from a [`SceneView`] (`Gource::updateVBOs`,
/// `drawScene` and the text/HUD part of `Gource::draw`). Keeps HUD state that
/// animates over time (file key, date offset) and scratch buffers between
/// frames, so a steady-state build allocates nothing.
#[derive(Debug, Default)]
pub struct FrameBuilder {
    /// Projected `RDirNode::pos` per dir (`projected_pos`), device pixels.
    dir_screen: Vec<Vec2>,
    /// Projected `RDirNode::spos` per dir (`projected_spos`).
    dir_screen_spos: Vec<Vec2>,
    file_key: FileKey,
    /// `Gource::date_x_offset`, logical pixels.
    date_x_offset: i32,
}

/// A range of quads in one stream, half-open.
#[derive(Copy, Clone, Debug)]
struct Range {
    first: u32,
    end: u32,
}

impl Range {
    fn count(self) -> u32 {
        self.end - self.first
    }
}

fn command(
    stream: Stream,
    range: Range,
    program: Program,
    space: Space,
    offset: [f32; 2],
    texture: TextureId,
    blend: Blend,
) -> DrawCommand {
    DrawCommand {
        stream,
        first_quad: range.first,
        quad_count: range.count(),
        program,
        space,
        offset,
        texture,
        blend,
    }
}

/// Column-major `glOrtho(0, w, h, 0, -1, 1)`.
pub(crate) fn screen_ortho(w: f32, h: f32) -> [f32; 16] {
    let sx = if w > 0.0 { 2.0 / w } else { 0.0 };
    let sy = if h > 0.0 { -2.0 / h } else { 0.0 };
    [
        sx, 0.0, 0.0, 0.0, //
        0.0, sy, 0.0, 0.0, //
        0.0, 0.0, -1.0, 0.0, //
        -1.0, 1.0, 0.0, 1.0,
    ]
}

/// Records quad ranges as passes append to a stream.
struct Mark(usize);

impl Mark {
    fn at(v: &[QuadVertex]) -> Mark {
        Mark(v.len())
    }

    fn range(self, v: &[QuadVertex]) -> Range {
        Range {
            first: quads_in(self.0),
            end: quads_in(v.len()),
        }
    }
}

/// Per-frame constants shared by the passes.
struct Ctx<'a> {
    view: &'a SceneView,
    settings: &'a Settings,
    /// Device pixels per logical pixel.
    scale: f32,
    world_to_clip: Mat4,
    /// Logical display size (`display.width`, `display.height`).
    display: (i32, i32),
    fonts: Fonts,
    texel: [f32; 2],
}

impl Ctx<'_> {
    fn project(&self, p: Vec2) -> Vec2 {
        project_with(&self.world_to_clip, self.view.viewport, p.extend(0.0))
    }

    fn text_command(&self, frame: &mut Frame, range: Range, shadow_strength: f32) {
        push_command(
            &mut frame.commands,
            command(
                Stream::Text,
                range,
                Program::Text {
                    shadow_strength,
                    texel: self.texel,
                },
                Space::Screen,
                [0.0, 0.0],
                TextureId::Glyphs,
                Blend::Alpha,
            ),
        );
    }

    /// Untextured HUD quads (`glDisable(GL_TEXTURE_2D)`).
    fn hud_quads_command(&self, frame: &mut Frame, range: Range) {
        push_command(
            &mut frame.commands,
            command(
                Stream::Quads,
                range,
                Program::Textured,
                Space::Screen,
                [0.0, 0.0],
                TextureId::White,
                Blend::Alpha,
            ),
        );
    }
}

impl FrameBuilder {
    pub fn new() -> Self {
        FrameBuilder::default()
    }

    /// Fill `frame` for `view`.
    ///
    /// `dt` is the real frame time in seconds; the HUD animates with
    /// Gource's `min(dt, 1/60) × time_scale`, like `Gource::logic`.
    ///
    /// Reads from `view`: screen-space values (`TextBoxView::pos`) are device
    /// pixels; `WorldCentredAbove` anchors are the user's text point
    /// (`pos.y - dims.y / 2` already applied); the file key follows
    /// `view.hud.show_key`. Hide flags are honoured from `settings` even if
    /// the view still carries the hidden entities.
    pub fn build(
        &mut self,
        view: &SceneView,
        settings: &Settings,
        dt: f32,
        glyphs: &mut dyn GlyphSource,
        frame: &mut Frame,
    ) {
        glyphs.begin_frame();
        frame.clear();
        let vp = view.viewport;
        frame.viewport = [vp.x.max(0.0) as u32, vp.y.max(0.0) as u32];
        frame.clear_colour = [view.background.x, view.background.y, view.background.z, 1.0];
        let world_to_clip = view.camera.world_to_clip(vp);
        frame.world_to_clip = world_to_clip.to_cols_array();
        frame.screen_to_clip = screen_ortho(vp.x, vp.y);

        let scale = if view.ui_scale > 0.0 && view.ui_scale.is_finite() {
            view.ui_scale
        } else {
            1.0
        };
        let dt = if dt.is_finite() {
            dt.clamp(0.0, MAX_TICK)
        } else {
            0.0
        } * settings.time_scale;

        let ctx = Ctx {
            view,
            settings,
            scale,
            world_to_clip,
            display: ((vp.x / scale).round() as i32, (vp.y / scale).round() as i32),
            fonts: Fonts::new(settings, scale, glyphs),
            texel: glyphs.texel_size(),
        };

        // calcScreenPos for every dir node.
        self.dir_screen.clear();
        self.dir_screen_spos.clear();
        for d in &view.dirs {
            self.dir_screen.push(ctx.project(d.pos));
            self.dir_screen_spos.push(ctx.project(d.spos));
        }

        self.scene(&ctx, frame);
        self.names(&ctx, glyphs, frame);
        self.hud(&ctx, glyphs, dt, frame);
    }

    /// `updateVBOs` + `drawScene`.
    fn scene(&self, ctx: &Ctx, frame: &mut Frame) {
        let view = ctx.view;
        let s = ctx.settings;

        // Edges: calcEdges + updateEdgeVBO, screen space.
        let m = Mark::at(&frame.quads);
        if !s.hide_tree {
            let half_width = EDGE_HALF_WIDTH * ctx.scale;
            for (i, d) in view.dirs.iter().enumerate() {
                let Some(p) = d.parent.map(|p| p as usize) else {
                    continue;
                };
                if !d.draw_edge || p >= view.dirs.len() {
                    continue;
                }
                Spline::new(
                    self.dir_screen[p],
                    view.dirs[p].colour,
                    self.dir_screen[i],
                    d.colour,
                    self.dir_screen_spos[i],
                )
                .push_quads(half_width, &mut frame.quads);
            }
        }
        let edges = m.range(&frame.quads);

        // updateFilesVBO.
        let m = Mark::at(&frame.quads);
        if !s.hide_files {
            for f in &view.files {
                quad(
                    &mut frame.quads,
                    f.pos - f.size * 0.5,
                    f.size,
                    f.colour.extend(f.alpha),
                    DEFAULT_TEXCOORD,
                );
            }
        }
        let files = m.range(&frame.quads);

        // updateActionsVBO (RAction::drawToVBO).
        let m = Mark::at(&frame.quads);
        if !s.hide_users {
            for a in &view.actions {
                action_quad(
                    &mut frame.quads,
                    a.src,
                    a.dest,
                    a.target_size,
                    a.colour,
                    a.progress,
                );
            }
        }
        let actions = m.range(&frame.quads);

        // Decorations ride with the user quads: the back block, then the
        // avatars (user_vbo, contiguous), then the front block (AI-52).
        let decorate = !s.hide_users && !s.hide_decorations;
        let dctx = DecoCtx {
            view,
            world_to_clip: ctx.world_to_clip,
            ui_scale: ctx.scale,
        };
        let m = Mark::at(&frame.quads);
        if decorate {
            deco::back_block(&dctx, &mut frame.quads);
        }
        let back = m.range(&frame.quads);

        // user_vbo.
        let m = Mark::at(&frame.quads);
        if !s.hide_users {
            for u in &view.users {
                quad(
                    &mut frame.quads,
                    u.pos - u.size * 0.5,
                    u.size,
                    u.colour.extend(u.alpha),
                    DEFAULT_TEXCOORD,
                );
            }
        }
        let users = m.range(&frame.quads);

        let m = Mark::at(&frame.quads);
        if decorate {
            deco::front_block(&dctx, &mut frame.quads);
        }
        let front = m.range(&frame.quads);

        // updateBloomVBO.
        let bloom_first = frame.bloom.len() / 4;
        if !s.hide_bloom {
            let multiplier = s.bloom_multiplier as f64;
            for d in &view.dirs {
                if !d.draw_bloom {
                    continue;
                }
                let radius = (d.radius as f64 * 2.0 * multiplier) as f32;
                let diameter = (radius as f64 * 2.0) as f32;
                let col = d.colour * s.bloom_intensity;
                let dims = Vec2::splat(diameter);
                bloom_quad(
                    &mut frame.bloom,
                    d.pos - dims * 0.5,
                    dims,
                    Vec4::new(col.x, col.y, col.z, 1.0),
                    [radius, d.pos.x, d.pos.y, 0.0],
                );
            }
        }
        // User glow follows hide=decorations, not hide=bloom (AI-58).
        if decorate {
            deco::glow(view, &mut frame.bloom);
        }
        let bloom = Range {
            first: bloom_first as u32,
            end: (frame.bloom.len() / 4) as u32,
        };

        let cmds = &mut frame.commands;
        let quads = |range, program, space, offset, texture| {
            command(
                Stream::Quads,
                range,
                program,
                space,
                offset,
                texture,
                Blend::Alpha,
            )
        };
        let edge_offset = [2.0 * ctx.scale, 2.0 * ctx.scale];
        push_command(
            cmds,
            quads(edges, SHADOW, Space::Screen, edge_offset, TextureId::Beam),
        );
        push_command(
            cmds,
            quads(
                edges,
                Program::Textured,
                Space::Screen,
                [0.0, 0.0],
                TextureId::Beam,
            ),
        );
        push_command(
            cmds,
            quads(files, SHADOW, Space::World, [2.0, 2.0], TextureId::File),
        );
        push_command(
            cmds,
            quads(
                actions,
                Program::Textured,
                Space::World,
                [0.0, 0.0],
                TextureId::Beam,
            ),
        );
        push_command(
            cmds,
            quads(
                files,
                Program::Textured,
                Space::World,
                [0.0, 0.0],
                TextureId::File,
            ),
        );
        let user_offset = [2.0 * s.user_scale, 2.0 * s.user_scale];
        push_command(
            cmds,
            quads(users, SHADOW, Space::World, user_offset, TextureId::User),
        );
        push_command(
            cmds,
            quads(
                back,
                Program::Textured,
                Space::World,
                [0.0, 0.0],
                TextureId::Deco,
            ),
        );
        push_command(
            cmds,
            quads(
                users,
                Program::Textured,
                Space::World,
                [0.0, 0.0],
                TextureId::User,
            ),
        );
        push_command(
            cmds,
            quads(
                front,
                Program::Textured,
                Space::World,
                [0.0, 0.0],
                TextureId::Deco,
            ),
        );
        push_command(
            cmds,
            command(
                Stream::Bloom,
                bloom,
                Program::Bloom,
                Space::World,
                [0.0, 0.0],
                TextureId::White,
                Blend::Additive,
            ),
        );
    }

    /// The name pass: dir, file and user names through the font buffer (text
    /// shader shadow), then the selected user/file names drawn directly.
    fn names(&self, ctx: &Ctx, glyphs: &mut dyn GlyphSource, frame: &mut Frame) {
        let m = Mark::at(&frame.text);
        for label in &ctx.view.labels {
            if !is_selected(label.font) {
                self.draw_label(ctx, glyphs, label, Mode::Buffered, &mut frame.text);
            }
        }
        let buffered = m.range(&frame.text);
        ctx.text_command(frame, buffered, SHADOW_STRENGTH);

        let m = Mark::at(&frame.text);
        for label in &ctx.view.labels {
            if is_selected(label.font) {
                self.draw_label(ctx, glyphs, label, Mode::Immediate, &mut frame.text);
            }
        }
        let selected = m.range(&frame.text);
        ctx.text_command(frame, selected, 0.0);
    }

    fn draw_label(
        &self,
        ctx: &Ctx,
        glyphs: &mut dyn GlyphSource,
        label: &LabelView,
        mode: Mode,
        out: &mut Vec<QuadVertex>,
    ) {
        let s = ctx.settings;
        let hidden = match label.font {
            FontRole::DirName => s.hide_dirnames,
            FontRole::FileName => s.hide_filenames || s.hide_files,
            FontRole::UserName => s.hide_usernames || s.hide_users,
            _ => false,
        };
        if hidden || label.text.is_empty() || label.colour.w <= 0.0 {
            return;
        }
        let text = ctx.view.str(label.text);
        let font = ctx.fonts.get(label.font);
        let inv = 1.0 / ctx.scale;

        let pos = match label.anchor {
            LabelAnchor::Dir { dir } => {
                let i = dir as usize;
                let Some(p) = ctx.view.dirs.get(i).and_then(|d| d.parent) else {
                    return;
                };
                let p = p as usize;
                if p >= self.dir_screen.len() {
                    return;
                }
                label_pos(
                    self.dir_screen[p],
                    self.dir_screen[i],
                    self.dir_screen_spos[i],
                    s.dir_name_position,
                ) * inv
            }
            LabelAnchor::WorldTopLeft { pos } => ctx.project(pos) * inv,
            LabelAnchor::WorldCentredAbove { pos } => {
                // RUser::calcScreenPos.
                let p = ctx.project(pos) * inv;
                let width = font.width(glyphs, text);
                Vec2::new(p.x - width * 0.5, p.y - font.max_height())
            }
        };
        font.draw(glyphs, out, mode, pos.x, pos.y, text, label.colour);
    }

    /// The 2D overlay after the scene: splash, date, title, file key, text
    /// box, message.
    fn hud(&mut self, ctx: &Ctx, glyphs: &mut dyn GlyphSource, dt: f32, frame: &mut Frame) {
        let view = ctx.view;
        let hud = &view.hud;
        let s = ctx.settings;
        let scale = ctx.scale;
        let (dw, dh) = ctx.display;
        let white = Vec4::ONE;

        // Splash.
        if hud.splash > 0.0 {
            let fs = s.font_scale;
            let large = &ctx.fonts.large;
            let small = &ctx.fonts.small;
            let logowidth = large.width(glyphs, "Gource") as i32;
            let logoheight = (100.0 * fs) as i32;
            let cwidth = small.width(glyphs, SPLASH_LINE_1) as i32;
            let awidth = small.width(glyphs, SPLASH_LINE_2) as i32;
            let corner_y = (dh / 2) as f32 - 40.0 * fs;

            let m = Mark::at(&frame.quads);
            let colour = Vec4::new(0.0, 0.5, 1.0, hud.splash * 0.015);
            let (y0, y1, w) = (corner_y, corner_y + logoheight as f32, dw as f32);
            frame.quads.extend_from_slice(&[
                vertex(Vec2::new(0.0, y0) * scale, colour, [0.0, 0.0]),
                vertex(Vec2::new(0.0, y1) * scale, colour, [0.0, 0.0]),
                vertex(Vec2::new(w, y1) * scale, colour, [0.0, 0.0]),
                vertex(Vec2::new(w, y0) * scale, colour, [0.0, 0.0]),
            ]);
            let r = m.range(&frame.quads);
            ctx.hud_quads_command(frame, r);

            let m = Mark::at(&frame.text);
            let cx = (dw / 2) as f32;
            let cy = (dh / 2) as f32;
            let t = &mut frame.text;
            large.draw(
                glyphs,
                t,
                Mode::Immediate,
                (dw / 2 - logowidth / 2) as f32,
                cy - 30.0 * fs,
                "Gource",
                white,
            );
            small.draw(
                glyphs,
                t,
                Mode::Immediate,
                cx - (cwidth / 2) as f32,
                cy + 10.0 * fs,
                SPLASH_LINE_1,
                white,
            );
            small.draw(
                glyphs,
                t,
                Mode::Immediate,
                cx - (awidth / 2) as f32,
                cy + 30.0 * fs,
                SPLASH_LINE_2,
                white,
            );
            let r = m.range(&frame.text);
            ctx.text_command(frame, r, 0.0);
        }

        // Date and title (fontmedium).
        let medium = ctx.fonts.medium;
        let font_colour = s.font_colour.extend(1.0);
        let m = Mark::at(&frame.text);
        if let Some(date) = hud.date.filter(|d| !d.is_empty()) {
            let date = view.str(date);
            // Gource::updateTime.
            let date_offset = ((medium.width(glyphs, date) as i32) as f64 * 0.5) as i32;
            if (self.date_x_offset - date_offset).abs() > DATE_WOBBLE {
                self.date_x_offset = date_offset;
            }
            if !s.hide_date {
                let x = (dw / 2 - self.date_x_offset) as f32;
                medium.draw(
                    glyphs,
                    &mut frame.text,
                    Mode::Immediate,
                    x,
                    20.0,
                    date,
                    font_colour,
                );
            }
        }
        if let Some(title) = hud.title.filter(|t| !t.is_empty()) {
            medium.with_align_top(false).draw(
                glyphs,
                &mut frame.text,
                Mode::Immediate,
                10.0,
                (dh - 10) as f32,
                view.str(title),
                font_colour,
            );
        }
        // AI filter line under the date, or in its place (AI-74), and the
        // agent key at the top right (AI-59): text only.
        let row_height = s.scaled_font_size() as f32 + 6.0;
        if let Some(note) = hud.filter_note.filter(|n| !n.is_empty()) {
            let note = view.str(note);
            let date_drawn = !s.hide_date && hud.date.is_some_and(|d| !d.is_empty());
            let y = agent_key::DATE_Y + if date_drawn { row_height } else { 0.0 };
            let x = ((dw / 2) as f32 - medium.width(glyphs, note) * 0.5).round();
            medium.draw(
                glyphs,
                &mut frame.text,
                Mode::Immediate,
                x,
                y,
                note,
                font_colour,
            );
        }
        // PR meanings and coverage share the existing date/HUD text command.
        // No new texture, shader, quad pass or remote font is introduced.
        let pr_count =
            hud.pr_coverage.len() + usize::from(hud.pr_flow.is_some()) + hud.pr_key.len();
        if pr_count > 0 {
            let has_coverage = !hud.pr_coverage.is_empty() || hud.pr_flow.is_some();
            let gap_rows = usize::from(has_coverage && !hud.pr_key.is_empty());
            let date_height = if !s.hide_date && hud.date.is_some() {
                medium.max_height()
            } else {
                0.0
            };
            let filter_height = if hud.filter_note.is_some() {
                row_height
            } else {
                0.0
            };
            // The rich-only text block stays below the page's keyboard hint.
            // Coordinates here are logical HUD pixels, before UI-scale/DPR.
            // Keep ordinary date/key placement and plain frames unchanged.
            let mut top = (30.0 + date_height + filter_height).max(108.0);
            let bottom = dh as f32
                - if hud.title.is_some() {
                    medium.max_height() + 20.0
                } else {
                    30.0
                };
            let full_width = (dw as f32 - 21.0).max(1.0);
            let agent_left = agent_key::left_edge(view, &ctx.fonts.key, glyphs, dw as f32);
            let mut width =
                agent_left.map_or(full_width, |left| full_width.min((left - 22.0).max(1.0)));
            if agent_left.is_some() {
                let smallest = crate::text::Font::grab(glyphs, 1, ctx.fonts.key.scale);
                let column_fits = hud
                    .pr_coverage
                    .iter()
                    .chain(hud.pr_flow.iter())
                    .chain(hud.pr_key.iter())
                    .all(|&range| smallest.width(glyphs, view.str(range)) <= width);
                if !column_fits {
                    // Very long F1 names can leave no usable left column. Keep
                    // F1 fixed and put the rich-only block below its last row.
                    width = full_width;
                    let agents = hud.agents.len() + usize::from(hud.agents_more > 0);
                    top = top.max(
                        agent_key::KEY_TOP
                            + row_height * agents as f32
                            + ctx.fonts.key.max_height()
                            + 6.0,
                    );
                }
            }
            let available = (bottom - top).max(0.0);
            // These short recordable meanings have their own bounded HUD size.
            // Reduce it only when measured width/height requires it, so changing
            // the scene font cannot make coverage and key rows collide.
            let mut size = ctx.fonts.key.size.clamp(1, 12);
            let (pr_font, pr_row, fits_width) = loop {
                let font = crate::text::Font::grab(glyphs, size, ctx.fonts.key.scale)
                    .with_shadow(true)
                    .with_round(true);
                let row = font.max_height() + 4.0;
                let fits_width = hud
                    .pr_coverage
                    .iter()
                    .chain(hud.pr_flow.iter())
                    .chain(hud.pr_key.iter())
                    .all(|&range| font.width(glyphs, view.str(range)) <= width);
                if size == 1 || (fits_width && row * (pr_count + gap_rows) as f32 <= available) {
                    break (font, row, fits_width);
                }
                size -= 1;
            };
            // An impossibly small region must not overwrite F1 or emit text
            // beyond the canvas. Lifecycle/coverage truth remains unchanged.
            if fits_width && pr_row * (pr_count + gap_rows) as f32 <= available {
                let mut coverage_y = top;
                for &range in hud.pr_coverage.iter().chain(hud.pr_flow.iter()) {
                    pr_font.draw(
                        glyphs,
                        &mut frame.text,
                        Mode::Immediate,
                        10.0,
                        coverage_y,
                        view.str(range),
                        font_colour,
                    );
                    coverage_y += pr_row;
                }
                // Keep the recordable legend with coverage, not behind the host's
                // fixed bottom player. The measured fit above still bounds all rows.
                let mut key_y = coverage_y + pr_row * gap_rows as f32;
                for &range in &hud.pr_key {
                    pr_font.draw(
                        glyphs,
                        &mut frame.text,
                        Mode::Immediate,
                        10.0,
                        key_y,
                        view.str(range),
                        font_colour,
                    );
                    key_y += pr_row;
                }
            }
        }
        agent_key::draw(
            view,
            &ctx.fonts.key,
            glyphs,
            dw as f32,
            row_height,
            font_colour,
            &mut frame.text,
        );
        let r = m.range(&frame.text);
        ctx.text_command(frame, r, 0.0);

        // File key: Gource::logic runs file_key.logic(dt) every frame, then
        // draw() draws it and calls setShow(show_key).
        let key_font = ctx.fonts.key;
        self.file_key.sync(
            view,
            &hud.extensions,
            &key_font,
            glyphs,
            s.font_scale,
            s.scaled_font_size(),
        );
        self.file_key.logic(dt, dh, s.scaled_font_size());
        let qm = Mark::at(&frame.quads);
        let tm = Mark::at(&frame.text);
        self.file_key
            .draw(&key_font, glyphs, scale, &mut frame.quads, &mut frame.text);
        let (qr, tr) = (qm.range(&frame.quads), tm.range(&frame.text));
        ctx.hud_quads_command(frame, qr);
        ctx.text_command(frame, tr, 0.0);
        self.file_key.set_show(hud.show_key);

        // Text box.
        if let Some(tb) = hud.textbox.as_ref().filter(|t| !t.lines.is_empty()) {
            let font = &ctx.fonts.text_box;
            let l = textbox::layout(view, tb, tb.pos / scale, font, glyphs, ctx.display);
            let qm = Mark::at(&frame.quads);
            let tm = Mark::at(&frame.text);
            textbox::draw(
                view,
                tb,
                &l,
                font,
                glyphs,
                scale,
                &mut frame.quads,
                &mut frame.text,
            );
            let (qr, tr) = (qm.range(&frame.quads), tm.range(&frame.text));
            ctx.hud_quads_command(frame, qr);
            ctx.text_command(frame, tr, 0.0);
        }

        // Message (font, at (1, 3)).
        if let Some((msg, remaining)) = hud.message
            && remaining > 0.0
            && !msg.is_empty()
        {
            let m = Mark::at(&frame.text);
            ctx.fonts.small.draw(
                glyphs,
                &mut frame.text,
                Mode::Immediate,
                1.0,
                3.0,
                view.str(msg),
                white,
            );
            let r = m.range(&frame.text);
            ctx.text_command(frame, r, 0.0);
        }
    }
}

const SPLASH_LINE_1: &str = "Software Version Control Visualization";
const SPLASH_LINE_2: &str = "(C) 2009 Andrew Caudwell";

fn is_selected(role: FontRole) -> bool {
    matches!(role, FontRole::FileSelected | FontRole::UserSelected)
}

/// `RAction::drawToVBO`: a beam from the user (narrow, faint) to the file.
pub(crate) fn action_quad(
    out: &mut Vec<QuadVertex>,
    src: Vec2,
    dest: Vec2,
    target_size: f32,
    colour: Vec3,
    progress: f32,
) {
    let n = normalise(dest - src);
    let perp = Vec2::new(-n.y, n.x);
    let offset = perp * target_size * 0.5;
    let offset_src = offset * 0.3;

    let alpha = (1.0 - progress as f64) as f32;
    let alpha2 = (alpha as f64 * 0.1) as f32;
    let col1 = colour.extend(alpha);
    let col2 = colour.extend(alpha2);

    out.extend_from_slice(&[
        vertex(src - offset_src, col2, [0.0, 0.0]),
        vertex(src + offset_src, col2, [0.0, 1.0]),
        vertex(dest + offset, col1, [1.0, 1.0]),
        vertex(dest - offset, col1, [1.0, 0.0]),
    ]);
}

#[cfg(test)]
mod tests;
