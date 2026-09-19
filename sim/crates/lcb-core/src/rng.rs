//! Deterministic RNG for the simulator.
//!
//! **Provenance note.** The client's RNG is not observable from the data this
//! project is allowed to use, so the generator below is *simulator-defined*
//! (`Synthetic`): xoshiro256** seeded through splitmix64.  What *is* sourced is
//! the coin-flip model, `H = 50 + SP` percent heads
//! (wiki.gg `Sanity`, `Clash`), which `flip_percent` implements.
//!
//! Replays never depend on the generator being re-run identically in a
//! different build: every flipped coin is recorded in the replay log.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rng {
    state: [u64; 4],
    draw_count: u64,
}

fn splitmix64(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub fn from_seed(seed: u64) -> Self {
        let mut s = seed;
        Self {
            state: [
                splitmix64(&mut s),
                splitmix64(&mut s),
                splitmix64(&mut s),
                splitmix64(&mut s),
            ],
            draw_count: 0,
        }
    }

    pub fn draw_count(&self) -> u64 {
        self.draw_count
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.state[1]
            .wrapping_mul(5)
            .rotate_left(7)
            .wrapping_mul(9);
        let t = self.state[1] << 17;
        self.state[2] ^= self.state[0];
        self.state[3] ^= self.state[1];
        self.state[1] ^= self.state[2];
        self.state[0] ^= self.state[3];
        self.state[2] ^= t;
        self.state[3] = self.state[3].rotate_left(45);
        self.draw_count += 1;
        result
    }

    /// Uniform integer in `[0, bound)`; `bound` must be non-zero.
    pub fn below(&mut self, bound: u32) -> u32 {
        assert!(bound > 0, "bound must be non-zero");
        // Lemire's multiply-shift, unbiased enough for our 1/10000 granularity.
        ((self.next_u64() >> 32) as u64 * bound as u64 >> 32) as u32
    }

    /// A coin flip with `percent` (0..=100) chance of heads.
    pub fn flip_percent(&mut self, percent: i32) -> bool {
        let p = percent.clamp(0, 100) as u32;
        self.below(100) < p
    }
}

/// Heads chance in percent: `H = 50 + SP` (wiki.gg `Sanity`).
pub fn heads_chance(sp: i32) -> i32 {
    (50 + sp).clamp(0, 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_same_seed() {
        let mut a = Rng::from_seed(42);
        let mut b = Rng::from_seed(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::from_seed(1);
        let mut b = Rng::from_seed(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn flip_bounds_are_exact() {
        let mut rng = Rng::from_seed(7);
        for _ in 0..1000 {
            assert!(!rng.flip_percent(0));
            assert!(rng.flip_percent(100));
        }
    }

    #[test]
    fn heads_chance_matches_source() {
        assert_eq!(heads_chance(0), 50);
        assert_eq!(heads_chance(45), 95);
        assert_eq!(heads_chance(-45), 5);
    }
}
