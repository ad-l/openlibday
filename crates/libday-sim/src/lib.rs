//! Deterministic, headless battle simulation.
//!
//! Placeholder for milestone M4: commands in, events out, integer state and
//! a seeded RNG so replays and network lockstep are reproducible.

pub mod fire;
pub mod path;

/// Small deterministic PRNG (SplitMix64) used for all simulation randomness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform value in `0..bound` (`bound > 0`).
    pub fn below(&mut self, bound: u32) -> u32 {
        (((self.next_u64() >> 32) * bound as u64) >> 32) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_reproducible() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        let xs: Vec<_> = (0..8).map(|_| a.below(100)).collect();
        assert_eq!(xs, (0..8).map(|_| b.below(100)).collect::<Vec<_>>());
        assert!(xs.iter().all(|&x| x < 100));
    }
}
