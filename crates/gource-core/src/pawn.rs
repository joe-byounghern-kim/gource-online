// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! `Pawn` (`pawn.cpp`): the state RFile and RUser share — position, fade
//! timer, name timers, size and graphic ratio — minus the GL drawing.
//!
//! `RFile` and (in part 2) `RUser` embed a [`Pawn`] and override the C++
//! virtuals (`getAlpha`, `getColour`, `drawNameText`) in their own modules.
//! Label alpha follows `Pawn::drawName`: [`Pawn::name_alpha`] returns the
//! value `drawName` would hand to `drawNameText`, or `None` when
//! `nameVisible()` is false.

use glam::{Vec2, Vec3};

use crate::bounds::Bounds2D;
use crate::vectors::cpp_min;

/// `gGourceShadowStrength`.
#[allow(
    dead_code,
    reason = "kept for parity with the C++ API; the simulation does not call it"
)]
pub(crate) const SHADOW_STRENGTH: f32 = 0.5;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pawn {
    pub(crate) pos: Vec2,
    pub(crate) shadow_offset: Vec2,
    pub(crate) name: String,
    pub(crate) namewidth: f32,
    pub(crate) accel: Vec2,
    pub(crate) speed: f32,

    pub(crate) elapsed: f32,
    pub(crate) fadetime: f32,

    pub(crate) nametime: f32,
    pub(crate) name_interval: f32,
    pub(crate) namecol: Vec3,

    pub(crate) shadow: bool,
    pub(crate) hidden: bool,
    pub(crate) tagid: i32,
    pub(crate) mouseover: bool,
    pub(crate) selected: bool,

    pub(crate) size: f32,
    pub(crate) graphic_ratio: f32,
    pub(crate) dims: Vec2,

    /// `QuadItem::quadItemBounds`.
    pub(crate) quad_item_bounds: Bounds2D,
}

impl Pawn {
    /// `Pawn(name, pos, tagid)`.
    pub(crate) fn new(name: &str, pos: Vec2, tagid: i32) -> Self {
        Pawn {
            pos,
            shadow_offset: Vec2::new(2.0, 2.0),
            name: name.to_string(),
            namewidth: 0.0,
            accel: Vec2::ZERO,
            speed: 1.0,
            elapsed: 0.0,
            fadetime: 1.0,
            nametime: 5.0,
            name_interval: 0.0,
            namecol: Vec3::ONE,
            shadow: false,
            hidden: false,
            tagid,
            mouseover: false,
            selected: false,
            size: 0.0,
            graphic_ratio: 1.0,
            dims: Vec2::ZERO,
            quad_item_bounds: Bounds2D::default(),
        }
    }

    /// `setGraphic(graphic)` for a texture of `width` x `height` pixels
    /// (`None` = no graphic): `graphic_ratio = h / (float) w`,
    /// `dims = (size, size * graphic_ratio)`.
    pub(crate) fn set_graphic(&mut self, texture_size: Option<(u32, u32)>) {
        self.graphic_ratio = match texture_size {
            Some((w, h)) => h as f32 / w as f32,
            None => 1.0,
        };
        self.dims = Vec2::new(self.size, self.size * self.graphic_ratio);
    }

    /// `showName()`.
    pub(crate) fn show_name(&mut self) {
        if self.name_interval <= 0.0 {
            self.name_interval = self.nametime;
        }
    }

    /// `updateQuadItemBounds()`.
    pub(crate) fn update_quad_item_bounds(&mut self) {
        let halfsize_x = self.size * 0.5;
        let halfsize = Vec2::new(halfsize_x, halfsize_x * self.graphic_ratio);
        self.quad_item_bounds
            .set(self.pos - halfsize, self.pos + halfsize);
    }

    /// `Pawn::logic(dt)`.
    pub(crate) fn logic(&mut self, dt: f32) {
        self.elapsed += dt;
        if !self.hidden && self.name_interval > 0.0 {
            self.name_interval -= dt;
        }
    }

    /// `Pawn::getAlpha()`.
    pub(crate) fn base_alpha(&self) -> f32 {
        cpp_min(self.elapsed / self.fadetime, 1.0)
    }

    /// `nameVisible()`.
    pub(crate) fn name_visible(&self) -> bool {
        !((!self.selected && self.name_interval < 0.0) || self.hidden)
    }

    /// The alpha `drawName()` passes to `drawNameText`, or `None` when the
    /// name is not visible. The comparisons against `1.0` and
    /// `nametime - 1.0` are double-precision in the C++.
    pub(crate) fn name_alpha(&self) -> Option<f32> {
        if !self.name_visible() {
            return None;
        }
        Some(self.draw_name_alpha())
    }

    /// The alpha `drawName()` computes once `nameVisible()` passed. `RUser`
    /// overrides `nameVisible` and checks its own rule before calling this.
    pub(crate) fn draw_name_alpha(&self) -> f32 {
        let done = self.nametime - self.name_interval;
        let d = done as f64;
        if d < 1.0 {
            done
        } else if d > 1.0 && d < self.nametime as f64 - 1.0 {
            1.0
        } else {
            self.nametime - done
        }
    }

    /// Top-left corner of the shadow quad (`drawShadow`).
    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn shadow_origin(&self) -> Vec2 {
        let halfsize = self.size * 0.5;
        self.pos - Vec2::new(halfsize, halfsize * self.graphic_ratio) + self.shadow_offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_alpha_ramps_up_holds_and_fades() {
        let mut p = Pawn::new("x", Vec2::ZERO, 1);
        p.nametime = 4.0;
        p.name_interval = 4.0;
        let mut seen = Vec::new();
        for _ in 0..10 {
            seen.push(p.name_alpha().unwrap());
            p.logic(0.5);
        }
        // done == 1.0 exactly is neither `< 1` nor `> 1`, so drawName takes
        // the fade-out branch and passes nametime - 1 = 3 (a C++ quirk). Once
        // name_interval reaches 0 it stops decreasing (`> 0.0` guard), so the
        // name stays "visible" at alpha 0.
        assert_eq!(seen, vec![0.0, 0.5, 3.0, 1.0, 1.0, 1.0, 1.0, 0.5, 0.0, 0.0]);

        p.name_interval = -0.1;
        assert_eq!(p.name_alpha(), None);
        p.selected = true;
        assert!(p.name_alpha().is_some());
        p.hidden = true;
        assert_eq!(p.name_alpha(), None);
    }
}
