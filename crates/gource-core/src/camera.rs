// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! `ZoomCamera` (`zoomcamera.cpp`) without the GL calls.
//!
//! The camera looks down +z at the z = 0 plane from `pos`, with up
//! `(0, -1, 0)` and a 90 degree vertical field of view. `adjust` sets the
//! destination from a bounding box and `logic` eases `pos` towards it.
//! The display aspect ratio, a global in the C++, is an argument here.
//! `tan` is evaluated in double precision through `libm`, as in the C++
//! (`fov * 0.5f * DEGREES_TO_RADIANS` is a double expression).

use glam::Vec3;

use crate::bounds::Bounds2D;
use crate::frustum::DEGREES_TO_RADIANS;
use crate::settings::Settings;
use crate::vectors::cpp_max;
use crate::view::CameraView;

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct ZoomCamera {
    pub(crate) pos: Vec3,
    pub(crate) dest: Vec3,
    pub(crate) target: Vec3,
    pub(crate) up: Vec3,
    initial_pos: Vec3,
    initial_target: Vec3,

    pub(crate) lockon: bool,
    pub(crate) speed: f32,
    pub(crate) lockon_time: f32,

    pub(crate) padding: f32,

    pub(crate) min_distance: f32,
    pub(crate) max_distance: f32,

    pub(crate) fov: f32,
    pub(crate) znear: f32,
    pub(crate) zfar: f32,
}

impl ZoomCamera {
    /// `ZoomCamera(pos, target, min_distance, max_distance)`.
    pub(crate) fn new(pos: Vec3, target: Vec3, min_distance: f32, max_distance: f32) -> Self {
        let mut cam = ZoomCamera {
            pos,
            dest: pos,
            target,
            up: Vec3::new(0.0, -1.0, 0.0),
            initial_pos: pos,
            initial_target: target,
            lockon: false,
            speed: 1.0,
            lockon_time: 0.0,
            padding: 1.0,
            min_distance: 0.0,
            max_distance: 0.0,
            fov: 90.0,
            znear: 0.1,
            zfar: 0.0,
        };
        cam.set_min_distance(min_distance);
        cam.set_max_distance(max_distance);
        cam.reset();
        cam
    }

    /// `reset()`: back to the initial position and target (the destination
    /// is kept, as in the C++).
    pub(crate) fn reset(&mut self) {
        self.pos = self.initial_pos;
        self.target = self.initial_target;
    }

    pub(crate) fn set_padding(&mut self, padding: f32) {
        self.padding = padding;
    }

    pub(crate) fn set_max_distance(&mut self, max: f32) {
        self.max_distance = max;
        self.zfar = (max as f64 + 1.0) as f32;
    }

    pub(crate) fn set_min_distance(&mut self, min: f32) {
        self.min_distance = min;
    }

    /// `lockOn(lockon)`.
    pub(crate) fn lock_on(&mut self, lockon: bool) {
        if lockon {
            self.lockon_time = 1.0;
        }
        self.lockon = lockon;
    }

    /// `stop()`.
    pub(crate) fn stop(&mut self) {
        self.dest = self.pos;
    }

    /// `adjust(bounds, adjust_distance)` for a display of aspect ratio
    /// `aspect_ratio` (`display.width / (float) display.height`).
    pub(crate) fn adjust(
        &mut self,
        bounds: &Bounds2D,
        adjust_distance: bool,
        aspect_ratio: f32,
        settings: &Settings,
    ) {
        // center camera on bounds
        let centre = bounds.centre();

        self.dest.x = centre.x;
        self.dest.y = centre.y;

        if !adjust_distance {
            return;
        }

        // scale by 10% so we dont have stuff right on the edge of the screen
        let mut width = bounds.width() * self.padding;
        let mut height = bounds.height() * self.padding;

        if (aspect_ratio as f64) < 1.0 {
            height /= aspect_ratio;
        } else {
            width /= aspect_ratio;
        }

        // calc visible width of the opposite wall at a distance of 1 this fov
        let toa = (libm::tan((self.fov * 0.5) as f64 * DEGREES_TO_RADIANS) * 2.0) as f32;

        // use the larger side of the box; cropping: vertical, horizontal or none
        let mut distance = if settings.crop_vertical {
            width / toa
        } else if settings.crop_horizontal {
            height / toa
        } else if width >= height {
            width / toa
        } else {
            height / toa
        };

        // check bounds are valid
        if distance < self.min_distance {
            distance = self.min_distance;
        }
        if distance > self.max_distance {
            distance = self.max_distance;
        }

        self.dest.z = -distance;
    }

    /// `setDistance(distance)`.
    pub(crate) fn set_distance(&mut self, distance: f32) {
        self.dest.z = -distance;
    }

    /// `setPos(pos, keep_angle)`.
    pub(crate) fn set_pos(&mut self, pos: Vec3, keep_angle: bool) {
        if keep_angle {
            let dir = self.target - self.pos;
            self.pos = pos;
            self.target = pos + dir;
        } else {
            self.pos = pos;
        }
    }

    /// `logic(dt)`: ease towards `dest` (faster while locking on), never
    /// overshooting, and look straight down.
    pub(crate) fn logic(&mut self, dt: f32) {
        let dp = self.dest - self.pos;

        let mut dpt = dp * dt * self.speed;

        if self.lockon {
            dpt = dpt * self.lockon_time + dp * (1.0 - self.lockon_time);

            if self.lockon_time > 0.0 {
                self.lockon_time = cpp_max(0.0, self.lockon_time - dt * 0.5);
            }
        }

        if dpt.dot(dpt) > dp.dot(dp) {
            dpt = dp;
        }

        self.pos += dpt;

        self.target = Vec3::new(self.pos.x, self.pos.y, 0.0);
    }

    /// The camera as the renderer needs it (`focus()`).
    pub(crate) fn view(&self) -> CameraView {
        CameraView {
            pos: self.pos,
            target: self.target,
            up: self.up,
            fov_y_degrees: self.fov,
            z_near: self.znear,
            z_far: self.zfar,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    fn cam() -> ZoomCamera {
        let mut c = ZoomCamera::new(Vec3::new(0.0, 0.0, -300.0), Vec3::ZERO, 100.0, 10000.0);
        c.set_padding(1.1);
        c
    }

    #[test]
    fn constructor_defaults() {
        let c = cam();
        assert_eq!(c.zfar, 10001.0);
        assert_eq!(c.znear, 0.1);
        assert_eq!(c.fov, 90.0);
        assert_eq!(c.dest, Vec3::new(0.0, 0.0, -300.0));
        let v = c.view();
        assert_eq!(v, CameraView::default());
    }

    #[test]
    fn adjust_fits_the_wider_side_and_clamps() {
        let s = Settings::default();
        let mut c = cam();
        let b = Bounds2D::new(Vec2::new(-1000.0, -100.0), Vec2::new(1000.0, 100.0));
        c.adjust(&b, true, 16.0 / 9.0, &s);
        let toa = (libm::tan(45.0f64 * DEGREES_TO_RADIANS) * 2.0) as f32;
        let width = 2000.0f32 * 1.1 / (16.0f32 / 9.0);
        assert_eq!(c.dest.z, -(width / toa));
        assert_eq!((c.dest.x, c.dest.y), (0.0, 0.0));

        // tiny bounds clamp to the minimum distance (camera_zoom_default)
        let small = Bounds2D::new(Vec2::new(10.0, 10.0), Vec2::new(12.0, 12.0));
        c.adjust(&small, true, 1.0, &s);
        assert_eq!(c.dest, Vec3::new(11.0, 11.0, -100.0));

        // adjust_distance = false only recentres
        c.set_distance(500.0);
        c.adjust(&b, false, 1.0, &s);
        assert_eq!(c.dest, Vec3::new(0.0, 0.0, -500.0));
    }

    #[test]
    fn logic_eases_without_overshoot_and_lockon_decays() {
        let mut c = cam();
        c.dest = Vec3::new(100.0, 0.0, -300.0);
        c.logic(0.5);
        assert_eq!(c.pos, Vec3::new(50.0, 0.0, -300.0));
        assert_eq!(c.target, Vec3::new(50.0, 0.0, 0.0));
        c.logic(5.0);
        assert_eq!(c.pos, Vec3::new(100.0, 0.0, -300.0));

        c.lock_on(true);
        assert_eq!(c.lockon_time, 1.0);
        c.dest = Vec3::new(200.0, 0.0, -300.0);
        c.logic(0.1);
        // dpt = dp * dt * 1 * 1 + dp * 0
        assert!((c.pos.x - 110.0).abs() < 1e-4);
        assert!((c.lockon_time - 0.95).abs() < 1e-6);
    }
}
