// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! [`WebGl2Backend`]: executes a [`Frame`]'s commands with raw WebGL2 calls
//! (`Gource::drawScene` and the text/HUD passes of `Gource::draw`).

use std::cell::Cell;
use std::rc::Rc;

use gource_render::{
    Backend, BackendStats, Blend, DrawCommand, Frame, GlyphSource, Program, Space, Stream,
    TextureId,
};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    Event, HtmlCanvasElement, WebGl2RenderingContext as Gl, WebGlContextAttributes,
    WebGlPowerPreference,
};

use crate::atlas::GlyphAtlas;
use crate::shaders::{PROGRAM_COUNT, ProgramKind, ProgramState, UniformCache, create_programs};
use crate::streams::{BLOOM_LAYOUT, IndexBuffer, QUAD_LAYOUT, VertexStream};
use crate::textures::Textures;

/// Every GPU object; dropped and recreated on context restoration.
struct Resources {
    programs: [ProgramState; PROGRAM_COUNT],
    indices: IndexBuffer,
    quads: VertexStream,
    bloom: VertexStream,
    text: VertexStream,
    textures: Textures,
}

impl Resources {
    fn new(gl: &Gl) -> Result<Self, JsValue> {
        let programs = create_programs(gl)?;
        let indices = IndexBuffer::new(gl)?;
        Ok(Resources {
            quads: VertexStream::new(gl, QUAD_LAYOUT, &indices)?,
            bloom: VertexStream::new(gl, BLOOM_LAYOUT, &indices)?,
            text: VertexStream::new(gl, QUAD_LAYOUT, &indices)?,
            textures: Textures::new(gl)?,
            programs,
            indices,
        })
    }

    fn stream(&self, stream: Stream) -> &VertexStream {
        match stream {
            Stream::Quads => &self.quads,
            Stream::Bloom => &self.bloom,
            Stream::Text => &self.text,
        }
    }
}

/// GL state last set by this backend, to skip redundant calls.
#[derive(Copy, Clone, Default)]
struct StateCache {
    program: Option<ProgramKind>,
    stream: Option<Stream>,
    texture: Option<TextureId>,
    blend: Option<Blend>,
}

/// `webglcontextlost` / `webglcontextrestored` listeners.
struct ContextEvents {
    canvas: HtmlCanvasElement,
    lost: Rc<Cell<bool>>,
    restored: Rc<Cell<bool>>,
    on_lost: Closure<dyn FnMut(Event)>,
    on_restored: Closure<dyn FnMut(Event)>,
}

impl ContextEvents {
    fn attach(canvas: &HtmlCanvasElement) -> Result<Self, JsValue> {
        let lost = Rc::new(Cell::new(false));
        let restored = Rc::new(Cell::new(false));
        let on_lost = {
            let lost = lost.clone();
            Closure::<dyn FnMut(Event)>::new(move |e: Event| {
                // Without preventDefault the browser never restores the context.
                e.prevent_default();
                lost.set(true);
            })
        };
        let on_restored = {
            let (lost, restored) = (lost.clone(), restored.clone());
            Closure::<dyn FnMut(Event)>::new(move |_: Event| {
                lost.set(false);
                restored.set(true);
            })
        };
        canvas.add_event_listener_with_callback(
            "webglcontextlost",
            on_lost.as_ref().unchecked_ref(),
        )?;
        canvas.add_event_listener_with_callback(
            "webglcontextrestored",
            on_restored.as_ref().unchecked_ref(),
        )?;
        Ok(ContextEvents {
            canvas: canvas.clone(),
            lost,
            restored,
            on_lost,
            on_restored,
        })
    }
}

impl Drop for ContextEvents {
    fn drop(&mut self) {
        let _ = self.canvas.remove_event_listener_with_callback(
            "webglcontextlost",
            self.on_lost.as_ref().unchecked_ref(),
        );
        let _ = self.canvas.remove_event_listener_with_callback(
            "webglcontextrestored",
            self.on_restored.as_ref().unchecked_ref(),
        );
    }
}

/// Raw WebGL2 implementation of [`Backend`].
pub struct WebGl2Backend {
    gl: Gl,
    canvas: HtmlCanvasElement,
    atlas: GlyphAtlas,
    /// `None` while the context is lost.
    res: Option<Resources>,
    events: ContextEvents,
    state: StateCache,
    size: [u32; 2],
    /// GL errors reported to the console so far (debug builds).
    #[cfg(debug_assertions)]
    gl_errors: u32,
}

impl WebGl2Backend {
    /// Create the backend on `canvas` (fails when WebGL2 is unavailable).
    ///
    /// The `GourceSans` font face must be loaded before glyphs are requested.
    pub fn new(canvas: &HtmlCanvasElement) -> Result<Self, JsValue> {
        let attrs = WebGlContextAttributes::new();
        attrs.set_alpha(false);
        attrs.set_antialias(false);
        attrs.set_depth(false);
        attrs.set_stencil(false);
        attrs.set_premultiplied_alpha(false);
        attrs.set_preserve_drawing_buffer(false);
        attrs.set_power_preference(WebGlPowerPreference::HighPerformance);
        let gl: Gl = canvas
            .get_context_with_context_options("webgl2", &attrs)?
            .ok_or_else(|| JsValue::from_str("WebGL2 is not available"))?
            .dyn_into()?;
        let document = canvas
            .owner_document()
            .ok_or_else(|| JsValue::from_str("canvas has no document"))?;
        let mut backend = WebGl2Backend {
            atlas: GlyphAtlas::new(&document)?,
            events: ContextEvents::attach(canvas)?,
            canvas: canvas.clone(),
            res: None,
            state: StateCache::default(),
            size: [canvas.width(), canvas.height()],
            #[cfg(debug_assertions)]
            gl_errors: 0,
            gl,
        };
        backend.create_resources()?;
        Ok(backend)
    }

    /// The glyph atlas (mutable access is [`Backend::glyphs`]).
    pub fn atlas(&self) -> &GlyphAtlas {
        &self.atlas
    }

    /// The WebGL2 context, for diagnostics.
    pub fn context(&self) -> &Gl {
        &self.gl
    }

    /// True while the WebGL context is lost (frames are skipped).
    pub fn is_context_lost(&self) -> bool {
        self.events.lost.get() || self.gl.is_context_lost()
    }

    /// New glyphs rasterised per frame; `u32::MAX` never defers a label to
    /// a later frame (the video recorder; live views keep the default).
    pub fn set_glyph_budget(&mut self, limit: u32) {
        self.atlas.set_frame_budget(limit);
    }

    /// Drop every rasterised glyph before the next frame and zero the
    /// eviction count.
    pub fn reset_glyphs(&mut self) {
        self.atlas.reset();
    }

    fn create_resources(&mut self) -> Result<(), JsValue> {
        let gl = &self.gl;
        gl.disable(Gl::DEPTH_TEST);
        gl.disable(Gl::CULL_FACE);
        gl.enable(Gl::BLEND);
        gl.active_texture(Gl::TEXTURE0);
        gl.viewport(0, 0, self.size[0] as i32, self.size[1] as i32);
        self.res = Some(Resources::new(gl)?);
        self.state = StateCache::default();
        self.atlas.mark_dirty();
        Ok(())
    }

    /// Debug builds: report pending GL errors on the console.
    #[cfg(debug_assertions)]
    fn check_errors(&mut self) {
        loop {
            let err = self.gl.get_error();
            if err == Gl::NO_ERROR || err == Gl::CONTEXT_LOST_WEBGL {
                break;
            }
            self.gl_errors += 1;
            if self.gl_errors <= 16 {
                web_sys::console::error_1(&JsValue::from_str(&format!(
                    "gource-render-webgl2: GL error 0x{err:04x} after draw_frame"
                )));
            }
        }
    }

    fn execute(
        &mut self,
        cmd: &DrawCommand,
        frame: &Frame,
        stats: &mut BackendStats,
    ) -> Result<(), JsValue> {
        let gl = &self.gl;
        let Some(res) = self.res.as_mut() else {
            return Ok(());
        };
        let stream = res.stream(cmd.stream);
        let end = u64::from(cmd.first_quad) + u64::from(cmd.quad_count);
        if cmd.quad_count == 0 || end * 4 > u64::from(stream.len) {
            return Ok(());
        }

        let kind = match cmd.program {
            Program::Textured => ProgramKind::Textured,
            Program::Shadow { .. } => ProgramKind::Shadow,
            Program::Bloom => ProgramKind::Bloom,
            Program::Text { .. } => ProgramKind::Text,
        };
        let program = &mut res.programs[kind as usize];
        if self.state.program != Some(kind) {
            gl.use_program(Some(&program.program));
            self.state.program = Some(kind);
        }

        // Uniforms.
        let c: &mut UniformCache = &mut program.cache;
        if c.matrix != Some(cmd.space) {
            let m = match cmd.space {
                Space::World => &frame.world_to_clip,
                Space::Screen => &frame.screen_to_clip,
            };
            gl.uniform_matrix4fv_with_f32_array(program.u_matrix.as_ref(), false, m);
            c.matrix = Some(cmd.space);
        }
        if c.offset != Some(cmd.offset) {
            gl.uniform2f(program.u_offset.as_ref(), cmd.offset[0], cmd.offset[1]);
            c.offset = Some(cmd.offset);
        }
        let uv_scale = if cmd.texture == TextureId::Glyphs {
            self.atlas.uv_scale()
        } else {
            [1.0, 1.0]
        };
        if kind != ProgramKind::Bloom && c.uv_scale != Some(uv_scale) {
            gl.uniform2f(program.u_uv_scale.as_ref(), uv_scale[0], uv_scale[1]);
            c.uv_scale = Some(uv_scale);
        }
        match cmd.program {
            Program::Shadow { strength } => {
                if c.strength != Some(strength) {
                    gl.uniform1f(program.u_strength.as_ref(), strength);
                    c.strength = Some(strength);
                }
            }
            Program::Text {
                shadow_strength,
                texel,
            } => {
                if c.strength != Some(shadow_strength) {
                    gl.uniform1f(program.u_strength.as_ref(), shadow_strength);
                    c.strength = Some(shadow_strength);
                }
                let texel = [texel[0] * uv_scale[0], texel[1] * uv_scale[1]];
                if c.texel != Some(texel) {
                    gl.uniform2f(program.u_texel.as_ref(), texel[0], texel[1]);
                    c.texel = Some(texel);
                }
            }
            Program::Textured | Program::Bloom => {}
        }

        // Texture (the bloom program is procedural).
        if kind != ProgramKind::Bloom {
            if cmd.texture == TextureId::Glyphs && self.atlas.take_dirty() {
                res.textures.upload_glyphs(gl, self.atlas.canvas())?;
                stats.uploaded_bytes += self.atlas.size() * self.atlas.size() * 4;
                self.state.texture = Some(TextureId::Glyphs);
            }
            if self.state.texture != Some(cmd.texture) {
                let t = &res.textures;
                let texture = match cmd.texture {
                    TextureId::White => &t.white,
                    TextureId::File => &t.file,
                    TextureId::User => &t.user,
                    TextureId::Beam => &t.beam,
                    TextureId::Glyphs => &t.glyphs,
                    TextureId::Deco => &t.deco,
                };
                gl.bind_texture(Gl::TEXTURE_2D, Some(texture));
                self.state.texture = Some(cmd.texture);
            }
        }

        if self.state.blend != Some(cmd.blend) {
            match cmd.blend {
                Blend::Alpha => gl.blend_func(Gl::SRC_ALPHA, Gl::ONE_MINUS_SRC_ALPHA),
                Blend::Additive => gl.blend_func(Gl::ONE, Gl::ONE),
            }
            self.state.blend = Some(cmd.blend);
        }

        if self.state.stream != Some(cmd.stream) {
            gl.bind_vertex_array(Some(&res.stream(cmd.stream).vao));
            self.state.stream = Some(cmd.stream);
        }

        gl.draw_elements_with_i32(
            Gl::TRIANGLES,
            (cmd.quad_count * 6) as i32,
            Gl::UNSIGNED_INT,
            (cmd.first_quad * 6 * 4) as i32,
        );
        stats.draw_calls += 1;
        stats.vertices += cmd.quad_count * 4;
        Ok(())
    }
}

impl Backend for WebGl2Backend {
    type Error = JsValue;

    fn resize(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if self.canvas.width() != width {
            self.canvas.set_width(width);
        }
        if self.canvas.height() != height {
            self.canvas.set_height(height);
        }
        self.size = [width, height];
        self.gl.viewport(0, 0, width as i32, height as i32);
    }

    fn glyphs(&mut self) -> &mut dyn GlyphSource {
        &mut self.atlas
    }

    fn drawing_buffer_size(&self) -> Option<[u32; 2]> {
        let (w, h) = (
            self.gl.drawing_buffer_width(),
            self.gl.drawing_buffer_height(),
        );
        Some([w.max(0) as u32, h.max(0) as u32])
    }

    fn atlas_evictions(&self) -> u32 {
        self.atlas.evictions()
    }

    fn draw_frame(&mut self, frame: &Frame) -> Result<BackendStats, JsValue> {
        if self.events.restored.take() {
            self.res = None;
            self.create_resources()?;
            // A GPU process crash can clear the atlas canvas too: every
            // glyph is drawn again from the next frame on.
            self.atlas.request_invalidate();
        }
        if self.is_context_lost() {
            return Ok(BackendStats::default());
        }
        let mut stats = BackendStats::default();
        let gl = &self.gl;
        let Some(res) = self.res.as_mut() else {
            return Ok(stats);
        };

        let [r, g, b, a] = frame.clear_colour.map(clear_level);
        gl.clear_color(r, g, b, a);
        gl.clear(Gl::COLOR_BUFFER_BIT);

        // Uploads, before any draw so the streams keep their VAO bindings.
        let max_quads = (frame
            .quads
            .len()
            .max(frame.bloom.len())
            .max(frame.text.len())
            / 4) as u32;
        stats.uploaded_bytes += res.indices.reserve(gl, max_quads);
        stats.uploaded_bytes += res.quads.upload(gl, &frame.quads);
        stats.uploaded_bytes += res.bloom.upload(gl, &frame.bloom);
        stats.uploaded_bytes += res.text.upload(gl, &frame.text);

        // The world matrix changes every frame; the VAO binding may have
        // been reset by the index upload.
        for p in &mut res.programs {
            p.cache.matrix = None;
        }
        self.state.stream = None;

        for cmd in &frame.commands {
            self.execute(cmd, frame, &mut stats)?;
        }

        #[cfg(debug_assertions)]
        self.check_errors();
        Ok(stats)
    }
}

/// A clear colour component snapped to the middle of the 8-bit level that
/// round-to-nearest gives (`0.1` is 25.5 levels: 26, as Mesa clears
/// Gource's default background). Drivers disagree on exact halves (NVIDIA's
/// GL clears 0.1 to 25), so the value handed to `clearColor` sits a quarter
/// level above the level, away from any rounding boundary.
fn clear_level(c: f32) -> f32 {
    let level = (c.clamp(0.0, 1.0) * 255.0 + 0.5).floor();
    if level >= 255.0 {
        1.0
    } else {
        (level + 0.25) / 255.0
    }
}
