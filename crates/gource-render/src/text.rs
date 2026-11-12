// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Text layout with `FXFont` semantics (`core/fxfont.cpp`).
//!
//! Coordinates passed to [`Font`] are Gource's (logical, CSS) pixels; glyphs
//! are requested at `size × ui_scale` device pixels and emitted in device
//! pixels, so at `ui_scale = 1` every position is exactly Gource's.

use glam::{Vec2, Vec4};

use gource_core::settings::Settings;
use gource_core::view::FontRole;

use crate::emit::quad;
use crate::{FontKey, FontMetrics, GlyphInfo, GlyphSource, QuadVertex};

/// `FXFont::shadow_offset` default.
const SHADOW_OFFSET: f32 = 1.0;
/// `FXFont::shadow_strength` default.
pub(crate) const SHADOW_STRENGTH: f32 = 0.7;
/// `FXGlyphSet::tab_width`.
const TAB_WIDTH: f32 = 4.0;

/// One `FXFont` value: a glyph set at a size plus the per-copy flags.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Font {
    pub key: FontKey,
    /// `getFontSize()` in logical pixels.
    pub size: i32,
    /// Device pixels per logical pixel.
    pub scale: f32,
    /// Device-pixel metrics of `key`.
    pub metrics: FontMetrics,
    pub shadow: bool,
    pub round: bool,
    pub align_top: bool,
    pub align_right: bool,
}

/// Whether text goes through the font manager's buffer (shadow from the
/// text shader) or is drawn immediately (shadow as a second, offset copy).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Buffered,
    Immediate,
}

impl Font {
    /// `fontmanager.grab(font_file, size)`: flags at `FXFont::init` defaults.
    pub fn grab(glyphs: &mut dyn GlyphSource, size: i32, scale: f32) -> Font {
        let size = size.max(1);
        let px = (size as f32 * scale).round().clamp(1.0, u16::MAX as f32) as u16;
        let key = FontKey { size_px: px };
        Font {
            key,
            size,
            scale,
            metrics: glyphs.metrics(key),
            shadow: false,
            round: false,
            align_top: true,
            align_right: false,
        }
    }

    pub fn with_shadow(mut self, shadow: bool) -> Font {
        self.shadow = shadow;
        self
    }

    pub fn with_round(mut self, round: bool) -> Font {
        self.round = round;
        self
    }

    pub fn with_align_top(mut self, top: bool) -> Font {
        self.align_top = top;
        self
    }

    fn device_width(&self, glyphs: &mut dyn GlyphSource, text: &str) -> f32 {
        let mut width = 0.0;
        for ch in text.chars() {
            width += glyphs.glyph(self.key, ch).advance;
        }
        width
    }

    /// `getWidth(text)`: sum of advances, in logical pixels.
    pub fn width(&self, glyphs: &mut dyn GlyphSource, text: &str) -> f32 {
        self.device_width(glyphs, text) / self.scale
    }

    /// Byte length of the longest prefix of `text` whose [`Font::width`] is
    /// at most `max_width` (advances are never negative, so the prefix
    /// widths only grow). Only the glyphs up to the cut are looked up.
    pub fn fitting_prefix(
        &self,
        glyphs: &mut dyn GlyphSource,
        text: &str,
        max_width: f32,
    ) -> usize {
        let mut width = 0.0;
        if width / self.scale > max_width {
            return 0;
        }
        for (i, ch) in text.char_indices() {
            width += glyphs.glyph(self.key, ch).advance;
            if width / self.scale > max_width {
                return i;
            }
        }
        text.len()
    }

    fn device_height(&self) -> f32 {
        libm::ceilf(self.metrics.ascender + self.metrics.descender)
    }

    /// `getMaxHeight()` in logical pixels.
    pub fn max_height(&self) -> f32 {
        self.metrics.max_height / self.scale
    }

    /// `FXFont::draw(x, y, text)` with the current colour (the shadow colour
    /// is `(0, 0, 0, colour.a × 0.7)`, as `setColour` / `setAlpha` keep it).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        glyphs: &mut dyn GlyphSource,
        out: &mut Vec<QuadVertex>,
        mode: Mode,
        x: f32,
        y: f32,
        text: &str,
        colour: Vec4,
    ) {
        let mut x = x * self.scale;
        let mut y = y * self.scale;

        if self.align_right {
            x -= self.device_width(glyphs, text);
        }
        if self.align_top {
            y += self.device_height();
        }
        if self.round {
            x = x.round();
            y = y.round();
        }

        // Buffered fonts get their shadow from the text shader.
        if self.shadow && mode == Mode::Immediate {
            let o = SHADOW_OFFSET * self.scale;
            let shadow = Vec4::new(0.0, 0.0, 0.0, colour.w * SHADOW_STRENGTH);
            self.render(glyphs, out, mode, Vec2::new(x + o, y + o), text, shadow);
        }
        self.render(glyphs, out, mode, Vec2::new(x, y), text, colour);
    }

    /// `FXGlyphSet::drawToVBO` (buffered) or `FXGlyphSet::draw` (immediate,
    /// which also expands tabs to four `M` advances).
    fn render(
        &self,
        glyphs: &mut dyn GlyphSource,
        out: &mut Vec<QuadVertex>,
        mode: Mode,
        mut pen: Vec2,
        text: &str,
        colour: Vec4,
    ) {
        for ch in text.chars() {
            if ch == '\t' && mode == Mode::Immediate {
                pen.x += glyphs.glyph(self.key, 'M').advance * TAB_WIDTH;
                continue;
            }
            let g = glyphs.glyph(self.key, ch);
            glyph_quad(out, pen, &g, colour);
            pen.x += g.advance;
        }
    }
}

/// `FXGlyph::drawToVBO`: the bitmap at `pen + bearing`. Empty bitmaps (space)
/// draw nothing visible, so they are skipped.
fn glyph_quad(out: &mut Vec<QuadVertex>, pen: Vec2, g: &GlyphInfo, colour: Vec4) {
    if g.size[0] <= 0.0 || g.size[1] <= 0.0 {
        return;
    }
    quad(
        out,
        pen + Vec2::from(g.bearing),
        Vec2::from(g.size),
        colour,
        [g.uv_min[0], g.uv_min[1], g.uv_max[0], g.uv_max[1]],
    );
}

/// `FontRole` → logical pixel size, from Gource's `fontmanager.grab` calls.
pub(crate) fn role_size(role: FontRole, settings: &Settings) -> i32 {
    let fs = settings.font_scale;
    match role {
        FontRole::DirName => settings.scaled_dirname_font_size(),
        FontRole::FileName => settings.scaled_filename_font_size(),
        FontRole::FileSelected | FontRole::UserSelected => 18,
        FontRole::UserName => settings.scaled_user_font_size(),
        FontRole::Medium | FontRole::Key => settings.scaled_font_size(),
        FontRole::Small => (14.0 * fs) as i32,
        FontRole::Large => (42.0 * fs) as i32,
        FontRole::TextBox => (18.0 * fs) as i32,
    }
}

/// The `FXFont` Gource draws a role with, flags as set where it is grabbed
/// (and, for `fontdirname`, as `Gource::draw` resets them).
pub(crate) fn role_font(
    role: FontRole,
    settings: &Settings,
    scale: f32,
    glyphs: &mut dyn GlyphSource,
) -> Font {
    let f = Font::grab(glyphs, role_size(role, settings), scale);
    match role {
        // fontdirname: dropShadow(true), roundCoordinates(false) in draw().
        // file_font / file_selected_font: dropShadow(true), roundCoordinates(false).
        FontRole::DirName | FontRole::FileName | FontRole::FileSelected => f.with_shadow(true),
        // RUser::updateFont: dropShadow(true), alignTop(false).
        FontRole::UserName | FontRole::UserSelected => f.with_shadow(true).with_align_top(false),
        // fontmedium: dropShadow(true), roundCoordinates(false).
        FontRole::Medium => f.with_shadow(true),
        // font / fontlarge: dropShadow(true), roundCoordinates(true).
        FontRole::Small | FontRole::Large => f.with_shadow(true).with_round(true),
        // TextBox and FileKey fonts: no shadow, no rounding.
        FontRole::TextBox | FontRole::Key => f,
    }
}

/// Every role's font for one frame.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Fonts {
    pub dir_name: Font,
    pub file_name: Font,
    pub file_selected: Font,
    pub user_name: Font,
    pub user_selected: Font,
    pub medium: Font,
    pub small: Font,
    pub large: Font,
    pub text_box: Font,
    pub key: Font,
}

impl Fonts {
    pub fn new(settings: &Settings, scale: f32, glyphs: &mut dyn GlyphSource) -> Fonts {
        let mut f = |role| role_font(role, settings, scale, glyphs);
        Fonts {
            dir_name: f(FontRole::DirName),
            file_name: f(FontRole::FileName),
            file_selected: f(FontRole::FileSelected),
            user_name: f(FontRole::UserName),
            user_selected: f(FontRole::UserSelected),
            medium: f(FontRole::Medium),
            small: f(FontRole::Small),
            large: f(FontRole::Large),
            text_box: f(FontRole::TextBox),
            key: f(FontRole::Key),
        }
    }

    pub fn get(&self, role: FontRole) -> &Font {
        match role {
            FontRole::DirName => &self.dir_name,
            FontRole::FileName => &self.file_name,
            FontRole::FileSelected => &self.file_selected,
            FontRole::UserName => &self.user_name,
            FontRole::UserSelected => &self.user_selected,
            FontRole::Medium => &self.medium,
            FontRole::Small => &self.small,
            FontRole::Large => &self.large,
            FontRole::TextBox => &self.text_box,
            FontRole::Key => &self.key,
        }
    }
}

/// Format an integer without allocating (`printf("%d")`).
pub(crate) fn format_int(value: i64, buf: &mut [u8; 24]) -> &str {
    let mut i = buf.len();
    let negative = value < 0;
    let mut v = value.unsigned_abs();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    if negative {
        i -= 1;
        buf[i] = b'-';
    }
    // Only ASCII digits and '-' were written.
    std::str::from_utf8(&buf[i..]).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MonospaceGlyphs;

    #[test]
    fn align_top_adds_ceil_ascender_plus_descender() {
        let mut g = MonospaceGlyphs;
        let f = Font::grab(&mut g, 14, 1.0);
        let mut out = Vec::new();
        f.draw(
            &mut g,
            &mut out,
            Mode::Buffered,
            10.0,
            20.0,
            "ab",
            Vec4::ONE,
        );
        // ascender 11.2, descender -2.8 → ceil(8.4) = 9; bearing -11.2.
        assert_eq!(out.len(), 8);
        assert_eq!(out[0].pos, [10.0, 20.0 + 9.0 - 14.0 * 0.8]);
        // Second glyph one advance (8.4) to the right.
        assert!((out[4].pos[0] - (10.0 + 14.0 * 0.6)).abs() < 1e-4);
    }

    #[test]
    fn immediate_shadow_is_offset_copy_drawn_first() {
        let mut g = MonospaceGlyphs;
        let f = Font::grab(&mut g, 10, 1.0)
            .with_shadow(true)
            .with_align_top(false);
        let mut out = Vec::new();
        f.draw(
            &mut g,
            &mut out,
            Mode::Immediate,
            5.0,
            5.0,
            "x",
            Vec4::new(1.0, 0.5, 0.0, 0.5),
        );
        assert_eq!(out.len(), 8);
        assert_eq!(out[0].colour, [0.0, 0.0, 0.0, 0.35]);
        assert_eq!(out[0].pos[0], 6.0);
        assert_eq!(out[4].colour, [1.0, 0.5, 0.0, 0.5]);
        assert_eq!(out[4].pos[0], 5.0);

        out.clear();
        f.draw(&mut g, &mut out, Mode::Buffered, 5.0, 5.0, "x", Vec4::ONE);
        assert_eq!(out.len(), 4, "buffered text has no geometric shadow");
    }

    #[test]
    fn round_and_align_right() {
        let mut g = MonospaceGlyphs;
        let mut f = Font::grab(&mut g, 10, 1.0)
            .with_round(true)
            .with_align_top(false);
        f.align_right = true;
        let mut out = Vec::new();
        f.draw(&mut g, &mut out, Mode::Buffered, 20.4, 7.6, "ab", Vec4::ONE);
        // x = round(20.4 - 12) = 8, y = round(7.6) = 8, glyph top at y - 8.
        assert_eq!(out[0].pos, [8.0, 0.0]);
    }

    #[test]
    fn ui_scale_scales_positions_and_glyph_size() {
        let mut g = MonospaceGlyphs;
        let f = Font::grab(&mut g, 10, 2.0).with_align_top(false);
        assert_eq!(f.key.size_px, 20);
        assert!((f.width(&mut g, "abc") - 18.0).abs() < 1e-4);
        assert_eq!(f.max_height(), 10.0);
        let mut out = Vec::new();
        f.draw(&mut g, &mut out, Mode::Buffered, 10.0, 30.0, "a", Vec4::ONE);
        assert_eq!(out[0].pos, [20.0, 60.0 - 16.0]);
        assert_eq!(out[2].pos, [32.0, 60.0 + 4.0]);
    }

    #[test]
    fn immediate_tabs_expand_to_four_m_advances() {
        let mut g = MonospaceGlyphs;
        let f = Font::grab(&mut g, 10, 1.0).with_align_top(false);
        let mut out = Vec::new();
        f.draw(
            &mut g,
            &mut out,
            Mode::Immediate,
            0.0,
            0.0,
            "\tx",
            Vec4::ONE,
        );
        assert_eq!(out.len(), 4);
        assert!((out[0].pos[0] - 24.0).abs() < 1e-4);
    }

    #[test]
    fn role_sizes_follow_gource_grabs() {
        let mut s = Settings {
            font_scale: 1.5,
            ..Settings::default()
        };
        assert_eq!(role_size(FontRole::Small, &s), 21);
        assert_eq!(role_size(FontRole::Large, &s), 63);
        assert_eq!(role_size(FontRole::TextBox, &s), 27);
        assert_eq!(role_size(FontRole::FileSelected, &s), 18);
        assert_eq!(role_size(FontRole::Medium, &s), 24);
        s.font_scale = 1.0;
        assert_eq!(role_size(FontRole::DirName, &s), 14);
    }

    #[test]
    fn format_int_matches_printf() {
        let mut b = [0u8; 24];
        assert_eq!(format_int(0, &mut b), "0");
        assert_eq!(format_int(1234, &mut b), "1234");
        assert_eq!(format_int(-56, &mut b), "-56");
        assert_eq!(format_int(i64::MIN, &mut b), "-9223372036854775808");
    }
}
