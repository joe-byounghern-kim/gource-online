// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Glyph atlas rasterised with canvas 2D (`core/fxfont.cpp` `FXGlyphSet` /
//! `FXGlyphPage` semantics, with the browser instead of FreeType).
//!
//! Glyphs are drawn white on transparent into one atlas canvas with the
//! `GourceSans` font face (the host page loads `data/fonts/FreeSans.ttf`
//! under that name before constructing the backend). Each font size first
//! precaches Gource's character list, exactly as `FXGlyphSet::init` does, so
//! `max_height` has `FXFont::getMaxHeight` semantics: the tallest glyph ink
//! height (`ceil(metrics.height)`) seen so far at that size.
//!
//! Texture coordinates are in units of the *base* atlas size (1024). The
//! atlas starts at 1024² and grows once to 2048² by copying its contents, so
//! glyphs never move; the backend multiplies texture coordinates by
//! [`GlyphAtlas::uv_scale`]. Gource's glyph sets add texture pages without
//! limit; here cells live on shelves ([`crate::packer`]) and, once the
//! 2048² atlas is full, the shelf unused for the longest time (never one
//! used in this frame or the previous one) is cleared and reused. A glyph
//! that finds no space, or exceeds the frame's budget ([`GlyphBudget`],
//! [`crate::budget::MAX_NEW_PER_FRAME`] unless raised), is laid out with its advance
//! (estimated until it is measured) and drawn from a later frame.
//!
//! After a GPU process crash the browser can clear the atlas canvas as well
//! as the WebGL context, so a restored context (and the canvas' own
//! `contextrestored`) re-rasterises every glyph on demand.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gource_render::{FontKey, FontMetrics, GlyphInfo, GlyphSource};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, Document, Event, HtmlCanvasElement};

use crate::budget::GlyphBudget;
use crate::packer::{Alloc, ShelfPacker};

/// Base atlas size: texture coordinates and `texel_size` use this unit.
const BASE_SIZE: u32 = 1024;
/// Largest atlas; beyond it shelves are evicted.
const MAX_SIZE: u32 = 2048;
/// Extra rows/columns kept around the measured ink box so antialiasing that
/// spills past `actualBoundingBox*` is not clipped by a neighbour.
const INK_MARGIN: f64 = 1.0;

/// `FXGlyphSet::init` precache list.
const PRECACHE: &str =
    "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ;:'\",<.>/?-_=+!@#$%^&*()\\ ";

/// Font family registered by the host page.
pub const FONT_FAMILY: &str = "GourceSans";

/// Characters below this code point are looked up in a per-size table.
const ASCII: usize = 128;

/// A rasterised glyph and the shelf holding its cell.
#[derive(Copy, Clone, Debug)]
struct Cached {
    info: GlyphInfo,
    shelf: u32,
}

/// A measured glyph still waiting for a cell.
#[derive(Copy, Clone, Debug)]
struct Measured {
    /// Bitmap box relative to the pen (x right, y up).
    left: f64,
    top: f64,
    w: u32,
    h: u32,
    advance: f32,
}

impl Measured {
    /// Laid out with its advance, drawn as nothing.
    fn placeholder(&self) -> GlyphInfo {
        GlyphInfo {
            advance: self.advance,
            ..GlyphInfo::default()
        }
    }
}

/// ASCII glyphs of one font size, indexed by code point. The frame builder
/// asks for every character of every label each frame, and a SipHash
/// `HashMap` lookup per glyph was ~6 % of a busy browser frame.
type AsciiTable = Box<[Option<Cached>; ASCII]>;

type Glyph = (u16, char);

/// `contextrestored` on the atlas canvas (a 2D context lost with the GPU
/// process comes back blank).
struct CanvasEvents {
    canvas: HtmlCanvasElement,
    restored: Rc<Cell<bool>>,
    on_restored: Closure<dyn FnMut(Event)>,
}

impl CanvasEvents {
    fn attach(canvas: &HtmlCanvasElement, restored: Rc<Cell<bool>>) -> Self {
        let flag = restored.clone();
        let on_restored = Closure::<dyn FnMut(Event)>::new(move |_: Event| flag.set(true));
        let _ = canvas.add_event_listener_with_callback(
            "contextrestored",
            on_restored.as_ref().unchecked_ref(),
        );
        CanvasEvents {
            canvas: canvas.clone(),
            restored,
            on_restored,
        }
    }
}

impl Drop for CanvasEvents {
    fn drop(&mut self) {
        let _ = self.canvas.remove_event_listener_with_callback(
            "contextrestored",
            self.on_restored.as_ref().unchecked_ref(),
        );
    }
}

/// Canvas-2D glyph atlas implementing [`GlyphSource`].
pub struct GlyphAtlas {
    document: Document,
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    events: CanvasEvents,
    packer: ShelfPacker,
    glyphs: HashMap<Glyph, Cached>,
    /// Copy of the ASCII entries of `glyphs`, one table per font size.
    ascii: Vec<(u16, AsciiTable)>,
    /// The glyphs on each shelf, to forget them when it is evicted.
    shelf_glyphs: Vec<Vec<Glyph>>,
    /// Measured glyphs without a cell yet.
    waiting: HashMap<Glyph, Measured>,
    /// Glyphs rasterised in this frame (precache lists excluded) against
    /// the per-frame limit, and the glyphs evicted so far.
    budget: GlyphBudget,
    /// No cell could be found in this frame: stop trying until the next.
    full: bool,
    precaching: bool,
    /// Re-rasterise everything before the next frame.
    invalidate: bool,
    fonts: HashMap<u16, FontMetrics>,
    /// Font size currently set on `ctx` (0 = unknown).
    ctx_font: u16,
    /// The face's ascender and descender (both positive) in units of
    /// 1/1000 em, measured once at a 1000 px font.
    em_metrics: Option<(f32, f32)>,
    /// Per character: ink ascent and descent in 1/1000 em (at 1000 px).
    em_boxes: HashMap<char, (f64, f64)>,
    dirty: bool,
    generation: u32,
    scratch: String,
}

impl GlyphAtlas {
    pub fn new(document: &Document) -> Result<Self, JsValue> {
        let (canvas, ctx) = new_canvas(document, BASE_SIZE)?;
        Ok(GlyphAtlas {
            document: document.clone(),
            events: CanvasEvents::attach(&canvas, Rc::new(Cell::new(false))),
            canvas,
            ctx,
            packer: ShelfPacker::new(BASE_SIZE, MAX_SIZE),
            glyphs: HashMap::new(),
            ascii: Vec::new(),
            shelf_glyphs: Vec::new(),
            waiting: HashMap::new(),
            budget: GlyphBudget::default(),
            full: false,
            precaching: false,
            invalidate: false,
            fonts: HashMap::new(),
            ctx_font: 0,
            em_metrics: None,
            em_boxes: HashMap::new(),
            dirty: false,
            generation: 0,
            scratch: String::new(),
        })
    }

    /// The atlas canvas (upload source).
    pub fn canvas(&self) -> &HtmlCanvasElement {
        &self.canvas
    }

    /// Incremented whenever glyphs are dropped (a shelf evicted, the atlas
    /// invalidated), so [`GlyphInfo`]s handed out before may be stale.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Current atlas size in pixels.
    pub fn size(&self) -> u32 {
        self.packer.size()
    }

    /// Factor from base-unit texture coordinates to the texture's.
    pub fn uv_scale(&self) -> [f32; 2] {
        let s = BASE_SIZE as f32 / self.size() as f32;
        [s, s]
    }

    /// True when the canvas changed since the last call.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Force a re-upload (the GL texture was recreated).
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Drop and re-rasterise every glyph before the next frame is built
    /// (the GL context was restored: the canvas may have been cleared with
    /// the GPU process). Font metrics are kept, so layout does not change.
    pub fn request_invalidate(&mut self) {
        self.invalidate = true;
        self.dirty = true;
    }

    /// New glyphs measured or rasterised per frame (`u32::MAX`: no glyph is
    /// ever deferred to a later frame, as a video recorder needs).
    pub fn set_frame_budget(&mut self, limit: u32) {
        self.budget.set_limit(limit);
    }

    /// Glyphs dropped with evicted shelves since creation or [`Self::reset`].
    pub fn evictions(&self) -> u32 {
        self.budget.evicted()
    }

    /// Forget every glyph before the next frame and zero the eviction count
    /// (a recording starts from a clean atlas, so its frames do not depend
    /// on what was drawn before).
    pub fn reset(&mut self) {
        self.request_invalidate();
        self.budget.reset_evictions();
    }

    fn invalidate_now(&mut self) {
        self.invalidate = false;
        // a restored 2D context is back at its default state
        init_ctx(&self.ctx);
        self.ctx_font = 0;
        let s = f64::from(self.size());
        self.ctx.clear_rect(0.0, 0.0, s, s);
        self.packer.clear();
        self.glyphs.clear();
        for (_, table) in &mut self.ascii {
            table.fill(None);
        }
        self.shelf_glyphs.clear();
        self.waiting.clear();
        // measured while the context may have been lost
        self.em_boxes.clear();
        self.dirty = true;
        self.generation += 1;
    }

    fn set_font(&mut self, size: u16) {
        if self.ctx_font != size {
            self.ctx
                .set_font(&format!("{size}px {FONT_FAMILY}, sans-serif"));
            self.ctx_font = size;
        }
    }

    /// The face's ascender and descender in 1/1000 em. Canvas rounds
    /// `fontBoundingBox*` to whole pixels, so they are read at 1000 px,
    /// where a pixel is a font unit of FreeSans (1000 units per em).
    fn em_metrics(&mut self) -> (f32, f32) {
        if let Some(m) = self.em_metrics {
            return m;
        }
        self.set_font(1000);
        let m = match self.ctx.measure_text("M") {
            Ok(m) => (
                m.font_bounding_box_ascent() as f32,
                m.font_bounding_box_descent() as f32,
            ),
            Err(_) => (800.0, 200.0),
        };
        self.em_metrics = Some(m);
        m
    }

    /// `FXGlyphSet::init`: vertical metrics plus the precache list.
    ///
    /// `getAscender()` / `getDescender()` are the face's unrounded
    /// `ascender` / `descender` times `y_ppem / units_per_EM` in single
    /// precision (72 dpi, so `y_ppem` is the pixel size), which
    /// `FXFont::getHeight` rounds up as a sum: 9 px at 12 px and 12 px at
    /// 16 px for FreeSans, where the pixel-rounded canvas metrics give 8 and
    /// 11 and would lift top-aligned text by a pixel.
    fn ensure_font(&mut self, size: u16) {
        if self.fonts.contains_key(&size) {
            return;
        }
        let (asc_units, desc_units) = self.em_metrics();
        let unit_scale = (f64::from(size) / 1000.0) as f32;
        let (ascender, descender) = (asc_units * unit_scale, -desc_units * unit_scale);
        self.set_font(size);
        self.fonts.insert(
            size,
            FontMetrics {
                ascender,
                descender,
                max_height: 0.0,
            },
        );
        self.precaching = true;
        for ch in PRECACHE.chars() {
            if !self.glyphs.contains_key(&(size, ch)) {
                self.rasterise(size, ch);
            }
        }
        self.precaching = false;
    }

    /// `ceil(metrics.height / 64)` of the FreeType glyph Gource loads
    /// (`FT_LOAD_DEFAULT`, hinted). Canvas rounds `actualBoundingBox*` to
    /// whole pixels at small sizes, which loses a pixel against FreeType
    /// (14 px FreeSans: 14 instead of 15, so user names sat a pixel low).
    /// Hinting grid-fits the outline, so the height is estimated as the
    /// unhinted ink ascent and descent (read at 1000 px) each rounded up:
    /// for FreeSans' precache set this is FreeType's value at 33 of the
    /// sizes 8..=48, including 10-14, 16, 18, 20 and 24 (one pixel more at
    /// 9, 15, 26, 30, 37, 41, 45 and 48).
    fn hinted_height(&mut self, size: u16, ch: char) -> f32 {
        let (asc, desc) = match self.em_boxes.get(&ch) {
            Some(b) => *b,
            None => {
                self.set_font(1000);
                self.scratch.clear();
                self.scratch.push(ch);
                let b = match self.ctx.measure_text(&self.scratch) {
                    Ok(m) => (
                        m.actual_bounding_box_ascent(),
                        m.actual_bounding_box_descent(),
                    ),
                    Err(_) => (0.0, 0.0),
                };
                self.em_boxes.insert(ch, b);
                self.set_font(size);
                b
            }
        };
        if asc + desc <= 0.0 {
            return 0.0;
        }
        let s = f64::from(size) / 1000.0;
        ((asc * s).ceil() + (desc * s).ceil()).max(0.0) as f32
    }

    /// `FXGlyph` metrics and bitmap box of one character.
    fn measure(&mut self, size: u16, ch: char) -> Option<Measured> {
        self.set_font(size);
        self.scratch.clear();
        self.scratch.push(ch);
        let m = self.ctx.measure_text(&self.scratch).ok()?;
        let (abl, abr) = (m.actual_bounding_box_left(), m.actual_bounding_box_right());
        let (aba, abd) = (
            m.actual_bounding_box_ascent(),
            m.actual_bounding_box_descent(),
        );

        // FT_Glyph_Metrics.height, rounded up as FXGlyph does.
        let ink_height = self.hinted_height(size, ch);
        if let Some(f) = self.fonts.get_mut(&size) {
            f.max_height = f.max_height.max(ink_height);
        }

        // Bitmap box in pixels relative to the pen (x right, y up).
        let has_ink = abr + abl > 0.0 && aba + abd > 0.0;
        let (left, top, w, h) = if has_ink {
            let left = (-abl - INK_MARGIN).floor();
            let right = (abr + INK_MARGIN).ceil();
            let top = (aba + INK_MARGIN).ceil();
            let bottom = (abd + INK_MARGIN).ceil();
            (left, top, (right - left) as u32, (top + bottom) as u32)
        } else {
            (0.0, 0.0, 0, 0)
        };
        Some(Measured {
            left,
            top,
            w,
            h,
            // FT_LOAD_DEFAULT hints advances to whole pixels (advance.x >> 6).
            advance: m.width().round() as f32,
        })
    }

    /// `FXGlyph` + `FXGlyphPage::addGlyph` for one character, or its
    /// placeholder when it has to wait for a cell.
    fn rasterise(&mut self, size: u16, ch: char) -> GlyphInfo {
        let key = (size, ch);
        let over_budget = !self.precaching && self.budget.exhausted();
        let m = match self.waiting.get(&key) {
            Some(m) => *m,
            // not even measured yet: an estimated advance for this frame
            None if over_budget => return estimate(size, ch),
            None => {
                if !self.precaching {
                    self.budget.spend();
                }
                match self.measure(size, ch) {
                    Some(m) => m,
                    None => return GlyphInfo::default(),
                }
            }
        };
        let over_budget = !self.precaching && self.budget.exhausted();
        let cell = if over_budget || self.full {
            None
        } else {
            self.place(m.w, m.h)
        };
        let Some(cell) = cell else {
            self.waiting.insert(key, m);
            return m.placeholder();
        };
        self.waiting.remove(&key);
        if !self.precaching {
            self.budget.spend();
        }

        let (x, y, w, h) = (cell.x, cell.y, m.w, m.h);
        if w > 0 {
            // Skia boosts the coverage of light text (its text contrast and
            // gamma preblend): white glyphs carry about 13% more ink than
            // FreeType's linear coverage, black ones about 4%. Rasterise in
            // black, then turn the cell white keeping its alpha.
            self.set_font(size);
            self.scratch.clear();
            self.scratch.push(ch);
            let (cx, cy) = (f64::from(x), f64::from(y));
            self.ctx.set_fill_style_str("#000000");
            let _ = self.ctx.fill_text(&self.scratch, cx - m.left, cy + m.top);
            // `source-in` is unbounded (it clears everything outside the
            // drawn shape), so it is clipped to the cell.
            self.ctx.save();
            self.ctx.begin_path();
            self.ctx.rect(cx, cy, f64::from(w), f64::from(h));
            self.ctx.clip();
            let _ = self.ctx.set_global_composite_operation("source-in");
            self.ctx.set_fill_style_str("#ffffff");
            self.ctx.fill_rect(cx, cy, f64::from(w), f64::from(h));
            self.ctx.restore();
            self.dirty = true;
        }

        // FXGlyph: dims = bitmap + 2, corner = (left, -top) + (0.5, -0.5);
        // FXGlyphPage texcoords span (corner - 0.5, corner + bitmap + 1.5).
        let base = BASE_SIZE as f32;
        let (x, y, wf, hf) = (x as f32, y as f32, w as f32, h as f32);
        let info = GlyphInfo {
            uv_min: [(x - 0.5) / base, (y - 0.5) / base],
            uv_max: [(x + wf + 1.5) / base, (y + hf + 1.5) / base],
            size: [wf + 2.0, hf + 2.0],
            bearing: [m.left as f32 + 0.5, -(m.top as f32) - 0.5],
            advance: m.advance,
        };
        let cached = Cached {
            info,
            shelf: cell.shelf,
        };
        self.glyphs.insert(key, cached);
        let shelf = cell.shelf as usize;
        if self.shelf_glyphs.len() <= shelf {
            self.shelf_glyphs.resize_with(shelf + 1, Vec::new);
        }
        self.shelf_glyphs[shelf].push(key);
        if (ch as usize) < ASCII {
            let i = match self.ascii.iter().position(|(s, _)| *s == size) {
                Some(i) => i,
                None => {
                    self.ascii.push((size, Box::new([None; ASCII])));
                    self.ascii.len() - 1
                }
            };
            self.ascii[i].1[ch as usize] = Some(cached);
        }
        info
    }

    /// Reserve a `w × h` cell, growing the atlas or evicting a shelf.
    fn place(&mut self, w: u32, h: u32) -> Option<crate::packer::Cell> {
        loop {
            match self.packer.alloc(w, h) {
                Alloc::Placed(cell) => return Some(cell),
                Alloc::Grow { new_size } => {
                    if self.grow(new_size).is_err() {
                        self.full = true;
                        return None;
                    }
                }
                Alloc::Evicted { shelf, cell } => {
                    self.evict(shelf);
                    return Some(cell);
                }
                Alloc::Full => {
                    self.full = true;
                    return None;
                }
                Alloc::Oversized => return None,
            }
        }
    }

    /// Grow the canvas to `size`, keeping every glyph where it is.
    fn grow(&mut self, size: u32) -> Result<(), JsValue> {
        let (canvas, ctx) = new_canvas(&self.document, size)?;
        ctx.draw_image_with_html_canvas_element(&self.canvas, 0.0, 0.0)?;
        let restored = self.events.restored.clone();
        self.events = CanvasEvents::attach(&canvas, restored);
        self.canvas = canvas;
        self.ctx = ctx;
        self.ctx_font = 0;
        self.dirty = true;
        Ok(())
    }

    /// Forget the glyphs of `shelf` and clear its rows.
    fn evict(&mut self, shelf: u32) {
        if let Some(keys) = self.shelf_glyphs.get_mut(shelf as usize) {
            self.budget.note_evicted(keys.len());
            for key in keys.drain(..) {
                self.glyphs.remove(&key);
                if (key.1 as usize) < ASCII
                    && let Some((_, table)) = self.ascii.iter_mut().find(|(s, _)| *s == key.0)
                {
                    table[key.1 as usize] = None;
                }
            }
        }
        let s = self.packer.shelf(shelf);
        let (y, h) = (f64::from(s.y), f64::from(s.height));
        let size = f64::from(self.size());
        self.ctx.clear_rect(0.0, y - 1.0, size, h + 2.0);
        self.dirty = true;
        self.generation += 1;
    }
}

impl GlyphSource for GlyphAtlas {
    fn metrics(&mut self, font: FontKey) -> FontMetrics {
        let size = font.size_px.max(1);
        self.ensure_font(size);
        self.fonts.get(&size).copied().unwrap_or_default()
    }

    fn glyph(&mut self, font: FontKey, ch: char) -> GlyphInfo {
        let size = font.size_px.max(1);
        let hit = if (ch as usize) < ASCII
            && let Some((_, table)) = self.ascii.iter().find(|(s, _)| *s == size)
            && let Some(g) = table[ch as usize]
        {
            Some(g)
        } else {
            self.glyphs.get(&(size, ch)).copied()
        };
        let hit = match hit {
            Some(g) => Some(g),
            None => {
                self.ensure_font(size);
                self.glyphs.get(&(size, ch)).copied()
            }
        };
        match hit {
            Some(g) => {
                self.packer.touch(g.shelf);
                g.info
            }
            None => self.rasterise(size, ch),
        }
    }

    fn texel_size(&self) -> [f32; 2] {
        [1.0 / BASE_SIZE as f32; 2]
    }

    fn begin_frame(&mut self) {
        if self.events.restored.take() || self.invalidate {
            self.invalidate_now();
        }
        self.packer.begin_frame();
        self.budget.begin_frame();
        self.full = false;
    }
}

/// Stand-in for a glyph not measured yet: one em for wide (East Asian)
/// characters, a little over half an em otherwise.
fn estimate(size: u16, ch: char) -> GlyphInfo {
    let wide = matches!(ch as u32,
        0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFD);
    let em = f32::from(size);
    GlyphInfo {
        advance: if wide { em } else { (em * 0.55).round() },
        ..GlyphInfo::default()
    }
}

fn new_canvas(
    document: &Document,
    size: u32,
) -> Result<(HtmlCanvasElement, CanvasRenderingContext2d), JsValue> {
    let canvas: HtmlCanvasElement = document.create_element("canvas")?.dyn_into()?;
    canvas.set_width(size);
    canvas.set_height(size);
    let ctx: CanvasRenderingContext2d = canvas
        .get_context("2d")?
        .ok_or_else(|| JsValue::from_str("canvas 2D is not available"))?
        .dyn_into()?;
    init_ctx(&ctx);
    Ok((canvas, ctx))
}

fn init_ctx(ctx: &CanvasRenderingContext2d) {
    ctx.set_fill_style_str("#ffffff");
    ctx.set_text_align("left");
    ctx.set_text_baseline("alphabetic");
}
