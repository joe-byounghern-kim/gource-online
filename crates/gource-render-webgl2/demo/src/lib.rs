// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Verification page for `gource-render-webgl2`: builds a synthetic
//! [`Frame`] by hand that exercises every program, blend mode and stream
//! (edges with shadows, file and user sprites with shadows, action beams,
//! bloom glows and several text labels in three sizes) and draws it on every
//! animation frame. The geometry follows the C++ emitters (`SplineEdge`,
//! `RFile`, `RUser`, `RAction::drawToVBO`, `RDirNode::updateBloomVBO`,
//! `FXGlyph::drawToVBO`) closely enough to look like Gource, but it is not the
//! real `FrameBuilder`.

use std::cell::RefCell;
use std::f32::consts::PI;
use std::rc::Rc;

use gource_render::gource_core::glam::{Vec2, Vec3};
use gource_render::gource_core::view::{CameraView, project_with};
use gource_render::{
    Backend, BackendStats, Blend, BloomVertex, DrawCommand, FontKey, Frame, GlyphSource, Program,
    QuadVertex, Space, Stream, TextureId,
};
use gource_render_webgl2::WebGl2Backend;
use wasm_bindgen::prelude::*;
use web_sys::{HtmlCanvasElement, HtmlElement};

struct Dir {
    pos: Vec2,
    parent: Option<usize>,
    name: &'static str,
    files: &'static [(&'static str, [f32; 3])],
}

const RS: [f32; 3] = [0.87, 0.48, 0.28];
const MD: [f32; 3] = [0.35, 0.75, 0.95];
const TOML: [f32; 3] = [0.65, 0.85, 0.35];
const PNG: [f32; 3] = [0.85, 0.45, 0.85];
const GLSL: [f32; 3] = [0.95, 0.85, 0.30];

const DIRS: [Dir; 6] = [
    Dir {
        pos: Vec2::new(0.0, 0.0),
        parent: None,
        name: "",
        files: &[
            ("README.md", MD),
            ("Cargo.toml", TOML),
            ("LICENSE", [1.0; 3]),
        ],
    },
    Dir {
        pos: Vec2::new(-150.0, -90.0),
        parent: Some(0),
        name: "src",
        files: &[
            ("main.rs", RS),
            ("lib.rs", RS),
            ("dirnode.rs", RS),
            ("file.rs", RS),
            ("user.rs", RS),
            ("action.rs", RS),
            ("camera.rs", RS),
            ("spline.rs", RS),
        ],
    },
    Dir {
        pos: Vec2::new(160.0, -70.0),
        parent: Some(0),
        name: "data",
        files: &[
            ("file.png", PNG),
            ("user.png", PNG),
            ("beam.png", PNG),
            ("bloom.tga", PNG),
        ],
    },
    Dir {
        pos: Vec2::new(40.0, 150.0),
        parent: Some(0),
        name: "docs",
        files: &[("PLAN.md", MD), ("RESULTS.md", MD)],
    },
    Dir {
        pos: Vec2::new(-250.0, 40.0),
        parent: Some(1),
        name: "core",
        files: &[
            ("quadtree.rs", RS),
            ("bounds.rs", RS),
            ("hash.rs", RS),
            ("regex.rs", RS),
            ("vectors.rs", RS),
        ],
    },
    Dir {
        pos: Vec2::new(270.0, 70.0),
        parent: Some(2),
        name: "shaders",
        files: &[
            ("bloom.frag", GLSL),
            ("text.frag", GLSL),
            ("shadow.frag", GLSL),
        ],
    },
];

struct User {
    name: &'static str,
    colour: [f32; 3],
    orbit: Vec2,
    targets: &'static [(usize, usize, [f32; 3])],
}

const CREATE: [f32; 3] = [0.0, 1.0, 0.0];
const MODIFY: [f32; 3] = [1.0, 0.7, 0.3];
const REMOVE: [f32; 3] = [1.0, 0.0, 0.0];

const USERS: [User; 3] = [
    User {
        name: "Andrew Caudwell",
        colour: [0.55, 0.75, 1.0],
        orbit: Vec2::new(-90.0, 30.0),
        targets: &[(1, 0, MODIFY), (1, 3, MODIFY), (4, 1, CREATE)],
    },
    User {
        name: "Ferris",
        colour: [1.0, 0.6, 0.45],
        orbit: Vec2::new(150.0, 40.0),
        targets: &[(5, 0, CREATE), (2, 1, REMOVE)],
    },
    User {
        name: "Gource Bot",
        colour: [0.7, 1.0, 0.6],
        orbit: Vec2::new(-20.0, 110.0),
        targets: &[(3, 0, MODIFY)],
    },
];

/// Gource's file ring layout (`RDirNode::calcFileDest`): one file at the
/// centre, ring `k` at `k * diameter` holding `floor((k + 1) * π)` files.
fn file_offset(index: usize, diameter: f32) -> Vec2 {
    if index == 0 {
        return Vec2::ZERO;
    }
    let (mut ring, mut first) = (1usize, 1usize);
    loop {
        let count = ((ring + 1) as f32 * PI) as usize;
        if index < first + count {
            let a = (index - first) as f32 / count as f32 * 2.0 * PI;
            return Vec2::new(a.cos(), a.sin()) * ring as f32 * diameter;
        }
        first += count;
        ring += 1;
    }
}

fn file_pos(dir: usize, file: usize) -> Vec2 {
    DIRS[dir].pos + file_offset(file, 8.0)
}

fn dir_radius(dir: usize) -> f32 {
    let area = DIRS[dir].files.len() as f32 * PI * 16.0;
    area.sqrt().max(1.0) * 1.5
}

fn dir_colour(dir: usize) -> [f32; 3] {
    let files = DIRS[dir].files;
    let mut c = [0.0; 3];
    for (_, col) in files {
        for i in 0..3 {
            c[i] += col[i] / files.len() as f32;
        }
    }
    c
}

fn quad(buf: &mut Vec<QuadVertex>, pos: Vec2, dims: Vec2, colour: [f32; 4], uv: [f32; 4]) {
    let v = |p: Vec2, u: f32, w: f32| QuadVertex {
        pos: p.into(),
        colour,
        uv: [u, w],
    };
    buf.push(v(pos, uv[0], uv[1]));
    buf.push(v(pos + Vec2::new(dims.x, 0.0), uv[2], uv[1]));
    buf.push(v(pos + dims, uv[2], uv[3]));
    buf.push(v(pos + Vec2::new(0.0, dims.y), uv[0], uv[3]));
}

/// Push an alpha-blended command for the quads added since vertex `start`.
fn quads_command(
    frame: &mut Frame,
    start: usize,
    program: Program,
    space: Space,
    offset: [f32; 2],
    texture: TextureId,
) {
    frame.commands.push(DrawCommand {
        stream: Stream::Quads,
        first_quad: (start / 4) as u32,
        quad_count: ((frame.quads.len() - start) / 4) as u32,
        program,
        space,
        offset,
        texture,
        blend: Blend::Alpha,
    });
}

/// `FXFont::draw` with `align_top` and rounding, appending glyph quads.
fn text(
    frame: &mut Frame,
    glyphs: &mut dyn GlyphSource,
    size: u16,
    mut pos: Vec2,
    s: &str,
    colour: [f32; 4],
    centre: bool,
) {
    let font = FontKey { size_px: size };
    let m = glyphs.metrics(font);
    if centre {
        let width: f32 = s.chars().map(|c| glyphs.glyph(font, c).advance).sum();
        pos.x -= width * 0.5;
    }
    pos.y += (m.ascender + m.descender).ceil();
    let mut pen = pos.round();
    for ch in s.chars() {
        let g = glyphs.glyph(font, ch);
        quad(
            &mut frame.text,
            pen + Vec2::from(g.bearing),
            g.size.into(),
            colour,
            [g.uv_min[0], g.uv_min[1], g.uv_max[0], g.uv_max[1]],
        );
        pen.x += g.advance;
    }
}

struct Demo {
    backend: WebGl2Backend,
    canvas: HtmlCanvasElement,
    stats_el: Option<HtmlElement>,
    frame: Frame,
    frames: u32,
    start_ms: f64,
    gl_errors: u32,
    background: [f32; 4],
    atlas_stress: u16,
}

impl Demo {
    fn resize(&mut self) {
        let window = web_sys::window().unwrap();
        let dpr = window.device_pixel_ratio();
        let w = (self.canvas.client_width() as f64 * dpr).round() as u32;
        let h = (self.canvas.client_height() as f64 * dpr).round() as u32;
        if [w, h] != self.frame.viewport {
            self.backend.resize(w, h);
            self.frame.viewport = [w.max(1), h.max(1)];
        }
    }

    fn build(&mut self, t: f32) {
        let frame = &mut self.frame;
        frame.clear();
        let vp = Vec2::new(frame.viewport[0] as f32, frame.viewport[1] as f32);
        frame.clear_colour = self.background;
        let camera = CameraView::default();
        let world = camera.world_to_clip(vp);
        frame.world_to_clip = world.to_cols_array();
        frame.screen_to_clip = [
            2.0 / vp.x,
            0.0,
            0.0,
            0.0,
            0.0,
            -2.0 / vp.y,
            0.0,
            0.0,
            0.0,
            0.0,
            -1.0,
            0.0,
            -1.0,
            1.0,
            0.0,
            1.0,
        ];
        let project = |p: Vec2| project_with(&world, vp, Vec3::new(p.x, p.y, 0.0));
        let ui = (vp.y / 768.0).max(1.0);

        // 1. Edges: screen-space quadratic splines, 2.5 px half-width.
        let start = frame.quads.len();
        for (i, d) in DIRS.iter().enumerate() {
            let Some(p) = d.parent else { continue };
            let (pos1, pos2) = (project(d.pos), project(DIRS[p].pos));
            let mid = (DIRS[p].pos + d.pos) * 0.5;
            let bend = (d.pos - DIRS[p].pos).perp().normalize() * 25.0;
            let spos = project(mid + bend);
            let (c1, c2) = (dir_colour(i), dir_colour(p));
            const DETAIL: usize = 10;
            let point = |t: f32| {
                (pos1 * t + spos * (1.0 - t)) * t + (spos * t + pos2 * (1.0 - t)) * (1.0 - t)
            };
            for s in 0..DETAIL {
                let (ta, tb) = (s as f32 / DETAIL as f32, (s + 1) as f32 / DETAIL as f32);
                let (a, b) = (point(ta), point(tb));
                let perp = (a - b).perp().normalize() * 2.5 * ui;
                let lerp = |t: f32| {
                    let mut c = [0.0; 4];
                    for k in 0..3 {
                        c[k] = c1[k] * t + c2[k] * (1.0 - t);
                    }
                    c[3] = 0.6 * t + 0.3 * (1.0 - t);
                    c
                };
                let (ca, cb) = (lerp(ta), lerp(tb));
                frame.quads.push(QuadVertex {
                    pos: (a + perp).into(),
                    colour: ca,
                    uv: [1.0, 0.0],
                });
                frame.quads.push(QuadVertex {
                    pos: (a - perp).into(),
                    colour: ca,
                    uv: [0.0, 0.0],
                });
                frame.quads.push(QuadVertex {
                    pos: (b - perp).into(),
                    colour: cb,
                    uv: [0.0, 0.0],
                });
                frame.quads.push(QuadVertex {
                    pos: (b + perp).into(),
                    colour: cb,
                    uv: [1.0, 0.0],
                });
            }
        }
        let edge_offset = [2.0 * ui, 2.0 * ui];
        quads_command(
            frame,
            start,
            Program::Shadow { strength: 0.5 },
            Space::Screen,
            edge_offset,
            TextureId::Beam,
        );
        let edges = *frame.commands.last().unwrap();
        frame.commands.push(DrawCommand {
            program: Program::Textured,
            offset: [0.0, 0.0],
            ..edges
        });

        // 2. Files (shared by the shadow and the sprite pass).
        let start = frame.quads.len();
        for (di, d) in DIRS.iter().enumerate() {
            for (fi, (_, c)) in d.files.iter().enumerate() {
                let p = file_pos(di, fi);
                quad(
                    &mut frame.quads,
                    p - Vec2::splat(4.0),
                    Vec2::splat(8.0),
                    [c[0], c[1], c[2], 1.0],
                    [0.0, 0.0, 1.0, 1.0],
                );
            }
        }
        quads_command(
            frame,
            start,
            Program::Shadow { strength: 0.5 },
            Space::World,
            [2.0, 2.0],
            TextureId::File,
        );
        let files = *frame.commands.last().unwrap();

        // 3. Action beams (RAction::drawToVBO), progress cycling with time.
        let user_pos = |u: &User, k: usize| {
            let a = t * 0.4 + k as f32 * 2.1;
            u.orbit + Vec2::new(a.cos(), a.sin()) * 30.0
        };
        let start = frame.quads.len();
        for (k, u) in USERS.iter().enumerate() {
            let src = user_pos(u, k);
            for (n, &(di, fi, c)) in u.targets.iter().enumerate() {
                let dest = file_pos(di, fi);
                let perp = (dest - src).normalize().perp();
                let offset = perp * 8.0 * 0.5;
                let offset_src = offset * 0.3;
                let progress = (t * 0.5 + n as f32 * 0.3).fract() * 0.8;
                let (a1, a2) = (1.0 - progress, (1.0 - progress) * 0.1);
                let c1 = [c[0], c[1], c[2], a1];
                let c2 = [c[0], c[1], c[2], a2];
                frame.quads.push(QuadVertex {
                    pos: (src - offset_src).into(),
                    colour: c2,
                    uv: [0.0, 0.0],
                });
                frame.quads.push(QuadVertex {
                    pos: (src + offset_src).into(),
                    colour: c2,
                    uv: [0.0, 1.0],
                });
                frame.quads.push(QuadVertex {
                    pos: (dest + offset).into(),
                    colour: c1,
                    uv: [1.0, 1.0],
                });
                frame.quads.push(QuadVertex {
                    pos: (dest - offset).into(),
                    colour: c1,
                    uv: [1.0, 0.0],
                });
            }
        }
        quads_command(
            frame,
            start,
            Program::Textured,
            Space::World,
            [0.0, 0.0],
            TextureId::Beam,
        );
        frame.commands.push(DrawCommand {
            program: Program::Textured,
            offset: [0.0, 0.0],
            ..files
        });

        // 4. Users: shadow then sprite.
        let start = frame.quads.len();
        for (k, u) in USERS.iter().enumerate() {
            let p = user_pos(u, k);
            let c = u.colour.map(|v| v * 0.6 + 0.4);
            quad(
                &mut frame.quads,
                p - Vec2::splat(10.0),
                Vec2::splat(20.0),
                [c[0], c[1], c[2], 1.0],
                [0.0, 0.0, 1.0, 1.0],
            );
        }
        quads_command(
            frame,
            start,
            Program::Shadow { strength: 0.5 },
            Space::World,
            [2.0, 2.0],
            TextureId::User,
        );
        let users = *frame.commands.last().unwrap();
        frame.commands.push(DrawCommand {
            program: Program::Textured,
            offset: [0.0, 0.0],
            ..users
        });

        // 5. Bloom: additive procedural glow per dir.
        for (i, d) in DIRS.iter().enumerate() {
            let radius = dir_radius(i) * 2.0 * 1.0;
            let c = dir_colour(i).map(|v| v * 0.75);
            let colour = [c[0], c[1], c[2], 1.0];
            let tc = [radius, d.pos.x, d.pos.y, 0.0];
            let p = d.pos - Vec2::splat(radius);
            let dims = Vec2::splat(radius * 2.0);
            for corner in [
                Vec2::ZERO,
                Vec2::new(dims.x, 0.0),
                dims,
                Vec2::new(0.0, dims.y),
            ] {
                frame.bloom.push(BloomVertex {
                    pos: (p + corner).into(),
                    colour,
                    texcoord: tc,
                });
            }
        }
        frame.commands.push(DrawCommand {
            stream: Stream::Bloom,
            first_quad: 0,
            quad_count: (frame.bloom.len() / 4) as u32,
            program: Program::Bloom,
            space: Space::World,
            offset: [0.0, 0.0],
            texture: TextureId::White,
            blend: Blend::Additive,
        });

        // 6. Text: dir names, file names, user names, date, title, logo.
        let glyphs = self.backend.glyphs();
        let size = |s: f32| (s * ui).round() as u16;
        for d in DIRS.iter().filter(|d| !d.name.is_empty()) {
            let p = project(d.pos);
            text(
                frame,
                glyphs,
                size(14.0),
                p,
                d.name,
                [1.0, 1.0, 1.0, 1.0],
                true,
            );
        }
        for (di, fi) in [(1, 0), (2, 1), (5, 0), (3, 0)] {
            let (name, _) = DIRS[di].files[fi];
            let p = project(file_pos(di, fi)) + Vec2::new(5.0, -7.0) * ui;
            text(
                frame,
                glyphs,
                size(14.0),
                p,
                name,
                [1.0, 1.0, 1.0, 1.0],
                false,
            );
        }
        for (k, u) in USERS.iter().enumerate() {
            let p = project(user_pos(u, k)) + Vec2::new(0.0, 14.0) * ui;
            text(
                frame,
                glyphs,
                size(14.0),
                p,
                u.name,
                [1.0, 1.0, 1.0, 1.0],
                true,
            );
        }
        text(
            frame,
            glyphs,
            size(16.0),
            Vec2::new(vp.x * 0.5, 20.0 * ui),
            "Wednesday, 1 October 2026",
            [1.0; 4],
            true,
        );
        text(
            frame,
            glyphs,
            size(16.0),
            Vec2::new(10.0 * ui, vp.y - 30.0 * ui),
            "gource-render-webgl2 demo",
            [1.0; 4],
            false,
        );
        text(
            frame,
            glyphs,
            size(42.0),
            Vec2::new(vp.x - 230.0 * ui, vp.y - 70.0 * ui),
            "Gource",
            [1.0, 1.0, 1.0, 0.9],
            false,
        );
        let texel = glyphs.texel_size();
        frame.commands.push(DrawCommand {
            stream: Stream::Text,
            first_quad: 0,
            quad_count: (frame.text.len() / 4) as u32,
            program: Program::Text {
                shadow_strength: 0.7,
                texel,
            },
            space: Space::Screen,
            offset: [0.0, 0.0],
            texture: TextureId::Glyphs,
            blend: Blend::Alpha,
        });
    }

    fn tick(&mut self, now_ms: f64) -> Result<(), JsValue> {
        // what FrameBuilder::build does first
        self.backend.glyphs().begin_frame();
        if self.frames == 0 {
            self.start_ms = now_ms;
            let glyphs = self.backend.glyphs();
            for size_px in 8..=self.atlas_stress {
                glyphs.metrics(FontKey { size_px });
            }
        }
        self.resize();
        self.build(((now_ms - self.start_ms) / 1000.0) as f32);
        let stats = self.backend.draw_frame(&self.frame)?;
        let gl = self.backend.context();
        loop {
            let err = gl.get_error();
            // Context loss is reported once through getError; it is expected.
            if err == web_sys::WebGl2RenderingContext::NO_ERROR
                || err == web_sys::WebGl2RenderingContext::CONTEXT_LOST_WEBGL
            {
                break;
            }
            self.gl_errors += 1;
            web_sys::console::error_1(&format!("GL error 0x{err:04x}").into());
        }
        self.frames += 1;
        self.report(&stats);
        Ok(())
    }

    fn report(&self, s: &BackendStats) {
        let Some(el) = &self.stats_el else { return };
        if self.frames % 10 != 1 && self.frames > 3 {
            return;
        }
        el.set_text_content(Some(&format!(
            "frame {}  {}x{}  draw calls {}  vertices {}  uploaded {} B  commands {}  atlas {} px gen {}  context {}  GL errors {}",
            self.frames,
            self.frame.viewport[0],
            self.frame.viewport[1],
            s.draw_calls,
            s.vertices,
            s.uploaded_bytes,
            self.frame.commands.len(),
            self.backend.atlas().size(),
            self.backend.atlas().generation(),
            if self.backend.is_context_lost() { "lost" } else { "ok" },
            self.gl_errors
        )));
        if self.frames >= 3 {
            let _ = el.dataset().set(
                "ready",
                if self.gl_errors == 0 {
                    "ok"
                } else {
                    "gl-error"
                },
            );
        }
    }
}

/// Start the demo on `canvas` (call once the GourceSans face has loaded).
/// `background` is an optional `RRGGBB` clear colour (default black).
/// `atlas_stress` > 0 precaches every font size from 8 px up to it on the
/// first frame, to exercise atlas growth (≈ 60) and reset (≈ 150).
#[wasm_bindgen]
pub fn start(
    canvas: HtmlCanvasElement,
    stats: Option<HtmlElement>,
    background: Option<String>,
    atlas_stress: u16,
) -> Result<(), JsValue> {
    let mut bg = [0.0, 0.0, 0.0, 1.0];
    if let Some(rgb) = background
        .as_deref()
        .and_then(|h| u32::from_str_radix(h, 16).ok())
    {
        for (i, c) in bg.iter_mut().take(3).enumerate() {
            *c = ((rgb >> (16 - 8 * i)) & 0xff) as f32 / 255.0;
        }
    }
    let backend = WebGl2Backend::new(&canvas)?;
    let demo = Rc::new(RefCell::new(Demo {
        backend,
        canvas,
        stats_el: stats,
        frame: Frame::default(),
        frames: 0,
        start_ms: 0.0,
        gl_errors: 0,
        background: bg,
        atlas_stress,
    }));

    type Loop = Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>>;
    let raf: Loop = Rc::new(RefCell::new(None));
    let next = raf.clone();
    *raf.borrow_mut() = Some(Closure::new(move |now: f64| {
        if let Err(e) = demo.borrow_mut().tick(now) {
            web_sys::console::error_1(&e);
        }
        request_frame(next.borrow().as_ref().unwrap());
    }));
    request_frame(raf.borrow().as_ref().unwrap());
    Ok(())
}

fn request_frame(f: &Closure<dyn FnMut(f64)>) {
    let _ = web_sys::window()
        .unwrap()
        .request_animation_frame(f.as_ref().unchecked_ref());
}
