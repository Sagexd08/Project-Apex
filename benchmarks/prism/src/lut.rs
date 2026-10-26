//! Per-query byte lookup tables: plane dot products without per-coordinate decoding.
//!
//! With `idx_b = sum_{p<b} bit_p 2^(b-1-p)` and `recon_b = (idx_b + 0.5) * 2s/2^b - s`:
//! `score_b = sum_{p<b} (s 2^-p) D_p + (s 2^-b - s) sum_i q_i`, where `D_p` is the sum of the
//! rotated query over the set bits of plane `p`.

use crate::store::Epoch;

/// Lookup tables for one rotated query.
pub struct Lut {
    t: Vec<f32>,
    words: usize,
    w: [f32; 4],
    off: [f32; 5],
}

impl Lut {
    /// Build `T_j[byte]` (sum of `qr` over the set bits of the byte, covering coords `8j..8j+8`).
    pub fn new(ep: &Epoch, qr: &[f32]) -> Self {
        let nb = ep.words * 8;
        let mut t = vec![0.0f32; nb * 256];
        for (j, tj) in t.chunks_exact_mut(256).enumerate() {
            for v in 1..256usize {
                let c = j * 8 + v.trailing_zeros() as usize;
                tj[v] = tj[v & (v - 1)] + qr.get(c).copied().unwrap_or(0.0);
            }
        }
        let (s, sum_q) = (ep.half_range(), qr.iter().sum::<f32>());
        let w = [s, s / 2.0, s / 4.0, s / 8.0];
        let mut off = [0.0f32; 5];
        for (b, o) in off.iter_mut().enumerate() {
            *o = (s / (1u32 << b) as f32 - s) * sum_q;
        }
        Self { t, words: ep.words, w, off }
    }

    /// `D_p` for the vector whose words start at `base` in `plane`.
    pub fn plane_dot(&self, plane: &[u64], base: usize) -> f32 {
        let mut a = [0.0f32; 8];
        for (w, &x) in plane[base..base + self.words].iter().enumerate() {
            let t = &self.t[w * 2048..(w + 1) * 2048];
            for (m, am) in a.iter_mut().enumerate() {
                *am += t[m * 256 + ((x >> (8 * m)) & 0xFF) as usize];
            }
        }
        a.iter().sum()
    }

    /// Score at `b` bits from the plane sums `d[0..b]`.
    pub fn score(&self, d: &[f32; 4], b: usize) -> f32 {
        d.iter().zip(&self.w).take(b).map(|(x, w)| x * w).sum::<f32>() + self.off[b]
    }
}
