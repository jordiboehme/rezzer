// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A small seeded random number generator (SplitMix64), so a round is the same for the same
//! seed on every platform.

/// SplitMix64 state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng(u64);

impl Rng {
    /// A generator that starts from `seed`.
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`.
    ///
    /// # Panics
    ///
    /// When `n` is 0.
    pub fn below(&mut self, n: u32) -> u32 {
        assert!(n > 0, "Rng::below needs n > 0");
        // The high half of a 64 by 32 bit product: no modulo bias worth the name.
        ((u128::from(self.next_u64()) * u128::from(n)) >> 64) as u32
    }

    /// True with a chance of `per_mille` in 1000 (1000 or more: always).
    pub fn chance(&mut self, per_mille: u32) -> bool {
        self.below(1000) < per_mille
    }
}
