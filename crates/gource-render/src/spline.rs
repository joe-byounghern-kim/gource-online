// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Directory edges (`spline.cpp`: `SplineEdge::update`, `getLabelPos`,
//! `drawToVBO`), computed in screen space from projected positions.

use glam::{Vec2, Vec4};

use crate::QuadVertex;
use crate::emit::vertex;

/// Upper bound of `edge_detail` (`std::min(10, ...)`).
pub(crate) const MAX_EDGE_DETAIL: usize = 10;

/// Half-width of an edge in pixels (`drawToVBO`'s `2.5f`).
pub(crate) const EDGE_HALF_WIDTH: f32 = 2.5;

/// Gource's `normalise`: `v / |v|`, or `v` unchanged when its length is 0.
#[inline]
pub(crate) fn normalise(v: Vec2) -> Vec2 {
    let l = v.length();
    if l > 0.0 { v / l } else { v }
}

/// `SplineEdge::update`'s point count minus one: more segments the more the
/// control point bends the edge.
pub(crate) fn edge_detail(pos1: Vec2, pos2: Vec2, spos: Vec2) -> usize {
    let mid = (pos1 - pos2) * 0.5;
    let to = pos1 - spos;
    let dp = normalise(to).dot(normalise(mid)).min(1.0);
    let ang = (libm::acos(dp as f64) / std::f64::consts::PI) as f32;
    // `(int)(ang * 100.0)`; NaN (dp < -1 from rounding) truncates to 0 here.
    let detail = ((ang as f64 * 100.0) as i32).min(MAX_EDGE_DETAIL as i32);
    detail.max(1) as usize
}

/// The sampled quadratic spline of one edge (`spline_point`,
/// `spline_colour`), running from the child (`pos2`) to the parent (`pos1`).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Spline {
    pub points: [Vec2; MAX_EDGE_DETAIL + 1],
    pub colours: [Vec4; MAX_EDGE_DETAIL + 1],
    pub len: usize,
}

impl Spline {
    /// `SplineEdge::update(pos1, col1, pos2, col2, spos)`.
    pub fn new(pos1: Vec2, col1: Vec4, pos2: Vec2, col2: Vec4, spos: Vec2) -> Spline {
        let detail = edge_detail(pos1, pos2, spos);
        let mut s = Spline {
            points: [Vec2::ZERO; MAX_EDGE_DETAIL + 1],
            colours: [Vec4::ZERO; MAX_EDGE_DETAIL + 1],
            len: detail + 1,
        };
        for i in 0..=detail {
            let t = i as f32 / detail as f32;
            let tt = 1.0 - t;
            let p0 = pos1 * t + spos * tt;
            let p1 = spos * t + pos2 * tt;
            s.points[i] = p0 * t + p1 * tt;
            s.colours[i] = col1 * t + col2 * tt;
        }
        s
    }

    /// `SplineEdge::drawToVBO`: one quad per segment, `half_width` either side.
    pub fn push_quads(&self, half_width: f32, out: &mut Vec<QuadVertex>) {
        for i in 0..self.len.saturating_sub(1) {
            let (a, b) = (self.points[i], self.points[i + 1]);
            let d = a - b;
            let perp = normalise(Vec2::new(-d.y, d.x)) * half_width;
            let (ca, cb) = (self.colours[i], self.colours[i + 1]);
            out.extend_from_slice(&[
                vertex(a + perp, ca, [1.0, 0.0]),
                vertex(a - perp, ca, [0.0, 0.0]),
                vertex(b - perp, cb, [0.0, 0.0]),
                vertex(b + perp, cb, [1.0, 0.0]),
            ]);
        }
    }
}

/// `SplineEdge::update`'s `label_pos` for `dir_name_position`.
pub(crate) fn label_pos(pos1: Vec2, pos2: Vec2, spos: Vec2, position: f32) -> Vec2 {
    let s_quota = 0.5 - (position - 0.5).abs();
    let p_quota = 1.0 - s_quota;
    pos1 * (p_quota * (1.0 - position)) + pos2 * (p_quota * position) + spos * s_quota
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn straight_edge_has_one_segment() {
        // spos on the line between the endpoints: angle 0 → detail clamps to 1.
        let p1 = Vec2::new(0.0, 0.0);
        let p2 = Vec2::new(100.0, 0.0);
        let spos = Vec2::new(50.0, 0.0);
        assert_eq!(edge_detail(p1, p2, spos), 1);
        let s = Spline::new(p1, Vec4::ONE, p2, Vec4::ZERO, spos);
        assert_eq!(s.len, 2);
        assert!(close(s.points[0], p2));
        assert!(close(s.points[1], p1));
        assert_eq!(s.colours[0], Vec4::ZERO);
        assert_eq!(s.colours[1], Vec4::ONE);
    }

    #[test]
    fn detail_follows_cpp_formula() {
        let p1 = Vec2::new(0.0, 0.0);
        let p2 = Vec2::new(-100.0, 0.0);
        // mid = (50, 0); to = p1 - spos.
        for (spos, expected) in [
            // to = (50, -5): angle atan(0.1) = 5.71° → 0.0317 × 100 = 3.
            (Vec2::new(-50.0, 5.0), 3),
            // to = (50, -50): 45° → 0.25 × 100 = 25 → clamp 10.
            (Vec2::new(-50.0, 50.0), 10),
            // to = (50, -1.5): 1.72° → 0.0095 × 100 = 0 → 1.
            (Vec2::new(-50.0, 1.5), 1),
        ] {
            let to = (p1 - spos).normalize();
            let mid = ((p1 - p2) * 0.5).normalize();
            let ang = to.dot(mid).min(1.0).acos() / std::f32::consts::PI;
            let formula = ((ang * 100.0) as i32).clamp(1, 10) as usize;
            assert_eq!(formula, expected);
            assert_eq!(edge_detail(p1, p2, spos), expected, "spos {spos:?}");
        }
    }

    #[test]
    fn points_are_the_quadratic_bezier() {
        let p1 = Vec2::new(10.0, 20.0);
        let p2 = Vec2::new(200.0, 80.0);
        let spos = Vec2::new(60.0, 150.0);
        let s = Spline::new(p1, Vec4::ONE, p2, Vec4::ONE, spos);
        assert_eq!(s.len, edge_detail(p1, p2, spos) + 1);
        let n = (s.len - 1) as f32;
        for i in 0..s.len {
            let t = i as f32 / n;
            // B(t) = t² p1 + 2 t (1 - t) spos + (1 - t)² p2
            let b = p1 * t * t + spos * 2.0 * t * (1.0 - t) + p2 * (1.0 - t) * (1.0 - t);
            assert!(close(s.points[i], b), "{i}: {:?} vs {b:?}", s.points[i]);
        }
    }

    #[test]
    fn quads_are_perpendicular_strips() {
        let s = Spline::new(
            Vec2::new(0.0, 0.0),
            Vec4::ONE,
            Vec2::new(0.0, 100.0),
            Vec4::new(1.0, 0.0, 0.0, 1.0),
            Vec2::new(0.0, 50.0),
        );
        let mut out = Vec::new();
        s.push_quads(2.5, &mut out);
        assert_eq!(out.len(), 4);
        // Segment from (0,100) to (0,0): d = (0, 100), perp = (-1, 0) × 2.5.
        assert_eq!(out[0].pos, [-2.5, 100.0]);
        assert_eq!(out[1].pos, [2.5, 100.0]);
        assert_eq!(out[2].pos, [2.5, 0.0]);
        assert_eq!(out[3].pos, [-2.5, 0.0]);
        assert_eq!(out[0].uv, [1.0, 0.0]);
        assert_eq!(out[1].uv, [0.0, 0.0]);
        assert_eq!(out[2].uv, [0.0, 0.0]);
        assert_eq!(out[3].uv, [1.0, 0.0]);
        assert_eq!(out[0].colour, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(out[3].colour, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn label_position_blends_endpoints_and_control_point() {
        let p1 = Vec2::new(0.0, 0.0);
        let p2 = Vec2::new(100.0, 0.0);
        let spos = Vec2::new(50.0, 40.0);
        // 0.5: s_quota 0.5, p_quota 0.5 → 0.25 p1 + 0.25 p2 + 0.5 spos.
        assert!(close(label_pos(p1, p2, spos, 0.5), Vec2::new(50.0, 20.0)));
        // 1.0: at the child.
        assert!(close(label_pos(p1, p2, spos, 1.0), p2));
        // 0.0: at the parent.
        assert!(close(label_pos(p1, p2, spos, 0.0), p1));
        // 0.25: s_quota 0.25, p_quota 0.75 → 0.5625 p1 + 0.1875 p2 + 0.25 spos.
        assert!(close(
            label_pos(p1, p2, spos, 0.25),
            Vec2::new(18.75 + 12.5, 10.0)
        ));
    }
}
