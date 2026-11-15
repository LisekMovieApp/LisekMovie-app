//! A tiny deterministic generator (splitmix64). The `rand` crate may change
//! its algorithms between versions; a stored seed must always give the same
//! shuffle, so the few lines live here.

/// One splitmix64 step of `x`.
pub fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Combines two values into one (order matters).
pub fn mix2(a: u64, b: u64) -> u64 {
    mix(a ^ mix(b))
}

/// A seed from a UUID (its two halves folded).
pub fn uuid_seed(id: uuid::Uuid) -> u64 {
    let (hi, lo) = id.as_u64_pair();
    mix2(hi, lo)
}

/// A stream of numbers from a seed.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(mix(seed))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = mix(self.0);
        self.0
    }

    /// A number in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// A seeded permutation of `0..n` (Fisher-Yates).
pub fn permutation(n: usize, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    let mut rng = Rng::new(seed);
    for i in (1..n).rev() {
        order.swap(i, rng.below(i + 1));
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_permutation() {
        assert_eq!(permutation(20, 7), permutation(20, 7));
        assert_ne!(permutation(20, 7), permutation(20, 8));
        let mut sorted = permutation(20, 7);
        sorted.sort_unstable();
        assert_eq!(sorted, (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn the_generator_is_pinned() {
        // A stored seed must keep meaning the same shuffle across releases.
        assert_eq!(permutation(5, 1), vec![0, 2, 1, 4, 3]);
    }
}
