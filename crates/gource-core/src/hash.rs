// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Core's `stringhash.cpp` (`stringHash`, `vec2Hash`, `vec3Hash`,
//! `colourHash`) and the deterministic replacement for `rand()`.
//!
//! The string hash is reproduced bit for bit against the x86-64 g++ build of
//! Gource:
//! - `str[i]` is a signed `char`, so UTF-8 bytes >= 0x80 contribute negative
//!   values;
//! - `gStringHashSeed ^ (n - i)` is a XOR, not a power;
//! - signed overflow wraps (g++ emits plain `imul`/`add`), and `-INT_MIN`
//!   stays `INT_MIN`, so `stringHash` itself can return `INT_MIN`;
//! - in `vec2Hash` / `vec3Hash` / `colourHash` the optimiser (Gource builds
//!   with `-O2`) inlines `stringHash`, assumes `-val` cannot overflow and
//!   therefore that the hash is non-negative, and emits unsigned division and
//!   remainder. For the one value where that matters (`INT_MIN`) the
//!   components are those of 2147483648; see [`magnitude`]. An `-O0` build
//!   would give negative components there instead.
//!
//! The global `gStringHashSeed` becomes an explicit `seed` argument
//! (`Settings::hash_seed`, default 31). `tests/golden/` holds the C++ harness
//! and the table the unit tests compare against.
//!
//! [`KeyedRng`] replaces `rand()`: a splitmix64 stream over
//! `(seed, key, counter)`, so a given seed, stream key and call sequence give
//! the same numbers on every platform. Each former `rand()` call site owns a
//! stream with its own key (see [`stream_key`]).

use glam::{Vec2, Vec3};

use crate::vectors::{normalise2, normalise3};

/// Gource's default `gStringHashSeed`.
#[allow(
    dead_code,
    reason = "kept for parity with the C++ API; the simulation does not call it"
)]
pub(crate) const DEFAULT_HASH_SEED: i32 = 31;

/// `stringHash(str)` with `gStringHashSeed = seed`.
pub(crate) fn string_hash(s: &[u8], seed: i32) -> i32 {
    // `int n = str.size();`
    let n = s.len() as i32;
    let mut val: i32 = 0;
    for (i, &byte) in s.iter().enumerate() {
        let c = byte as i8 as i32;
        let i = i as i32;
        val = val.wrapping_add(c.wrapping_mul(seed ^ n.wrapping_sub(i)));
    }
    if val < 0 {
        val = val.wrapping_neg();
    }
    val
}

/// The hash as the `-O2` build divides it: `stringHash` ends with
/// `if (val < 0) val = -val`, so g++ treats the result as non-negative and
/// uses unsigned arithmetic; `INT_MIN` therefore acts as 2147483648.
#[inline]
fn magnitude(hash: i32) -> u32 {
    hash as u32
}

/// `(hash / d) % 255` as the `-O2` build computes it.
#[inline]
fn component(hash: i32, d: u32) -> i32 {
    ((magnitude(hash) / d) % 255) as i32
}

/// `vec2Hash(str)`: a unit vector (or zero) derived from the hash.
pub(crate) fn vec2_hash(s: &[u8], seed: i32) -> Vec2 {
    let hash = string_hash(s, seed);
    let x = component(hash, 7) - 127;
    let y = component(hash, 3) - 127;
    normalise2(Vec2::new(x as f32, y as f32))
}

/// `vec3Hash(str)`.
#[allow(
    dead_code,
    reason = "kept for parity with the C++ API; the simulation does not call it"
)]
pub(crate) fn vec3_hash(s: &[u8], seed: i32) -> Vec3 {
    let hash = string_hash(s, seed);
    let x = component(hash, 7) - 127;
    let y = component(hash, 3) - 127;
    let z = component(hash, 1);
    normalise3(Vec3::new(x as f32, y as f32, z as f32))
}

/// `colourHash(str)`: the colour of a file extension or user name.
pub(crate) fn colour_hash(s: &[u8], seed: i32) -> Vec3 {
    let mut hash = string_hash(s, seed);
    if hash == 0 {
        hash += 1;
    }
    let r = component(hash, 7);
    let g = component(hash, 3);
    let b = component(hash, 1);
    normalise3(Vec3::new(r as f32, g as f32, b as f32))
}

/// splitmix64 finaliser.
#[inline]
pub(crate) fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

const GOLDEN_GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;

/// 64-bit FNV-1a of `bytes`, used to turn a stream name or entity path into
/// an RNG key.
pub(crate) const fn stream_key(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    h
}

/// Stateless keyed hash: the `counter`-th value of stream `(seed, key)`.
#[inline]
pub(crate) fn keyed_u64(seed: u64, key: u64, counter: u64) -> u64 {
    let base = mix64(seed ^ mix64(key.wrapping_add(GOLDEN_GAMMA)));
    mix64(base.wrapping_add(counter.wrapping_add(1).wrapping_mul(GOLDEN_GAMMA)))
}

/// Deterministic replacement for `rand()`: values of stream `(seed, key)` in
/// call order. [`KeyedRng::reset`] rewinds it (`Gource::reset` does so, so a
/// restarted simulation repeats itself).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeyedRng {
    seed: u64,
    key: u64,
    counter: u64,
}

impl KeyedRng {
    pub(crate) fn new(seed: u64, key: u64) -> Self {
        KeyedRng {
            seed,
            key,
            counter: 0,
        }
    }

    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn reset(&mut self) {
        self.counter = 0;
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        let v = keyed_u64(self.seed, self.key, self.counter);
        self.counter = self.counter.wrapping_add(1);
        v
    }

    /// A `rand()`-like value in `0..=i32::MAX` (glibc's `RAND_MAX`).
    pub(crate) fn rand(&mut self) -> i32 {
        (self.next_u64() >> 33) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: &str = include_str!("../tests/golden/stringhash_golden.tsv");

    fn decode_hex(s: &str) -> Vec<u8> {
        if s == "-" {
            return Vec::new();
        }
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn bits(s: &str) -> u32 {
        u32::from_str_radix(s, 16).unwrap()
    }

    #[test]
    fn matches_cpp_golden_table() {
        let mut rows = 0;
        for line in GOLDEN.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            assert_eq!(cols.len(), 11, "bad golden row {line}");
            let seed: i32 = cols[0].parse().unwrap();
            let bytes = decode_hex(cols[1]);
            let expected_hash: i32 = cols[2].parse().unwrap();
            let ctx = format!("seed {seed} string {}", cols[1]);

            assert_eq!(string_hash(&bytes, seed), expected_hash, "{ctx}");

            let v2 = vec2_hash(&bytes, seed);
            assert_eq!(v2.x.to_bits(), bits(cols[3]), "vec2Hash.x {ctx}");
            assert_eq!(v2.y.to_bits(), bits(cols[4]), "vec2Hash.y {ctx}");

            let v3 = vec3_hash(&bytes, seed);
            assert_eq!(v3.x.to_bits(), bits(cols[5]), "vec3Hash.x {ctx}");
            assert_eq!(v3.y.to_bits(), bits(cols[6]), "vec3Hash.y {ctx}");
            assert_eq!(v3.z.to_bits(), bits(cols[7]), "vec3Hash.z {ctx}");

            let c = colour_hash(&bytes, seed);
            assert_eq!(c.x.to_bits(), bits(cols[8]), "colourHash.r {ctx}");
            assert_eq!(c.y.to_bits(), bits(cols[9]), "colourHash.g {ctx}");
            assert_eq!(c.z.to_bits(), bits(cols[10]), "colourHash.b {ctx}");
            rows += 1;
        }
        assert!(rows > 400, "golden table too small: {rows} rows");
    }

    #[test]
    fn negative_hash_is_reachable() {
        // -INT_MIN wraps back to INT_MIN in the C++ build.
        assert_eq!(string_hash(&[0x01], i32::MIN ^ 1), i32::MIN);
    }

    #[test]
    fn keyed_rng_is_deterministic_and_keyed() {
        let mut a = KeyedRng::new(31, stream_key(b"a"));
        let mut b = KeyedRng::new(31, stream_key(b"a"));
        let mut c = KeyedRng::new(31, stream_key(b"b"));
        let xs: Vec<i32> = (0..8).map(|_| a.rand()).collect();
        let ys: Vec<i32> = (0..8).map(|_| b.rand()).collect();
        let zs: Vec<i32> = (0..8).map(|_| c.rand()).collect();
        assert_eq!(xs, ys);
        assert_ne!(xs, zs);
        assert!(xs.iter().all(|&x| x >= 0));
        a.reset();
        assert_eq!(a.rand(), xs[0]);
        assert_eq!(keyed_u64(1, 2, 3), keyed_u64(1, 2, 3));
        assert_ne!(keyed_u64(1, 2, 3), keyed_u64(1, 2, 4));
    }
}
