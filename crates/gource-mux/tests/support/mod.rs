// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(dead_code)]

pub mod h264;
pub mod walk;

/// `round(i * 1e6 / fps)`: the controller's frame timestamps.
pub fn pts_us(i: u64, fps: u64) -> i64 {
    ((i * 1_000_000 + fps / 2) / fps) as i64
}

/// xorshift64*: deterministic test data without dependencies.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}
