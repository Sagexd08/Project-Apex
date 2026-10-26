//! SplitMix64 PRNG with a Box-Muller Gaussian sampler (std only).

/// Deterministic SplitMix64 generator.
pub struct SplitMix64 {
    state: u64,
    spare: Option<f64>,
}

impl SplitMix64 {
    /// Create a generator from a seed.
    pub fn new(seed: u64) -> Self {
        Self { state: seed, spare: None }
    }

    /// Next raw 64-bit output.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform integer in `[0, n)` (slight modulo bias, irrelevant here).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Standard normal sample via Box-Muller.
    pub fn gauss(&mut self) -> f64 {
        if let Some(s) = self.spare.take() {
            return s;
        }
        let u1 = 1.0 - self.next_f64();
        let u2 = self.next_f64();
        let r = (-2.0 * u1.ln()).sqrt();
        let t = std::f64::consts::TAU * u2;
        self.spare = Some(r * t.sin());
        r * t.cos()
    }
}
