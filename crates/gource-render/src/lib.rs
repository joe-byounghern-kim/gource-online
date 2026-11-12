// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Renderer contract for the Gource port.
//!
//! This crate defines the batched frame data produced from a core scene (the
//! equivalent of Gource's per-frame VBO rebuild), the draw-pass order, and the
//! trait a GPU backend implements. It contains no GPU API calls itself;
//! backends such as `gource-render-webgl2` implement the trait against the raw
//! platform API. No scene-graph or engine library is used.
//!
//! Data flow per frame:
//!
//! ```text
//! Simulation::extract_view ─► SceneView ─► FrameBuilder::build ─► Frame ─► Backend::draw_frame
//!                                               ▲
//!                                     GlyphSource (backend's atlas)
//! ```
//!
//! A [`Frame`] holds three vertex streams (textured quads, bloom quads, glyph
//! quads), each four vertices per quad in the order Gource's `quadbuf::add`
//! emits them, and an ordered list of [`DrawCommand`]s that reference quad
//! ranges of those streams. Backends draw quads as two triangles
//! (0,1,2)(0,2,3) from a shared index buffer, so the whole scene is roughly a
//! dozen draw calls regardless of entity count.

mod agent_key;
mod builder;
mod deco;
pub mod deco_atlas;
mod emit;
mod key;
mod spline;
mod text;
mod textbox;

pub use builder::FrameBuilder;
pub use deco::DECO_TEXTURE_MAX_LEVEL;
pub use gource_core;

/// `quadbuf_vertex`: position, colour, texcoord (32 bytes).
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct QuadVertex {
    pub pos: [f32; 2],
    pub colour: [f32; 4],
    pub uv: [f32; 2],
}

/// `bloom_vertex`: position, colour, and `(radius, centre.x, centre.y, 0)`
/// packed into the texcoord as `RDirNode::updateBloomVBO` does (40 bytes).
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct BloomVertex {
    pub pos: [f32; 2],
    pub colour: [f32; 4],
    pub texcoord: [f32; 4],
}

/// Built-in textures. Backends load the PNGs from `data/`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextureId {
    /// 1×1 opaque white, for untextured quads.
    White,
    /// `data/file.png`.
    File,
    /// `data/user.png`.
    User,
    /// `data/beam.png`.
    Beam,
    /// The backend's glyph atlas.
    Glyphs,
    /// `data/decorations.png`, the AI decoration atlas (cells in
    /// [`deco_atlas`]); mipmapped up to [`DECO_TEXTURE_MAX_LEVEL`].
    Deco,
}

/// Coordinate space of a command's vertices.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Space {
    /// World units, transformed by [`Frame::world_to_clip`].
    World,
    /// Top-left-origin pixels, transformed by [`Frame::screen_to_clip`].
    Screen,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Program {
    /// `texture * colour` (fixed-function path of Gource).
    Textured,
    /// `data/shaders/shadow.frag`: black with `colour.a * tex.a * strength`.
    Shadow { strength: f32 },
    /// `data/shaders/bloom.frag` (procedural glow; texture unused).
    Bloom,
    /// `data/shaders/text.frag`: atlas alpha with a drop shadow sampled
    /// `texel` atlas units up-left.
    Text {
        shadow_strength: f32,
        texel: [f32; 2],
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Blend {
    /// `GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA`.
    Alpha,
    /// `GL_ONE, GL_ONE`.
    Additive,
}

/// Which vertex stream a command draws from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Stream {
    Quads,
    Bloom,
    Text,
}

/// One draw call: `quad_count` quads starting at quad `first_quad` of `stream`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DrawCommand {
    pub stream: Stream,
    pub first_quad: u32,
    pub quad_count: u32,
    pub program: Program,
    pub space: Space,
    /// Translation applied to every vertex before the space transform
    /// (Gource's `glTranslatef` shadow offsets).
    pub offset: [f32; 2],
    pub texture: TextureId,
    pub blend: Blend,
}

/// Everything a backend needs to draw one frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// Drawable size in device pixels.
    pub viewport: [u32; 2],
    pub clear_colour: [f32; 4],
    /// Column-major perspective × look-at matrix.
    pub world_to_clip: [f32; 16],
    /// Column-major `glOrtho(0, w, h, 0, -1, 1)`.
    pub screen_to_clip: [f32; 16],
    pub quads: Vec<QuadVertex>,
    pub bloom: Vec<BloomVertex>,
    pub text: Vec<QuadVertex>,
    pub commands: Vec<DrawCommand>,
}

impl Frame {
    /// Empty every stream, keeping capacity.
    pub fn clear(&mut self) {
        self.quads.clear();
        self.bloom.clear();
        self.text.clear();
        self.commands.clear();
    }
}

/// A font face at one pixel size.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FontKey {
    pub size_px: u16,
}

/// Vertical metrics of a font size (FTGL's `FTFont` / `FXFont`).
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct FontMetrics {
    pub ascender: f32,
    /// Negative below the baseline, as in FreeType.
    pub descender: f32,
    /// `FXFont::getMaxHeight()`.
    pub max_height: f32,
}

/// Placement of one glyph in the atlas.
///
/// The frame builder draws each glyph as a `size` quad at
/// `pen + bearing` sampling `uv_min..uv_max`, exactly as given; a backend
/// that wants `FXGlyph`'s one-texel padding (quad `bitmap + 2`, corner
/// `(left + 0.5, -top - 0.5)`, texcoords widened by `-0.5 / +1.5` texels, so
/// the text shader's shadow is not clipped) folds it into these fields.
/// Glyphs with an empty `size` (spaces) only advance the pen.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct GlyphInfo {
    /// Atlas texture coordinates of the glyph bitmap (top-left, bottom-right).
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    /// Bitmap size in pixels.
    pub size: [f32; 2],
    /// Offset from the pen position (on the baseline) to the bitmap's
    /// top-left corner, in pixels, y down.
    pub bearing: [f32; 2],
    /// Horizontal pen advance in pixels.
    pub advance: f32,
}

/// Glyph rasterisation and metrics, implemented by the backend (canvas 2D in
/// the browser, a fixed-width stub in tests).
pub trait GlyphSource {
    fn metrics(&mut self, font: FontKey) -> FontMetrics;
    /// Rasterise on first use. Must return the same placement for the same
    /// `(font, ch)` until the next [`GlyphSource::begin_frame`] but one
    /// (frames may be built twice before one is drawn); an atlas may move
    /// or drop glyphs that were not used for longer.
    fn glyph(&mut self, font: FontKey, ch: char) -> GlyphInfo;
    /// Size of one atlas texel in texture coordinates.
    fn texel_size(&self) -> [f32; 2];
    /// A new frame starts being built (the frame builder calls it first).
    fn begin_frame(&mut self) {}
}

/// Fixed-advance glyphs for tests and headless runs: every character is a
/// `0.6 × size` wide box with no atlas behind it.
#[derive(Debug, Default)]
pub struct MonospaceGlyphs;

impl GlyphSource for MonospaceGlyphs {
    fn metrics(&mut self, font: FontKey) -> FontMetrics {
        let s = font.size_px as f32;
        FontMetrics {
            ascender: s * 0.8,
            descender: -s * 0.2,
            max_height: s,
        }
    }

    fn glyph(&mut self, font: FontKey, _ch: char) -> GlyphInfo {
        let s = font.size_px as f32;
        GlyphInfo {
            uv_min: [0.0, 0.0],
            uv_max: [0.0, 0.0],
            size: [s * 0.6, s],
            bearing: [0.0, -s * 0.8],
            advance: s * 0.6,
        }
    }

    fn texel_size(&self) -> [f32; 2] {
        [1.0 / 1024.0, 1.0 / 1024.0]
    }
}

/// Draw-call and upload counters for the debug overlay.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct BackendStats {
    pub draw_calls: u32,
    pub vertices: u32,
    pub uploaded_bytes: u32,
}

/// A GPU backend.
pub trait Backend {
    type Error;

    /// Resize the drawable to `width × height` device pixels.
    fn resize(&mut self, width: u32, height: u32);

    /// The backend's glyph atlas.
    fn glyphs(&mut self) -> &mut dyn GlyphSource;

    /// Upload the frame's streams and execute its commands in order.
    fn draw_frame(&mut self, frame: &Frame) -> Result<BackendStats, Self::Error>;

    /// The drawable size the platform actually allocated, when the backend
    /// can tell (a browser silently shrinks an oversized canvas). The
    /// recorder compares it with the size it asked for.
    fn drawing_buffer_size(&self) -> Option<[u32; 2]> {
        None
    }

    /// Glyphs the glyph atlas has dropped to make room since it was created
    /// or last reset (labels drawn after that were rasterised again).
    fn atlas_evictions(&self) -> u32 {
        0
    }
}
