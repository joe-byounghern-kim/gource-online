// SPDX-License-Identifier: GPL-3.0-or-later

//! Where the former `rand()` call sites draw from ([`Settings::rand_mode`]).
//!
//! - [`RandMode::Keyed`] (the default): each call site owns a
//!   [`KeyedRng`] stream keyed by its name, rewound by every reset, so
//!   browser and cloud runs agree and a restart repeats itself.
//! - [`RandMode::Glibc`] (parity runs against the C++ reference renders):
//!   every call site draws from one shared emulation of glibc's `rand()`,
//!   seeded like a process that never calls `srand` (seed 1), in the call
//!   order of the C++ and never rewound (Gource never reseeds it). A
//!   simulation started in this mode makes the same `rand()` calls as the
//!   C++ build linked against glibc, so overlap tie-breaks and colour
//!   reseeding agree with Gource's.
//!
//! [`Settings::rand_mode`]: crate::settings::Settings::rand_mode

use std::sync::{Arc, Mutex};

use crate::hash::KeyedRng;
use crate::settings::RandMode;

/// glibc's `rand()`: `random_r` with the default `TYPE_3` state (an additive
/// lagged Fibonacci generator `x[i] = x[i-3] + x[i-31]` over 31 words),
/// initialised by `srandom_r`, output `x >> 1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GlibcRand {
    state: [u32; DEG],
    front: usize,
    rear: usize,
}

/// `DEG_3`.
const DEG: usize = 31;
/// `SEP_3`.
const SEP: usize = 3;

impl GlibcRand {
    /// `srandom_r(seed)`: a seed of 0 counts as 1.
    pub(crate) fn new(seed: u32) -> Self {
        let seed = if seed == 0 { 1 } else { seed };
        let mut state = [0u32; DEG];
        state[0] = seed;
        // Park-Miller 16807 by Schrage's method, in glibc's `long int`
        let mut word = i64::from(seed);
        for slot in state.iter_mut().skip(1) {
            let hi = word / 127_773;
            let lo = word % 127_773;
            word = 16_807 * lo - 2_836 * hi;
            if word < 0 {
                word += 2_147_483_647;
            }
            *slot = word as u32;
        }
        let mut rng = GlibcRand {
            state,
            front: SEP,
            rear: 0,
        };
        for _ in 0..DEG * 10 {
            rng.rand();
        }
        rng
    }

    /// `rand()`: a value in `0..=RAND_MAX` (2^31 - 1).
    pub(crate) fn rand(&mut self) -> i32 {
        let v = self.state[self.front].wrapping_add(self.state[self.rear]);
        self.state[self.front] = v;
        self.front = (self.front + 1) % DEG;
        self.rear = (self.rear + 1) % DEG;
        (v >> 1) as i32
    }
}

impl Default for GlibcRand {
    /// The state of a process that never called `srand` (`srand(1)`).
    fn default() -> Self {
        GlibcRand::new(1)
    }
}

/// The generator every call site shares in [`RandMode::Glibc`].
pub(crate) type SharedGlibcRand = Arc<Mutex<GlibcRand>>;

/// A new shared generator for `mode` (`None` in keyed mode).
pub(crate) fn shared_for(mode: RandMode) -> Option<SharedGlibcRand> {
    match mode {
        RandMode::Keyed => None,
        RandMode::Glibc => Some(Arc::new(Mutex::new(GlibcRand::default()))),
    }
}

/// One former `rand()` call site.
#[derive(Clone, Debug)]
pub(crate) enum RandStream {
    /// The site's own keyed stream.
    Keyed(KeyedRng),
    /// glibc's process-wide generator, shared with every other site.
    Glibc(SharedGlibcRand),
}

impl RandStream {
    /// The keyed stream `(seed, key)`, or a handle on `shared` when given.
    pub(crate) fn new(seed: u64, key: u64, shared: Option<&SharedGlibcRand>) -> Self {
        match shared {
            Some(g) => RandStream::Glibc(Arc::clone(g)),
            None => RandStream::Keyed(KeyedRng::new(seed, key)),
        }
    }

    /// What `Gource::reset` does to the site: a keyed stream restarts as
    /// `(seed, key)`; glibc's generator is left alone (Gource never
    /// reseeds it).
    pub(crate) fn rewind(&mut self, seed: u64, key: u64) {
        if let RandStream::Keyed(k) = self {
            *k = KeyedRng::new(seed, key);
        }
    }

    pub(crate) fn rand(&mut self) -> i32 {
        match self {
            RandStream::Keyed(k) => k.rand(),
            RandStream::Glibc(g) => g.lock().unwrap_or_else(|e| e.into_inner()).rand(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// First 1000 values of `rand()` from glibc 2.39 (x86-64), printed by
    /// `scripts/parity/glibc-rand.c`.
    const GLIBC_RAND_1000: &str = include_str!("../tests/golden/glibc_rand_1000.txt");

    #[test]
    fn matches_glibc_rand() {
        let want: Vec<i32> = GLIBC_RAND_1000
            .lines()
            .map(|l| l.trim().parse().unwrap())
            .collect();
        assert_eq!(want.len(), 1000);
        let mut rng = GlibcRand::default();
        let got: Vec<i32> = (0..1000).map(|_| rng.rand()).collect();
        assert_eq!(got, want);
        assert_eq!(GlibcRand::new(0), GlibcRand::new(1));
    }

    #[test]
    fn glibc_sites_share_one_sequence() {
        let shared = shared_for(RandMode::Glibc).unwrap();
        let mut a = RandStream::new(31, 1, Some(&shared));
        let mut b = RandStream::new(31, 2, Some(&shared));
        let mut reference = GlibcRand::default();
        for _ in 0..10 {
            assert_eq!(a.rand(), reference.rand());
            assert_eq!(b.rand(), reference.rand());
        }
        a.rewind(31, 1);
        assert_eq!(a.rand(), reference.rand(), "rewind leaves glibc alone");
    }

    #[test]
    fn keyed_sites_match_keyed_rng() {
        assert!(shared_for(RandMode::Keyed).is_none());
        let mut s = RandStream::new(31, 7, None);
        let mut k = KeyedRng::new(31, 7);
        let first = s.rand();
        assert_eq!(first, k.rand());
        assert_eq!(s.rand(), k.rand());
        s.rewind(31, 7);
        assert_eq!(s.rand(), first);
    }
}
