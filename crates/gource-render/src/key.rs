// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! The file-extension key (`key.cpp`: `FileKeyEntry`, `FileKey`).
//!
//! Gource increments and decrements per-extension counters as files come and
//! go; the scene view carries the resulting counts, and [`FileKey::sync`]
//! turns them back into the same entry lifecycle (an entry is created on the
//! first positive count and kept until it has faded out). Layout is in
//! logical pixels; [`FileKey::draw`] scales to device pixels.

use std::cmp::Ordering;

use glam::{Vec2, Vec3, Vec4};

use gource_core::view::{ExtensionCount, SceneView};

use crate::GlyphSource;
use crate::QuadVertex;
use crate::emit::{DEFAULT_TEXCOORD, quad, vertex};
use crate::text::{Font, Mode, format_int};

/// `FileKey(1.0f)` in the `Gource` constructor.
const UPDATE_INTERVAL: f32 = 1.0;

#[derive(Clone, Debug, Default)]
pub(crate) struct KeyEntry {
    pub ext: String,
    pub display_ext: String,
    pub colour: Vec3,
    pub alpha: f32,
    pub count: i32,
    pub pos_y: f32,
    pub src_y: f32,
    pub dest_y: f32,
    pub move_elapsed: f32,
    pub left_margin: f32,
    pub width: f32,
    pub height: f32,
    pub pos: Vec2,
    pub shadow: Vec2,
    pub show: bool,
    /// Scratch: seen in this frame's extension list.
    seen: bool,
    /// `display_ext` is still to be measured (deferred while the key is
    /// hidden).
    label_pending: bool,
}

impl KeyEntry {
    /// `FileKeyEntry(font, ext, colour)`, reusing `self`'s string buffers.
    /// The truncated label is measured by [`KeyEntry::measure_label`].
    fn init(&mut self, ext: &str, colour: Vec3, font_scale: f32, scaled_font_size: i32) {
        self.ext.clear();
        self.ext.push_str(ext);
        self.colour = colour;
        self.pos_y = -1.0;
        self.shadow = Vec2::new(3.0, 3.0);
        self.width = 90.0 * font_scale;
        self.height = scaled_font_size as f32 + 4.0;
        self.left_margin = scaled_font_size as f32 + 4.0;
        self.count = 0;
        self.alpha = 0.0;
        self.move_elapsed = 1.0;
        self.src_y = -1.0;
        self.dest_y = -1.0;
        self.pos = Vec2::ZERO;
        self.show = true;
        self.seen = false;
        self.display_ext.clear();
        self.label_pending = true;
    }

    /// The label part of `FileKeyEntry`'s constructor: `ext`, cut to
    /// `width - 15 × font_scale` plus `"..."`. Gource pops one character
    /// and measures the whole rest again until it fits (quadratic, seconds
    /// for a hostile 20000-character extension); the widths of the prefixes
    /// are the same left-to-right sums, so one pass that stops at the first
    /// character overflowing the width cuts at the same place.
    fn measure_label(&mut self, font: &Font, glyphs: &mut dyn GlyphSource, font_scale: f32) {
        self.label_pending = false;
        let max_width = self.width - 15.0 * font_scale;
        let cut = font.fitting_prefix(glyphs, &self.ext, max_width);
        self.display_ext.clear();
        self.display_ext.push_str(&self.ext[..cut]);
        if cut < self.ext.len() {
            self.display_ext.push_str("...");
        }
    }

    /// `isFinished()`.
    pub fn is_finished(&self) -> bool {
        self.count <= 0 && self.alpha <= 0.0
    }

    /// `setDestY`.
    fn set_dest_y(&mut self, dest_y: f32) {
        if dest_y == self.dest_y {
            return;
        }
        self.dest_y = dest_y;
        self.src_y = self.pos_y;
        self.move_elapsed = 0.0;
    }

    /// `FileKeyEntry::logic`.
    fn logic(&mut self, dt: f32) {
        if self.count <= 0 || !self.show {
            self.alpha = (self.alpha - dt).max(0.0);
        } else if self.alpha < 1.0 {
            self.alpha = (self.alpha + dt).min(1.0);
        }

        if self.pos_y != self.dest_y {
            if self.pos_y < 0.0 {
                self.pos_y = self.dest_y;
            } else {
                self.move_elapsed += dt;
                if self.move_elapsed >= 1.0 {
                    self.pos_y = self.dest_y;
                } else {
                    self.pos_y = self.src_y + (self.dest_y - self.src_y) * self.move_elapsed;
                }
            }
        }

        self.pos = Vec2::new(self.alpha * self.left_margin, self.pos_y);
    }
}

/// `file_key_entry_sort`: by count descending, then extension ascending.
fn entry_order(a: &KeyEntry, b: &KeyEntry) -> Ordering {
    b.count.cmp(&a.count).then_with(|| a.ext.cmp(&b.ext))
}

#[derive(Debug)]
pub(crate) struct FileKey {
    /// `keymap` (unordered; lookups go through `index`).
    entries: Vec<KeyEntry>,
    /// Entry indices sorted by extension.
    index: Vec<u32>,
    /// `active_keys`, as indices into `entries`.
    active: Vec<u32>,
    /// Deleted entries kept for their string buffers.
    pool: Vec<KeyEntry>,
    interval_remaining: f32,
    show: bool,
}

impl Default for FileKey {
    fn default() -> Self {
        FileKey {
            entries: Vec::new(),
            index: Vec::new(),
            active: Vec::new(),
            pool: Vec::new(),
            interval_remaining: 1.0,
            show: true,
        }
    }
}

impl FileKey {
    fn find(&self, ext: &str) -> Result<usize, usize> {
        self.index
            .binary_search_by(|&i| self.entries[i as usize].ext.as_str().cmp(ext))
    }

    /// Apply this frame's extension counts: `inc` creates missing entries,
    /// counts not listed drop to zero (as `dec` / `clear` leave them).
    /// Labels are measured only while the key is shown (`logic` adds
    /// entries to the drawn list only then).
    pub fn sync(
        &mut self,
        view: &SceneView,
        extensions: &[ExtensionCount],
        font: &Font,
        glyphs: &mut dyn GlyphSource,
        font_scale: f32,
        scaled_font_size: i32,
    ) {
        for e in &mut self.entries {
            e.seen = false;
        }
        for ext in extensions {
            let name = view.str(ext.ext);
            let count = i32::try_from(ext.count).unwrap_or(i32::MAX);
            match self.find(name) {
                Ok(slot) => {
                    let e = &mut self.entries[self.index[slot] as usize];
                    e.count = count;
                    e.colour = ext.colour;
                    e.seen = true;
                }
                Err(slot) if count > 0 => {
                    let mut e = self.pool.pop().unwrap_or_default();
                    e.init(name, ext.colour, font_scale, scaled_font_size);
                    e.count = count;
                    e.seen = true;
                    self.index.insert(slot, self.entries.len() as u32);
                    self.entries.push(e);
                }
                Err(_) => {}
            }
        }
        for e in &mut self.entries {
            if !e.seen {
                e.count = 0;
            }
            if self.show && e.label_pending {
                e.measure_label(font, glyphs, font_scale);
            }
        }
        // Grow the bookkeeping now, while entries are being created, so the
        // later recompute and deletion never allocate.
        let n = self.entries.len();
        self.active.reserve(n.saturating_sub(self.active.len()));
        self.pool.reserve(n);
    }

    /// `FileKey::logic(dt)`; `display_height` in logical pixels.
    pub fn logic(&mut self, dt: f32, display_height: i32, scaled_font_size: i32) {
        self.interval_remaining -= dt;

        if self.interval_remaining <= 0.0 {
            if self.show {
                // Remove finished entries first (Gource deletes them after
                // building active_keys, which never contains them).
                let mut i = 0;
                let mut removed = false;
                while i < self.entries.len() {
                    if self.entries[i].is_finished() {
                        let e = self.entries.swap_remove(i);
                        self.pool.push(e);
                        removed = true;
                    } else {
                        i += 1;
                    }
                }
                if removed {
                    self.index.clear();
                    self.index.extend(0..self.entries.len() as u32);
                    let entries = &self.entries;
                    self.index.sort_unstable_by(|&a, &b| {
                        entries[a as usize].ext.cmp(&entries[b as usize].ext)
                    });
                }

                self.active.clear();
                self.active.extend(0..self.entries.len() as u32);
                let entries = &self.entries;
                self.active.sort_unstable_by(|&a, &b| {
                    entry_order(&entries[a as usize], &entries[b as usize])
                });

                let max_visible = (((display_height as f32 - 150.0) / 20.0) as i32).max(0) as usize;
                self.active.truncate(max_visible);

                let offset_y = scaled_font_size as f32 + 6.0;
                let mut key_y = offset_y;
                for &i in &self.active {
                    let e = &mut self.entries[i as usize];
                    if e.count > 0 {
                        e.set_dest_y(key_y);
                    }
                    key_y += offset_y;
                }
            }
            self.interval_remaining = UPDATE_INTERVAL;
        }

        for &i in &self.active {
            self.entries[i as usize].logic(dt);
        }
    }

    /// `FileKey::setShow` (Gource calls it after every draw, which also
    /// forces the next `logic` to recompute the active entries).
    pub fn set_show(&mut self, show: bool) {
        self.show = show;
        for &i in &self.active {
            self.entries[i as usize].show = show;
        }
        self.interval_remaining = 0.0;
    }

    /// `FileKey::draw`: per entry a shadow quad and a gradient quad into
    /// `quads`, the extension and the count into `text`. `scale` converts
    /// logical to device pixels.
    pub fn draw(
        &self,
        font: &Font,
        glyphs: &mut dyn GlyphSource,
        scale: f32,
        quads: &mut Vec<QuadVertex>,
        text: &mut Vec<QuadVertex>,
    ) {
        for &i in &self.active {
            let e = &self.entries[i as usize];
            if e.is_finished() {
                continue;
            }
            let (x, y, w, h) = (e.pos.x, e.pos.y, e.width, e.height);

            let s = e.shadow;
            quad(
                quads,
                Vec2::new(x + s.x, y + s.y) * scale,
                Vec2::new(w, h) * scale,
                Vec4::new(0.0, 0.0, 0.0, e.alpha * 0.333),
                DEFAULT_TEXCOORD,
            );

            let dark = (e.colour * 0.5).extend(e.alpha);
            let full = e.colour.extend(e.alpha);
            quads.extend_from_slice(&[
                vertex(Vec2::new(x, y) * scale, dark, [0.0, 0.0]),
                vertex(Vec2::new(x, y + h) * scale, dark, [0.0, 0.0]),
                vertex(Vec2::new(x + w, y + h) * scale, full, [0.0, 0.0]),
                vertex(Vec2::new(x + w, y) * scale, full, [0.0, 0.0]),
            ]);

            let white = Vec4::new(1.0, 1.0, 1.0, e.alpha);
            let tx = (x as i32) as f32;
            let ty = (y as i32 + 3) as f32;
            font.with_shadow(false).draw(
                glyphs,
                text,
                Mode::Immediate,
                tx + 2.0,
                ty,
                &e.display_ext,
                white,
            );
            let mut buf = [0u8; 24];
            let count = format_int(e.count as i64, &mut buf);
            font.with_shadow(true).draw(
                glyphs,
                text,
                Mode::Immediate,
                tx + w + 4.0,
                ty,
                count,
                white,
            );
        }
    }

    /// Active entries in draw order (for tests).
    #[cfg(test)]
    pub fn active_entries(&self) -> impl Iterator<Item = &KeyEntry> {
        self.active.iter().map(|&i| &self.entries[i as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MonospaceGlyphs;

    struct Rig {
        key: FileKey,
        view: SceneView,
        glyphs: CountingGlyphs,
        font: Font,
        show: bool,
    }

    /// Advances that vary per character (`ch % 7 + 3` px), counting lookups.
    #[derive(Default)]
    struct CountingGlyphs {
        lookups: usize,
    }

    impl GlyphSource for CountingGlyphs {
        fn metrics(&mut self, font: crate::FontKey) -> crate::FontMetrics {
            MonospaceGlyphs.metrics(font)
        }

        fn glyph(&mut self, font: crate::FontKey, ch: char) -> crate::GlyphInfo {
            self.lookups += 1;
            crate::GlyphInfo {
                advance: (ch as u32 % 7 + 3) as f32,
                ..MonospaceGlyphs.glyph(font, ch)
            }
        }

        fn texel_size(&self) -> [f32; 2] {
            MonospaceGlyphs.texel_size()
        }
    }

    impl Rig {
        fn new() -> Rig {
            let mut glyphs = CountingGlyphs::default();
            let font = Font::grab(&mut glyphs, 16, 1.0);
            Rig {
                key: FileKey::default(),
                view: SceneView::default(),
                glyphs,
                font,
                show: true,
            }
        }

        /// One Gource frame: counts change, logic(dt), draw, setShow.
        fn frame(&mut self, counts: &[(&str, u32)], dt: f32, height: i32) {
            self.view.clear();
            for &(ext, count) in counts {
                let r = self.view.push_text(ext);
                self.view.hud.extensions.push(ExtensionCount {
                    ext: r,
                    colour: Vec3::ONE,
                    count,
                });
            }
            let exts = std::mem::take(&mut self.view.hud.extensions);
            self.key
                .sync(&self.view, &exts, &self.font, &mut self.glyphs, 1.0, 16);
            self.view.hud.extensions = exts;
            self.key.logic(dt, height, 16);
            self.key.set_show(self.show);
        }

        fn order(&self) -> Vec<(String, i32)> {
            self.key
                .active_entries()
                .map(|e| (e.ext.clone(), e.count))
                .collect()
        }
    }

    #[test]
    fn first_frame_waits_for_interval_then_sorts_by_count_then_name() {
        let mut r = Rig::new();
        let counts = [("rs", 3), ("c", 5), ("h", 3), ("md", 1)];
        r.frame(&counts, 0.1, 720);
        assert!(r.order().is_empty(), "interval starts at 1 s");
        r.frame(&counts, 0.1, 720);
        assert_eq!(
            r.order(),
            [
                ("c".to_string(), 5),
                ("h".to_string(), 3),
                ("rs".to_string(), 3),
                ("md".to_string(), 1)
            ]
        );
        let ys: Vec<f32> = r.key.active_entries().map(|e| e.dest_y).collect();
        assert_eq!(ys, [22.0, 44.0, 66.0, 88.0]);
    }

    #[test]
    fn entries_fade_in_and_slide_between_slots() {
        let mut r = Rig::new();
        r.frame(&[("a", 1), ("b", 2)], 0.0, 720);
        r.frame(&[("a", 1), ("b", 2)], 0.25, 720);
        let a = r
            .key
            .active_entries()
            .find(|e| e.ext == "a")
            .unwrap()
            .clone();
        // New entry jumps to its slot and fades in at 1/s.
        assert_eq!(a.pos_y, 44.0);
        assert_eq!(a.alpha, 0.25);
        assert_eq!(a.pos, Vec2::new(0.25 * 20.0, 44.0));

        // "a" overtakes "b": it slides from 44 to 22 over one second.
        r.frame(&[("a", 3), ("b", 2)], 0.5, 720);
        let a = r.key.active_entries().next().unwrap().clone();
        assert_eq!(a.ext, "a");
        assert_eq!(a.src_y, 44.0);
        assert_eq!(a.dest_y, 22.0);
        assert_eq!(a.pos_y, 44.0 + (22.0 - 44.0) * 0.5);
        assert_eq!(a.alpha, 0.75);
        r.frame(&[("a", 3), ("b", 2)], 0.6, 720);
        let a = r.key.active_entries().next().unwrap().clone();
        assert_eq!(a.pos_y, 22.0);
        assert_eq!(a.alpha, 1.0);
    }

    #[test]
    fn removed_extension_fades_out_and_is_deleted() {
        let mut r = Rig::new();
        r.frame(&[("a", 1), ("b", 1)], 0.0, 720);
        for _ in 0..3 {
            r.frame(&[("a", 1), ("b", 1)], 0.5, 720);
        }
        r.frame(&[("a", 1)], 0.5, 720);
        let b = r
            .key
            .active_entries()
            .find(|e| e.ext == "b")
            .unwrap()
            .clone();
        assert_eq!(b.count, 0);
        assert_eq!(b.alpha, 0.5);
        r.frame(&[("a", 1)], 0.5, 720);
        r.frame(&[("a", 1)], 0.5, 720);
        assert_eq!(r.order(), [("a".to_string(), 1)]);
        assert_eq!(r.key.pool.len(), 1);
        // Re-adding reuses the pooled entry.
        r.frame(&[("a", 1), ("zz", 4)], 0.5, 720);
        assert!(r.key.pool.is_empty());
        assert_eq!(r.order()[0], ("zz".to_string(), 4));
    }

    #[test]
    fn visible_entries_limited_by_display_height() {
        let mut r = Rig::new();
        let names: Vec<String> = (0..20).map(|i| format!("e{i:02}")).collect();
        let counts: Vec<(&str, u32)> = names.iter().map(|n| (n.as_str(), 1)).collect();
        r.frame(&counts, 0.0, 300);
        r.frame(&counts, 0.0, 300);
        // (300 - 150) / 20 = 7.5 → 7.
        assert_eq!(r.order().len(), 7);
        assert_eq!(r.order()[0].0, "e00");
    }

    #[test]
    fn long_extensions_are_truncated_with_ellipsis() {
        let mut glyphs = MonospaceGlyphs;
        let font = Font::grab(&mut glyphs, 16, 1.0);
        let mut e = KeyEntry::default();
        e.init("averyveryverylongextension", Vec3::ONE, 1.0, 16);
        e.measure_label(&font, &mut glyphs, 1.0);
        // 16 px monospace: 9.6 px per char, limit 90 - 15 = 75 → 7 chars.
        assert_eq!(e.display_ext, "averyve...");
    }

    /// `FileKeyEntry`'s loop: pop a character, measure the rest again.
    fn gource_truncation(font: &Font, glyphs: &mut dyn GlyphSource, ext: &str, max: f32) -> String {
        let mut s = ext.to_string();
        let mut truncated = false;
        while !s.is_empty() && font.width(glyphs, &s) > max {
            s.pop();
            truncated = true;
        }
        if truncated {
            s.push_str("...");
        }
        s
    }

    #[test]
    fn one_pass_truncation_cuts_where_gource_does() {
        let mut glyphs = CountingGlyphs::default();
        for (size, scale) in [(16, 1.0), (12, 1.5), (16, 2.0), (14, 0.7)] {
            let font = Font::grab(&mut glyphs, size, scale);
            for font_scale in [1.0, 0.5, 2.0, -1.0] {
                for ext in [
                    "",
                    "c",
                    "rs",
                    "markdown",
                    "üñíçødé-ext",
                    "日本語のファイル拡張子",
                    "averyveryverylongextension",
                ] {
                    let mut e = KeyEntry::default();
                    e.init(ext, Vec3::ONE, font_scale, size);
                    e.measure_label(&font, &mut glyphs, font_scale);
                    let max = e.width - 15.0 * font_scale;
                    assert_eq!(
                        e.display_ext,
                        gource_truncation(&font, &mut glyphs, ext, max),
                        "{ext} {size} {scale} {font_scale}"
                    );
                }
            }
        }
    }

    /// A hostile 100000-character extension costs a few glyph lookups, and
    /// none at all while the key is hidden.
    #[test]
    fn labels_are_measured_once_shown_and_only_up_to_the_cut() {
        let long = "x".repeat(100_000);
        let mut r = Rig::new();
        r.show = false;
        // hidden since the previous frame (the very first frame measures:
        // the key starts shown, as FileKey's constructor leaves it)
        r.key.set_show(false);
        let before = r.glyphs.lookups;
        for _ in 0..4 {
            r.frame(&[(long.as_str(), 1), ("rs", 2)], 0.5, 720);
        }
        assert_eq!(r.glyphs.lookups, before, "hidden key: nothing measured");
        assert!(r.key.entries.iter().all(|e| e.label_pending));

        r.show = true;
        r.key.set_show(true);
        r.frame(&[(long.as_str(), 1), ("rs", 2)], 0.5, 720);
        assert!(
            r.glyphs.lookups - before < 32,
            "{}",
            r.glyphs.lookups - before
        );
        let e = r
            .key
            .entries
            .iter()
            .find(|e| e.ext.len() == 100_000)
            .unwrap();
        assert!(e.display_ext == format!("{}...", "x".repeat(18)));
        assert!(r.key.active_entries().count() == 2);
    }

    #[test]
    fn draw_emits_shadow_gradient_and_texts() {
        let mut r = Rig::new();
        r.frame(&[("rs", 12)], 0.0, 720);
        r.frame(&[("rs", 12)], 1.0, 720);
        let mut quads = Vec::new();
        let mut text = Vec::new();
        r.key
            .draw(&r.font, &mut r.glyphs, 1.0, &mut quads, &mut text);
        assert_eq!(quads.len(), 8);
        // Entry at x = alpha × left_margin = 20, y = 22; shadow offset 3.
        assert_eq!(quads[0].pos, [23.0, 25.0]);
        assert_eq!(quads[0].colour, [0.0, 0.0, 0.0, 0.333]);
        assert_eq!(quads[4].pos, [20.0, 22.0]);
        assert_eq!(quads[5].pos, [20.0, 42.0]);
        assert_eq!(quads[6].pos, [110.0, 42.0]);
        assert_eq!(quads[4].colour, [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(quads[6].colour, [1.0, 1.0, 1.0, 1.0]);
        // "rs" (2 glyphs, no shadow) + "12" (2 glyphs + 2 shadow glyphs).
        assert_eq!(text.len(), 6 * 4);
    }
}
