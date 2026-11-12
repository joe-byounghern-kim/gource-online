// SPDX-License-Identifier: GPL-3.0-or-later

//! Cell allocation for the glyph atlas: horizontal shelves with
//! least-recently-used eviction between frames.
//!
//! Gource's `FXGlyphSet` adds texture pages without limit; a WebGL texture
//! has a size limit, so the atlas grows to a maximum and then reuses space.
//! A shelf is a strip of rows holding cells of similar height side by side.
//! Every glyph lookup marks its shelf used in the current frame
//! ([`ShelfPacker::touch`]); when nothing fits, the shelf unused for the
//! longest time is cleared and refilled. A shelf used in the current or the
//! previous frame is never evicted, so the texture coordinates handed out
//! while a frame is built stay valid until it is drawn (several frames can
//! be built before one is drawn). When everything is in use the allocation
//! fails and the caller draws without the glyph until space frees up.
//!
//! Pure bookkeeping (no canvas), so it is tested natively.

/// Gap between cells and between shelves (`FXGlyphPage::addGlyph`).
pub const PADDING: u32 = 3;
/// Distance of the first cell from the atlas edge.
pub const MARGIN: u32 = 2;
/// Shelf heights are multiples of this, so glyphs of one font size share
/// shelves although their ink heights differ.
const HEIGHT_STEP: u32 = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct Shelf {
    pub y: u32,
    pub height: u32,
    /// Next free x.
    x: u32,
    last_used: u32,
}

/// A reserved cell.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub x: u32,
    pub y: u32,
    pub shelf: u32,
}

/// What the caller has to do to the canvas before drawing into a cell.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Alloc {
    /// Draw into `cell`.
    Placed(Cell),
    /// Grow the canvas to `new_size` (keeping its contents), then call
    /// [`ShelfPacker::alloc`] again.
    Grow { new_size: u32 },
    /// Every glyph of shelf `shelf` was dropped: forget them and clear the
    /// shelf's rows (`Shelf::y - 1 .. y + height + 1`), then draw into
    /// `cell`.
    Evicted { shelf: u32, cell: Cell },
    /// No space this frame.
    Full,
    /// Larger than the largest atlas.
    Oversized,
}

#[derive(Clone, Debug)]
pub struct ShelfPacker {
    size: u32,
    max_size: u32,
    shelves: Vec<Shelf>,
    /// First row below the last shelf.
    next_y: u32,
    /// Current frame number (starts at 2 so "previous frame" never
    /// underflows).
    frame: u32,
}

impl ShelfPacker {
    pub fn new(size: u32, max_size: u32) -> Self {
        ShelfPacker {
            size,
            max_size: max_size.max(size),
            shelves: Vec::new(),
            next_y: MARGIN,
            frame: 2,
        }
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn shelf(&self, i: u32) -> &Shelf {
        &self.shelves[i as usize]
    }

    /// Drop every shelf (the size is kept).
    pub fn clear(&mut self) {
        self.shelves.clear();
        self.next_y = MARGIN;
    }

    /// A new frame starts being built.
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        if self.frame < 2 {
            // wrapped: restart the clock, keeping the order of use
            for s in &mut self.shelves {
                s.last_used = 0;
            }
            self.frame = 2;
        }
    }

    /// A glyph on `shelf` is used in this frame.
    #[inline]
    pub fn touch(&mut self, shelf: u32) {
        if let Some(s) = self.shelves.get_mut(shelf as usize) {
            s.last_used = self.frame;
        }
    }

    /// Reserve a `w × h` cell (a zero-sized one still takes `PADDING`
    /// columns, so its texture coordinates sample empty texels).
    pub fn alloc(&mut self, w: u32, h: u32) -> Alloc {
        if w + 2 * MARGIN + PADDING > self.max_size || h + 2 * MARGIN + PADDING > self.max_size {
            return Alloc::Oversized;
        }
        let bucket = h.div_ceil(HEIGHT_STEP).max(1) * HEIGHT_STEP;
        let frame = self.frame;

        // an open shelf of about this height
        for (i, s) in self.shelves.iter_mut().enumerate() {
            if s.height >= h && s.height <= bucket + HEIGHT_STEP && s.x + w + PADDING <= self.size {
                let cell = Cell {
                    x: s.x,
                    y: s.y,
                    shelf: i as u32,
                };
                s.x += w + PADDING;
                s.last_used = frame;
                return Alloc::Placed(cell);
            }
        }

        // a new shelf below the others
        if self.next_y + bucket + PADDING <= self.size {
            let i = self.shelves.len() as u32;
            self.shelves.push(Shelf {
                y: self.next_y,
                height: bucket,
                x: MARGIN + w + PADDING,
                last_used: frame,
            });
            self.next_y += bucket + PADDING;
            return Alloc::Placed(Cell {
                x: MARGIN,
                y: self.shelves[i as usize].y,
                shelf: i,
            });
        }

        if self.size < self.max_size {
            self.size = (self.size * 2).min(self.max_size);
            return Alloc::Grow {
                new_size: self.size,
            };
        }

        // the least recently used shelf that is tall enough and was not used
        // in this frame or the previous one (lowest such shelf first)
        let victim = self
            .shelves
            .iter()
            .enumerate()
            .filter(|(_, s)| s.height >= h && s.last_used + 1 < frame)
            .min_by_key(|(_, s)| (s.last_used, s.height))
            .map(|(i, _)| i);
        let Some(i) = victim else {
            return Alloc::Full;
        };
        let s = &mut self.shelves[i];
        s.x = MARGIN + w + PADDING;
        s.last_used = frame;
        Alloc::Evicted {
            shelf: i as u32,
            cell: Cell {
                x: MARGIN,
                y: s.y,
                shelf: i as u32,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placed(a: Alloc) -> Cell {
        match a {
            Alloc::Placed(c) => c,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn cells_flow_along_shelves_of_similar_height() {
        let mut p = ShelfPacker::new(64, 64);
        let a = placed(p.alloc(10, 9));
        let b = placed(p.alloc(10, 12));
        assert_eq!((a.x, a.y, a.shelf), (MARGIN, MARGIN, 0));
        assert_eq!((b.x, b.y, b.shelf), (MARGIN + 13, MARGIN, 0));
        // much taller: its own shelf below
        let c = placed(p.alloc(10, 30));
        assert_eq!((c.x, c.y, c.shelf), (MARGIN, MARGIN + 12 + PADDING, 1));
        // a zero-sized cell still advances
        let d = placed(p.alloc(0, 0));
        let e = placed(p.alloc(0, 0));
        assert_eq!(e.x, d.x + PADDING);
    }

    #[test]
    fn grows_to_the_maximum_then_evicts_only_shelves_unused_for_a_frame() {
        let mut p = ShelfPacker::new(32, 64);
        let mut cells = Vec::new();
        loop {
            match p.alloc(20, 8) {
                Alloc::Placed(c) => cells.push(c),
                Alloc::Grow { new_size } => assert_eq!(new_size, 64),
                Alloc::Full => break,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(p.size(), 64);
        let shelves = cells.iter().map(|c| c.shelf).max().unwrap() + 1;
        assert!(shelves >= 4, "{shelves}");

        // next frame: everything was used in the previous frame
        p.begin_frame();
        assert_eq!(p.alloc(20, 8), Alloc::Full);

        // a frame later: shelf 1 is still in use, the others are free
        p.begin_frame();
        p.touch(1);
        p.touch(0);
        match p.alloc(20, 8) {
            Alloc::Evicted { shelf, cell } => {
                assert!(shelf > 1);
                assert_eq!(cell.shelf, shelf);
                assert_eq!(cell.x, MARGIN);
            }
            other => panic!("{other:?}"),
        }
        // the evicted shelf now belongs to this frame: the next one is
        // another shelf, never 0 or 1
        let mut evicted = 1;
        loop {
            match p.alloc(20, 8) {
                Alloc::Placed(c) => assert!(c.shelf > 1),
                Alloc::Evicted { shelf, .. } => {
                    assert!(shelf > 1);
                    evicted += 1;
                }
                Alloc::Full => break,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(evicted, shelves - 2);
    }

    #[test]
    fn oldest_shelf_goes_first_and_must_be_tall_enough() {
        let mut p = ShelfPacker::new(64, 64);
        // three shelves of height 8 and one of 16
        let s: Vec<u32> = [8, 8, 8, 16]
            .iter()
            .map(|&h| {
                let c = placed(p.alloc(30, h));
                c.shelf
            })
            .collect();
        assert_eq!(s, [0, 1, 2, 3]);
        assert_eq!(
            (p.shelf(3).y, p.shelf(3).height),
            (MARGIN + 3 * (8 + PADDING), 16)
        );
        for f in 0..4 {
            p.begin_frame();
            // shelf f is touched last in frame f
            p.touch(f);
        }
        p.begin_frame();
        p.begin_frame();
        // height 8: the least recently used is shelf 0
        assert!(matches!(p.alloc(30, 8), Alloc::Evicted { shelf: 0, .. }));
        // height 16: only shelf 3 is tall enough
        assert!(matches!(p.alloc(30, 16), Alloc::Evicted { shelf: 3, .. }));
        // taller than every shelf, and taller than the atlas
        assert_eq!(p.alloc(30, 40), Alloc::Full);
        assert_eq!(p.alloc(30, 100), Alloc::Oversized);
        p.clear();
        assert_eq!(placed(p.alloc(30, 8)).y, MARGIN);
    }
}
