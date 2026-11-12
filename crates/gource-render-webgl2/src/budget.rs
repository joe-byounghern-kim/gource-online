// SPDX-License-Identifier: GPL-3.0-or-later

//! The glyph atlas' per-frame rasterisation budget and its eviction count,
//! kept apart from the canvas code so the policy is tested natively.
//!
//! Live, a frame rasterises at most [`MAX_NEW_PER_FRAME`] new glyphs and
//! the rest are laid out with their advance and drawn from a later frame, so
//! a commit of thousands of new names does not stall one frame. A video
//! recorder cannot accept names that appear a frame late, so it lifts the
//! limit ([`GlyphBudget::set_limit`] with `u32::MAX`) and reports how many
//! glyphs the full atlas had to drop (`recording.md` REC-42).

/// Glyph measurements plus rasterisations per frame, beyond each font
/// size's precache list (a new glyph is one of each). A commit of thousands
/// of CJK names fills in over a few frames instead of stalling one (a new
/// glyph costs two `measureText` calls, a `fillText` and a composite).
pub const MAX_NEW_PER_FRAME: u32 = 512;

/// Per-frame budget and eviction counter.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct GlyphBudget {
    limit: u32,
    used: u32,
    evicted: u32,
}

impl Default for GlyphBudget {
    fn default() -> Self {
        GlyphBudget::new(MAX_NEW_PER_FRAME)
    }
}

impl GlyphBudget {
    pub const fn new(limit: u32) -> Self {
        GlyphBudget {
            limit,
            used: 0,
            evicted: 0,
        }
    }

    /// New glyphs allowed per frame; `u32::MAX` never defers one.
    pub fn set_limit(&mut self, limit: u32) {
        self.limit = limit;
    }

    /// A new frame: the budget is full again.
    pub fn begin_frame(&mut self) {
        self.used = 0;
    }

    /// No new glyph may be measured or rasterised in this frame.
    pub fn exhausted(&self) -> bool {
        self.used >= self.limit
    }

    /// One measurement or rasterisation was made.
    pub fn spend(&mut self) {
        self.used = self.used.saturating_add(1);
    }

    /// `glyphs` were dropped with an evicted shelf.
    pub fn note_evicted(&mut self, glyphs: usize) {
        let n = u32::try_from(glyphs).unwrap_or(u32::MAX);
        self.evicted = self.evicted.saturating_add(n);
    }

    /// Glyphs dropped since creation or [`GlyphBudget::reset_evictions`].
    pub fn evicted(&self) -> u32 {
        self.evicted
    }

    pub fn reset_evictions(&mut self) {
        self.evicted = 0;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// Runs one frame that asks for every glyph of `names` the way the atlas
    /// does (a glyph costs one measurement and one rasterisation); returns
    /// how many distinct glyphs had to wait for a later frame.
    fn frame(budget: &mut GlyphBudget, names: &[String], seen: &mut HashSet<char>) -> usize {
        budget.begin_frame();
        let mut deferred = HashSet::new();
        for ch in names.iter().flat_map(|n| n.chars()) {
            if seen.contains(&ch) || deferred.contains(&ch) {
                continue;
            }
            if budget.exhausted() {
                deferred.insert(ch);
                continue;
            }
            budget.spend(); // measure
            if budget.exhausted() {
                deferred.insert(ch);
                continue;
            }
            budget.spend(); // rasterise
            seen.insert(ch);
        }
        deferred.len()
    }

    /// 3,000 names, each with a character of its own (CJK).
    fn names() -> Vec<String> {
        (0..3000u32)
            .map(|i| {
                let ch = char::from_u32(0x4e00 + i).unwrap();
                format!("user{i}{ch}")
            })
            .collect()
    }

    #[test]
    fn the_live_budget_defers_glyphs_to_later_frames() {
        let mut budget = GlyphBudget::default();
        assert_eq!(budget, GlyphBudget::new(MAX_NEW_PER_FRAME));
        let mut seen = HashSet::new();
        let names = names();
        let first = frame(&mut budget, &names, &mut seen);
        assert!(first > 2000, "{first}");
        // every frame makes progress until nothing waits
        let mut frames = 1;
        let mut waiting = first;
        while waiting > 0 {
            let next = frame(&mut budget, &names, &mut seen);
            assert!(next < waiting);
            waiting = next;
            frames += 1;
        }
        assert!(frames > 2);
    }

    #[test]
    fn an_unlimited_budget_rasterises_every_glyph_in_its_frame() {
        let mut budget = GlyphBudget::default();
        budget.set_limit(u32::MAX);
        let mut seen = HashSet::new();
        assert_eq!(frame(&mut budget, &names(), &mut seen), 0);
        assert_eq!(seen.len(), 3000 + 14); // the CJK characters plus "user0123456789"
    }

    #[test]
    fn evictions_are_counted_until_reset() {
        let mut budget = GlyphBudget::default();
        budget.note_evicted(12);
        budget.begin_frame();
        budget.note_evicted(3);
        assert_eq!(budget.evicted(), 15);
        budget.note_evicted(usize::MAX);
        assert_eq!(budget.evicted(), u32::MAX);
        budget.reset_evictions();
        assert_eq!(budget.evicted(), 0);
    }
}
