// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Core's `Frustum` and `Plane` (`core/frustum.cpp`, `core/plane.cpp`), the
//! Lighthouse 3D view-frustum construction Gource uses for
//! `RDirNode::checkFrustum`.
//!
//! The C++ reads the aspect ratio from the global `display`; here it is the
//! `view_ratio` argument (`display.width / display.height` as floats).
//! `tan` is evaluated in double precision through `libm`, as the C++ does
//! (`fov * 0.5 * DEGREES_TO_RADIANS` is a double expression).

use glam::Vec3;

use crate::bounds::Bounds2D;
use crate::vectors::{glm_normalize3, normalise3};

/// `DEGREES_TO_RADIANS` from Core's `pi.h` (a double literal).
pub(crate) const DEGREES_TO_RADIANS: f64 = 0.017453292;

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) struct Plane {
    pub(crate) normal: Vec3,
    pub(crate) point: Vec3,
    pub(crate) d: f32,
}

impl Plane {
    /// `Plane(v1, v2, v3)`.
    pub(crate) fn new(v1: Vec3, v2: Vec3, v3: Vec3) -> Self {
        let edge1 = v1 - v2;
        let edge2 = v3 - v2;
        let normal = glm_normalize3(edge2.cross(edge1));
        let point = v2;
        Plane {
            normal,
            point,
            d: -normal.dot(point),
        }
    }

    pub(crate) fn distance(&self, p: Vec3) -> f32 {
        self.d + self.normal.dot(p)
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) struct Frustum {
    near_distance: f32,
    far_distance: f32,
    view_ratio: f32,
    near_half_width: f32,
    near_half_height: f32,
    far_half_width: f32,
    far_half_height: f32,
    planes: [Plane; 6],
}

impl Frustum {
    /// `Frustum(source, target, up, fov, near, far)` for a display of aspect
    /// ratio `view_ratio`.
    pub(crate) fn new(
        source: Vec3,
        target: Vec3,
        up: Vec3,
        fov: f32,
        near_distance: f32,
        far_distance: f32,
        view_ratio: f32,
    ) -> Self {
        let mut f = Frustum::default();
        f.update(
            source,
            target,
            up,
            fov,
            near_distance,
            far_distance,
            view_ratio,
        );
        f
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(
        &mut self,
        source: Vec3,
        target: Vec3,
        up: Vec3,
        fov: f32,
        near_distance: f32,
        far_distance: f32,
        view_ratio: f32,
    ) {
        self.update_perspective(fov, near_distance, far_distance, view_ratio);
        self.update_view(source, target, up);
    }

    pub(crate) fn update_perspective(
        &mut self,
        fov: f32,
        near_distance: f32,
        far_distance: f32,
        view_ratio: f32,
    ) {
        self.near_distance = near_distance;
        self.far_distance = far_distance;
        self.view_ratio = view_ratio;

        let toa = libm::tan(fov as f64 * 0.5 * DEGREES_TO_RADIANS) as f32;

        self.near_half_height = near_distance * toa;
        self.near_half_width = self.near_half_height * view_ratio;
        self.far_half_height = far_distance * toa;
        self.far_half_width = self.far_half_height * view_ratio;
    }

    pub(crate) fn update_view(&mut self, source: Vec3, target: Vec3, up: Vec3) {
        let view_ray = normalise3(target - source);
        let horiz_normal = normalise3(view_ray.cross(up));
        let vert_normal = horiz_normal.cross(view_ray);

        let near_centre = source + view_ray * self.near_distance;
        let far_centre = source + view_ray * self.far_distance;

        let near_horiz_offset = horiz_normal * self.near_half_width;
        let near_vert_offset = vert_normal * self.near_half_height;

        let near_top_left = near_centre + near_vert_offset - near_horiz_offset;
        let near_top_right = near_centre + near_vert_offset + near_horiz_offset;
        let near_bottom_left = near_centre - near_vert_offset - near_horiz_offset;
        let near_bottom_right = near_centre - near_vert_offset + near_horiz_offset;

        let far_horiz_offset = horiz_normal * self.far_half_width;
        let far_vert_offset = vert_normal * self.far_half_height;

        let far_top_left = far_centre + far_vert_offset - far_horiz_offset;
        let far_top_right = far_centre + far_vert_offset + far_horiz_offset;
        let far_bottom_left = far_centre - far_vert_offset - far_horiz_offset;
        let far_bottom_right = far_centre - far_vert_offset + far_horiz_offset;

        self.planes = [
            // top
            Plane::new(near_top_right, near_top_left, far_top_left),
            // bottom
            Plane::new(near_bottom_left, near_bottom_right, far_bottom_right),
            // left
            Plane::new(near_top_left, near_bottom_left, far_bottom_left),
            // right
            Plane::new(near_bottom_right, near_top_right, far_bottom_right),
            // near
            Plane::new(near_top_left, near_top_right, near_bottom_right),
            // far
            Plane::new(far_top_right, far_top_left, far_bottom_left),
        ];
    }

    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn contains(&self, p: Vec3) -> bool {
        for plane in &self.planes {
            if plane.distance(p) < 0.0 {
                return false;
            }
        }
        true
    }

    /// `intersects(const Bounds2D&, float z = 0.0)`: the box's most positive
    /// corner along each plane normal must lie on the inner side.
    pub(crate) fn intersects(&self, bounds: &Bounds2D, z: f32) -> bool {
        for plane in &self.planes {
            let corner = Vec3::new(
                if plane.normal.x > 0.0 {
                    bounds.max.x
                } else {
                    bounds.min.x
                },
                if plane.normal.y > 0.0 {
                    bounds.max.y
                } else {
                    bounds.min.y
                },
                z,
            );
            if plane.distance(corner) < 0.0 {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    fn gource_default_frustum(cam: Vec3) -> Frustum {
        // ZoomCamera defaults: fov 90, znear 0.1, zfar = 10000 + 1, up (0,-1,0),
        // looking straight down -z at its own x/y.
        Frustum::new(
            cam,
            Vec3::new(cam.x, cam.y, 0.0),
            Vec3::new(0.0, -1.0, 0.0),
            90.0,
            0.1,
            10001.0,
            16.0 / 9.0,
        )
    }

    #[test]
    fn camera_above_origin_sees_nearby_bounds() {
        let f = gource_default_frustum(Vec3::new(0.0, 0.0, -300.0));
        let near = Bounds2D::new(Vec2::splat(-10.0), Vec2::splat(10.0));
        assert!(f.intersects(&near, 0.0));
        // Half height at distance 300 with fov 90 is ~300, half width ~533.
        let right_edge = Bounds2D::new(Vec2::new(520.0, -5.0), Vec2::new(540.0, 5.0));
        assert!(f.intersects(&right_edge, 0.0));
        let outside_x = Bounds2D::new(Vec2::new(560.0, -5.0), Vec2::new(580.0, 5.0));
        assert!(!f.intersects(&outside_x, 0.0));
        let outside_y = Bounds2D::new(Vec2::new(-5.0, 320.0), Vec2::new(5.0, 340.0));
        assert!(!f.intersects(&outside_y, 0.0));
        assert!(f.contains(Vec3::ZERO));
        assert!(!f.contains(Vec3::new(0.0, 0.0, -400.0)));
    }

    const GOLDEN: &str = include_str!("../tests/golden/frustum_golden.tsv");

    /// Planes and intersection results against Core's frustum.cpp, built by
    /// tests/golden/dirtree/build.sh on a 1280x720 display.
    #[test]
    fn matches_cpp_golden() {
        let cams = [
            (Vec3::new(0.0, 0.0, -300.0), Vec3::new(0.0, 0.0, 0.0)),
            (
                Vec3::new(123.5, -77.25, -850.0),
                Vec3::new(123.5, -77.25, 0.0),
            ),
            (Vec3::new(5.0, 5.0, -50.0), Vec3::new(5.0, 5.0, 0.0)),
            (Vec3::new(10.0, -20.0, -400.0), Vec3::new(0.0, 0.0, 0.0)),
        ];
        let frustums: Vec<Frustum> = cams
            .iter()
            .map(|&(pos, target)| {
                Frustum::new(
                    pos,
                    target,
                    Vec3::new(0.0, -1.0, 0.0),
                    90.0,
                    0.1,
                    10001.0,
                    1280.0 / 720.0,
                )
            })
            .collect();

        let mut ours = Vec::new();
        for (c, f) in frustums.iter().enumerate() {
            for (i, p) in f.planes.iter().enumerate() {
                ours.push(format!(
                    "{c}\tplane\t{i}\t{:08x}\t{:08x}\t{:08x}\t{:08x}",
                    p.normal.x.to_bits(),
                    p.normal.y.to_bits(),
                    p.normal.z.to_bits(),
                    p.d.to_bits()
                ));
            }
            let mut k = 0;
            for gx in -12i32..=12 {
                for gy in -8i32..=8 {
                    let centre = Vec2::new(cams[c].0.x, cams[c].0.y)
                        + Vec2::new(gx as f32 * 50.0, gy as f32 * 41.0);
                    let r = 3.0 + ((gx * 7 + gy * 13) & 15) as f32;
                    let b = Bounds2D::new(centre - Vec2::splat(r), centre + Vec2::splat(r));
                    ours.push(format!(
                        "{c}\tbox\t{k}\t{}\t{}",
                        f.intersects(&b, 0.0) as i32,
                        f.contains(centre.extend(0.0)) as i32
                    ));
                    k += 1;
                }
            }
        }

        let theirs: Vec<&str> = GOLDEN
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .collect();
        for (i, (a, b)) in ours.iter().zip(theirs.iter()).enumerate() {
            assert_eq!(a, b, "first difference at golden row {i}");
        }
        assert_eq!(ours.len(), theirs.len());
    }

    #[test]
    fn frustum_follows_camera() {
        let f = gource_default_frustum(Vec3::new(1000.0, 0.0, -300.0));
        let origin = Bounds2D::new(Vec2::splat(-10.0), Vec2::splat(10.0));
        assert!(!f.intersects(&origin, 0.0));
        let there = Bounds2D::new(Vec2::new(990.0, -10.0), Vec2::new(1010.0, 10.0));
        assert!(f.intersects(&there, 0.0));
    }
}
