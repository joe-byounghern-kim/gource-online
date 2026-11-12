// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Vector helpers from Core's `vectors.cpp` plus the few C++ library
//! semantics the port needs to reproduce exactly.
//!
//! Gource has two normalisations that round differently:
//! - [`normalise2`] / [`normalise3`] / [`normalise4`] are Core's `normalise`:
//!   divide by the length, and return the vector unchanged when its length is
//!   not positive (zero vectors stay zero, NaN stays NaN).
//! - [`glm_normalize3`] is `glm::normalize`: multiply by `1 / sqrt(dot)`, no
//!   zero check (used by `Plane`).
//!
//! `std::min` / `std::max` / `glm::clamp` differ from `f32::min` / `f32::max`
//! for NaN operands; [`cpp_min`], [`cpp_max`] and [`glm_clamp`] keep the C++
//! operand selection.

use glam::{Vec2, Vec3, Vec4};

/// `PI` from Core's `pi.h`: a double literal, so expressions using it are
/// evaluated in double precision by the C++.
#[allow(clippy::approx_constant)]
pub(crate) const PI: f64 = 3.14159265;

/// `rotate_vec2(v, s, c)`.
#[inline]
pub(crate) fn rotate_vec2(v: Vec2, s: f32, c: f32) -> Vec2 {
    Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// `glm::length(vec2)`: glam's `length()` (same dot product) with the
/// hardware square root. glam's `libm` feature sends `length()` through
/// `libm::sqrtf`, which is a software routine on wasm32 (libm's wasm
/// intrinsics need a nightly cfg); IEEE 754 square root is correctly
/// rounded, so the instruction gives the same bits, several times faster.
#[inline]
pub(crate) fn length(v: Vec2) -> f32 {
    v.dot(v).sqrt()
}

/// `glm::length(vec3)`, as [`length`].
#[inline]
pub(crate) fn length3(v: Vec3) -> f32 {
    v.dot(v).sqrt()
}

/// Core's `normalise(vec2)`.
#[inline]
pub(crate) fn normalise2(v: Vec2) -> Vec2 {
    let l = length(v);
    if l > 0.0 { v / l } else { v }
}

/// Core's `normalise(vec3)`.
#[inline]
pub(crate) fn normalise3(v: Vec3) -> Vec3 {
    let l = length3(v);
    if l > 0.0 { v / l } else { v }
}

/// Core's `normalise(vec4)`. glm sums a vec4 dot product as
/// `(x + y) + (z + w)`, which differs from glam's left-to-right sum.
#[inline]
#[allow(
    dead_code,
    reason = "kept for parity with the C++ API; the simulation does not call it"
)]
pub(crate) fn normalise4(v: Vec4) -> Vec4 {
    let t = v * v;
    let l = ((t.x + t.y) + (t.z + t.w)).sqrt();
    if l > 0.0 { v / l } else { v }
}

/// `glm::normalize(vec3)`: `v * inversesqrt(dot(v, v))`.
#[inline]
pub(crate) fn glm_normalize3(v: Vec3) -> Vec3 {
    v * (1.0 / v.dot(v).sqrt())
}

/// `glm::length2`.
#[inline]
pub(crate) fn length2(v: Vec2) -> f32 {
    v.dot(v)
}

/// `std::min(a, b)`: `(b < a) ? b : a`.
#[inline]
pub(crate) fn cpp_min(a: f32, b: f32) -> f32 {
    if b < a { b } else { a }
}

/// `std::max(a, b)`: `(a < b) ? b : a`.
#[inline]
pub(crate) fn cpp_max(a: f32, b: f32) -> f32 {
    if a < b { b } else { a }
}

/// `glm::clamp(x, lo, hi)`: `min(max(x, lo), hi)` with glm's operand order.
#[inline]
pub(crate) fn glm_clamp(x: f32, lo: f32, hi: f32) -> f32 {
    cpp_min(cpp_max(x, lo), hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_matches_glam_bit_for_bit() {
        // A spread of magnitudes (subnormal to near overflow), signs and the
        // special values, through an xorshift so it is reproducible.
        let mut state = 0x9e37_79b9_u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            f32::from_bits(state)
        };
        let specials = [0.0, -0.0, 1e-45, 1.0, f32::MAX, f32::INFINITY, f32::NAN];
        for i in 0..200_000 {
            let (x, y, z) = if i < specials.len() * specials.len() {
                let a = specials[i % specials.len()];
                let b = specials[i / specials.len()];
                (a, b, -a)
            } else {
                (next(), next(), next())
            };
            let v2 = Vec2::new(x, y);
            let v3 = Vec3::new(x, y, z);
            assert_eq!(length(v2).to_bits(), v2.length().to_bits(), "{v2:?}");
            assert_eq!(length3(v3).to_bits(), v3.length().to_bits(), "{v3:?}");
        }
    }

    #[test]
    fn normalise_keeps_zero_and_divides() {
        assert_eq!(normalise2(Vec2::ZERO), Vec2::ZERO);
        assert_eq!(normalise3(Vec3::ZERO), Vec3::ZERO);
        assert_eq!(normalise2(Vec2::new(3.0, 4.0)), Vec2::new(0.6, 0.8));
        assert!(normalise2(Vec2::new(f32::NAN, 0.0)).x.is_nan());
    }

    #[test]
    fn rotate_quarter_turn() {
        let v = rotate_vec2(Vec2::new(1.0, 0.0), 1.0, 0.0);
        assert_eq!(v, Vec2::new(0.0, 1.0));
    }

    #[test]
    fn cpp_min_max_select_like_the_standard_library() {
        assert_eq!(cpp_min(1.0, 2.0), 1.0);
        assert_eq!(cpp_max(1.0, 2.0), 2.0);
        // std::min(NaN, x) returns NaN (the first operand), unlike f32::min.
        assert!(cpp_min(f32::NAN, 1.0).is_nan());
        assert_eq!(cpp_min(1.0, f32::NAN), 1.0);
        assert_eq!(glm_clamp(5.0, 0.0, 1.0), 1.0);
        assert_eq!(glm_clamp(-5.0, 0.0, 1.0), 0.0);
    }
}
