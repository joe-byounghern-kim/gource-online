// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! The hover tooltip (`textbox.cpp`), in logical pixels.

use glam::{Vec2, Vec4};

use gource_core::view::{SceneView, TextBoxView};

use crate::emit::{DEFAULT_TEXCOORD, quad};
use crate::text::{Font, Mode};
use crate::{GlyphSource, QuadVertex};

/// `TextBox::max_width_chars`.
const MAX_WIDTH_CHARS: usize = 1024;
/// `textbox.setBrightness(0.5f)` in the `Gource` constructor.
const BRIGHTNESS: f32 = 0.5;
/// `TextBox::shadow`.
const SHADOW: Vec2 = Vec2::new(3.0, 3.0);

/// `addLine`'s `str.substr(0, max_width_chars)`, kept on a char boundary.
fn clip(line: &str) -> &str {
    if line.len() <= MAX_WIDTH_CHARS {
        return line;
    }
    let mut end = MAX_WIDTH_CHARS;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}

/// Box rectangle after `setText` / `addLine` / `setPos(mouse, true)`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Layout {
    pub corner: Vec2,
    pub rect_width: i32,
    pub rect_height: i32,
}

/// `setText(lines[0])`, `addLine(lines[1..])`, then `setPos(pos, true)`
/// against a `display` of logical pixels.
pub(crate) fn layout(
    view: &SceneView,
    tb: &TextBoxView,
    pos: Vec2,
    font: &Font,
    glyphs: &mut dyn GlyphSource,
    display: (i32, i32),
) -> Layout {
    // clear()
    let mut rect_width = 0;
    let mut rect_height = 2;
    for &line in &tb.lines {
        let s = clip(view.str(line));
        let width = (font.width(glyphs, s) + 6.0) as i32;
        if width > rect_width {
            rect_width = width;
        }
        rect_height += font.size + 4;
    }

    let mut corner = pos;
    let fontheight = (font.size + 4) as f32;
    let (w, h) = (rect_width as f32, rect_height as f32);

    corner.y -= h;

    if corner.x + w > display.0 as f32 {
        if corner.x - w - fontheight > 0.0 {
            corner.x -= w;
        } else {
            corner.x = display.0 as f32 - w;
        }
    }

    if corner.y < 0.0 {
        corner.y += h + fontheight;
    }
    if corner.y + h > display.1 as f32 {
        corner.y -= h;
    }

    Layout {
        corner,
        rect_width,
        rect_height,
    }
}

/// `TextBox::draw`: shadow and background quads into `quads`, lines into
/// `text`. `scale` converts logical to device pixels.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    view: &SceneView,
    tb: &TextBoxView,
    l: &Layout,
    font: &Font,
    glyphs: &mut dyn GlyphSource,
    scale: f32,
    quads: &mut Vec<QuadVertex>,
    text: &mut Vec<QuadVertex>,
) {
    let alpha = 1.0;
    let dims = Vec2::new(l.rect_width as f32, l.rect_height as f32);

    quad(
        quads,
        (l.corner + SHADOW) * scale,
        dims * scale,
        Vec4::new(0.0, 0.0, 0.0, alpha * 0.333),
        DEFAULT_TEXCOORD,
    );
    quad(
        quads,
        l.corner * scale,
        dims * scale,
        (tb.colour * BRIGHTNESS).extend(alpha),
        DEFAULT_TEXCOORD,
    );

    let white = Vec4::new(1.0, 1.0, 1.0, alpha);
    let x = (l.corner.x as i32 + 2) as f32;
    let mut yinc = 3;
    for &line in &tb.lines {
        let y = (l.corner.y as i32 + yinc) as f32;
        font.draw(
            glyphs,
            text,
            Mode::Immediate,
            x,
            y,
            clip(view.str(line)),
            white,
        );
        yinc += font.size + 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MonospaceGlyphs;
    use glam::Vec3;

    fn tb(view: &mut SceneView, lines: &[&str]) -> TextBoxView {
        let lines = lines.iter().map(|l| view.push_text(l)).collect();
        TextBoxView {
            lines,
            colour: Vec3::new(1.0, 0.5, 0.0),
            pos: Vec2::ZERO,
        }
    }

    #[test]
    fn rect_size_from_widest_line_and_line_count() {
        let mut g = MonospaceGlyphs;
        let font = Font::grab(&mut g, 18, 1.0);
        let mut v = SceneView::default();
        let t = tb(&mut v, &["main.rs", "src/very/long/"]);
        let l = layout(&v, &t, Vec2::new(100.0, 300.0), &font, &mut g, (1280, 720));
        // 14 chars × 10.8 = 151.2 + 6 → 157; 2 + 2 × 22 = 46.
        assert_eq!(l.rect_width, 157);
        assert_eq!(l.rect_height, 46);
        assert_eq!(l.corner, Vec2::new(100.0, 300.0 - 46.0));
    }

    #[test]
    fn placement_flips_at_screen_edges() {
        let mut g = MonospaceGlyphs;
        let font = Font::grab(&mut g, 18, 1.0);
        let mut v = SceneView::default();
        let t = tb(&mut v, &["abcdefghij"]); // 108 + 6 = 114 wide, 24 high
        // Right edge: flips to the left of the mouse.
        let l = layout(&v, &t, Vec2::new(1250.0, 300.0), &font, &mut g, (1280, 720));
        assert_eq!(l.corner, Vec2::new(1250.0 - 114.0, 276.0));
        // Right edge with no room on the left: clamps to the screen.
        let l = layout(&v, &t, Vec2::new(100.0, 300.0), &font, &mut g, (150, 720));
        assert_eq!(l.corner.x, 150.0 - 114.0);
        // Top edge: moves below the mouse by one line height.
        let l = layout(&v, &t, Vec2::new(10.0, 10.0), &font, &mut g, (1280, 720));
        assert_eq!(l.corner.y, 10.0 - 24.0 + 24.0 + 22.0);
    }

    #[test]
    fn draw_emits_shadow_box_and_lines() {
        let mut g = MonospaceGlyphs;
        let font = Font::grab(&mut g, 18, 1.0);
        let mut v = SceneView::default();
        let t = tb(&mut v, &["ab", "c"]);
        let l = layout(&v, &t, Vec2::new(100.5, 300.0), &font, &mut g, (1280, 720));
        let (mut quads, mut text) = (Vec::new(), Vec::new());
        draw(&v, &t, &l, &font, &mut g, 1.0, &mut quads, &mut text);
        assert_eq!(quads.len(), 8);
        assert_eq!(quads[0].pos, [103.5, l.corner.y + 3.0]);
        assert_eq!(quads[0].colour, [0.0, 0.0, 0.0, 0.333]);
        assert_eq!(quads[4].colour, [0.5, 0.25, 0.0, 1.0]);
        assert_eq!(text.len(), 3 * 4);
        // First line at ((int)x + 2, (int)y + 3), align top: + ceil(14.4 - 3.6) = 11.
        let top = (l.corner.y as i32 + 3) as f32 + 11.0 - 18.0 * 0.8;
        assert_eq!(text[0].pos, [102.0, top]);
        // Second line one font size + 4 lower.
        assert!((text[8].pos[1] - (top + 22.0)).abs() < 1e-4);
    }
}
