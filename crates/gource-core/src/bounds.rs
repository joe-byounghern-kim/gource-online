// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Core's `Bounds2D` (`core/bounds.h`).
//!
//! A freshly reset box is `(0,0)..(0,0)` with `first = true`: the first
//! `update` snaps both corners to the point, `contains` is false until then,
//! but `overlaps`, `area` and `centre` ignore `first` and use the zero box,
//! exactly as in the C++.

use glam::Vec2;

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Bounds2D {
    pub(crate) min: Vec2,
    pub(crate) max: Vec2,
    pub(crate) first: bool,
}

impl Default for Bounds2D {
    fn default() -> Self {
        Bounds2D {
            min: Vec2::ZERO,
            max: Vec2::ZERO,
            first: true,
        }
    }
}

impl Bounds2D {
    /// `Bounds2D(min, max)`: reset, then update with both points.
    pub(crate) fn new(min: Vec2, max: Vec2) -> Self {
        let mut b = Bounds2D::default();
        b.update(min);
        b.update(max);
        b
    }

    pub(crate) fn centre(&self) -> Vec2 {
        self.min + (self.max - self.min) * 0.5
    }

    pub(crate) fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    pub(crate) fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    pub(crate) fn area(&self) -> f32 {
        self.width() * self.height()
    }

    pub(crate) fn reset(&mut self) {
        *self = Bounds2D::default();
    }

    /// `update(const Bounds2D&)`.
    pub(crate) fn update_bounds(&mut self, b: &Bounds2D) {
        self.update(b.min);
        self.update(b.max);
    }

    /// `set(const Bounds2D&)`.
    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn set_bounds(&mut self, b: &Bounds2D) {
        self.reset();
        self.update_bounds(b);
    }

    /// `set(vec2)`.
    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn set_point(&mut self, p: Vec2) {
        self.reset();
        self.update(p);
    }

    /// `set(a, b)`.
    pub(crate) fn set(&mut self, a: Vec2, b: Vec2) {
        self.reset();
        self.update(a);
        self.update(b);
    }

    /// `update(const vec2&)`.
    pub(crate) fn update(&mut self, p: Vec2) {
        if self.first {
            self.min = p;
            self.max = p;
            self.first = false;
            return;
        }
        if self.min.x > p.x {
            self.min.x = p.x;
        }
        if self.min.y > p.y {
            self.min.y = p.y;
        }
        if self.max.x < p.x {
            self.max.x = p.x;
        }
        if self.max.y < p.y {
            self.max.y = p.y;
        }
    }

    pub(crate) fn contains(&self, p: Vec2) -> bool {
        if self.first {
            return false;
        }
        self.min.x <= p.x && self.min.y <= p.y && self.max.x >= p.x && self.max.y >= p.y
    }

    pub(crate) fn overlaps(&self, b: &Bounds2D) -> bool {
        if self.max.y < b.min.y {
            return false;
        }
        if self.min.y > b.max.y {
            return false;
        }
        if self.max.x < b.min.x {
            return false;
        }
        if self.min.x > b.max.x {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_update_snaps_to_point() {
        let mut b = Bounds2D::default();
        assert!(b.first);
        assert!(!b.contains(Vec2::ZERO));
        // overlaps ignores `first`: the reset box is the zero box.
        assert!(b.overlaps(&Bounds2D::new(Vec2::splat(-1.0), Vec2::splat(1.0))));

        b.update(Vec2::new(5.0, 7.0));
        assert_eq!(b.min, Vec2::new(5.0, 7.0));
        assert_eq!(b.max, Vec2::new(5.0, 7.0));
        assert!(b.contains(Vec2::new(5.0, 7.0)));

        b.update(Vec2::new(-1.0, 10.0));
        assert_eq!(b.min, Vec2::new(-1.0, 7.0));
        assert_eq!(b.max, Vec2::new(5.0, 10.0));
        assert_eq!(b.area(), 18.0);
        assert_eq!(b.centre(), Vec2::new(2.0, 8.5));
    }

    #[test]
    fn constructor_orders_corners() {
        let b = Bounds2D::new(Vec2::new(3.0, -2.0), Vec2::new(-3.0, 2.0));
        assert_eq!(b.min, Vec2::new(-3.0, -2.0));
        assert_eq!(b.max, Vec2::new(3.0, 2.0));
    }

    #[test]
    fn overlap_is_inclusive() {
        let a = Bounds2D::new(Vec2::ZERO, Vec2::ONE);
        let touching = Bounds2D::new(Vec2::ONE, Vec2::splat(2.0));
        let apart = Bounds2D::new(Vec2::splat(1.5), Vec2::splat(2.0));
        assert!(a.overlaps(&touching));
        assert!(!a.overlaps(&apart));
    }
}
