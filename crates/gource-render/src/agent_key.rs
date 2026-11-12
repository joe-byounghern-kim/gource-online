// SPDX-License-Identifier: GPL-3.0-or-later

//! The agent key (AI-59) and the AI filter line (AI-74): text only, drawn
//! into the HUD text stream with the date, so neither adds a quad or a draw
//! command.
//!
//! The key sits at the top right, its first row at `KEY_TOP` (below the
//! web page's toolbar, which covers the top right corner of the live view
//! down to about y = 56; recordings have no toolbar and keep the same
//! layout), one row per agent: a
//! bullet in the agent's colour, the name, and the number of played commits
//! crediting it, right-aligned; then "N more" when agents were left out.
//! Rows do not move or animate. Layout is in logical pixels, like the rest
//! of the HUD.

use glam::Vec4;

use gource_core::view::SceneView;

use crate::GlyphSource;
use crate::QuadVertex;
use crate::text::{Font, Mode, format_int};

/// Top of the date line (`Gource::draw` draws the date at y = 20).
pub(crate) const DATE_Y: f32 = 20.0;
/// Top of the key's first row, in logical pixels: clear of the page's
/// top-right toolbar (CSS y 12 to 56, logical pixels are CSS pixels).
pub(crate) const KEY_TOP: f32 = 68.0;
/// Right margin of the key.
const RIGHT_MARGIN: f32 = 20.0;
/// Space between the name and the count columns.
const COLUMN_GAP: f32 = 10.0;
/// The bullet in front of each agent name. `GourceSans` falls back across
/// fonts for it in the browser.
pub(crate) const BULLET: &str = "\u{25CF} ";

/// `n` with thousands separators (`1,204`), without allocating.
pub(crate) fn format_count(n: u32, buf: &mut [u8; 24]) -> &str {
    let mut digits = [0u8; 24];
    let plain = format_int(n as i64, &mut digits).as_bytes();
    let mut len = 0;
    for (i, &d) in plain.iter().enumerate() {
        if i > 0 && (plain.len() - i).is_multiple_of(3) {
            buf[len] = b',';
            len += 1;
        }
        buf[len] = d;
        len += 1;
    }
    std::str::from_utf8(&buf[..len]).unwrap_or("")
}

/// `"<n> more"` into `buf`.
fn format_more(n: u32, buf: &mut [u8; 24]) -> &str {
    let mut digits = [0u8; 24];
    let d = format_int(n as i64, &mut digits).as_bytes();
    let tail = b" more";
    buf[..d.len()].copy_from_slice(d);
    buf[d.len()..d.len() + tail.len()].copy_from_slice(tail);
    std::str::from_utf8(&buf[..d.len() + tail.len()]).unwrap_or("")
}

/// The unchanged key's leftmost logical coordinate, including its bullet.
/// Rich HUD text reserves this footprint without moving or evicting F1 rows.
pub(crate) fn left_edge(
    view: &SceneView,
    font: &Font,
    glyphs: &mut dyn GlyphSource,
    display_width: f32,
) -> Option<f32> {
    let hud = &view.hud;
    if !hud.show_agent_key || (hud.agents.is_empty() && hud.agents_more == 0) {
        return None;
    }
    let mut buf = [0u8; 24];
    let mut name_w: f32 = 0.0;
    let mut count_w: f32 = 0.0;
    for row in &hud.agents {
        name_w = name_w.max(font.width(glyphs, view.str(row.name)));
        count_w = count_w.max(font.width(glyphs, format_count(row.count, &mut buf)));
    }
    if hud.agents_more > 0 {
        name_w = name_w.max(font.width(glyphs, format_more(hud.agents_more, &mut buf)));
    }
    Some(display_width - RIGHT_MARGIN - count_w - COLUMN_GAP - name_w - font.width(glyphs, BULLET))
}

/// Draw the agent key rows of `view.hud`; `row_height` is
/// `scaled_font_size + 6`, `display_width` the logical width.
pub(crate) fn draw(
    view: &SceneView,
    font: &Font,
    glyphs: &mut dyn GlyphSource,
    display_width: f32,
    row_height: f32,
    font_colour: Vec4,
    out: &mut Vec<QuadVertex>,
) {
    let hud = &view.hud;
    if !hud.show_agent_key || (hud.agents.is_empty() && hud.agents_more == 0) {
        return;
    }
    let mut buf = [0u8; 24];
    let mut name_w: f32 = 0.0;
    let mut count_w: f32 = 0.0;
    for row in &hud.agents {
        name_w = name_w.max(font.width(glyphs, view.str(row.name)));
        count_w = count_w.max(font.width(glyphs, format_count(row.count, &mut buf)));
    }
    if hud.agents_more > 0 {
        name_w = name_w.max(font.width(glyphs, format_more(hud.agents_more, &mut buf)));
    }
    let right = display_width - RIGHT_MARGIN;
    let name_x = right - count_w - COLUMN_GAP - name_w;
    let bullet_x = name_x - font.width(glyphs, BULLET);

    let mut y = KEY_TOP;
    for row in &hud.agents {
        let c = row.colour;
        font.draw(
            glyphs,
            out,
            Mode::Immediate,
            bullet_x,
            y,
            BULLET,
            Vec4::new(c.x, c.y, c.z, font_colour.w),
        );
        font.draw(
            glyphs,
            out,
            Mode::Immediate,
            name_x,
            y,
            view.str(row.name),
            font_colour,
        );
        let count = format_count(row.count, &mut buf);
        let x = right - font.width(glyphs, count);
        font.draw(glyphs, out, Mode::Immediate, x, y, count, font_colour);
        y += row_height;
    }
    if hud.agents_more > 0 {
        let more = format_more(hud.agents_more, &mut buf);
        font.draw(glyphs, out, Mode::Immediate, name_x, y, more, font_colour);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_get_thousands_separators() {
        let mut buf = [0u8; 24];
        for (n, s) in [
            (0, "0"),
            (87, "87"),
            (311, "311"),
            (1204, "1,204"),
            (1_000_000, "1,000,000"),
            (u32::MAX, "4,294,967,295"),
        ] {
            assert_eq!(format_count(n, &mut buf), s);
        }
        assert_eq!(format_more(3, &mut buf), "3 more");
    }
}
