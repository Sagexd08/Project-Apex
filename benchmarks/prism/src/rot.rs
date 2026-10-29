
use crate::rng::SplitMix64;

const ROUNDS: usize = 3;

pub struct Rotation {
    dim: usize,
    p: usize,
    signs: [Vec<f32>; ROUNDS],
}

impl Rotation {
    pub fn new(dim: usize, seed: u64) -> Self {
        let p = dim.next_power_of_two();
        let mut rng = SplitMix64::new(seed);
        let mut diag = || -> Vec<f32> {
            (0..p).map(|_| if rng.next_u64() & 1 == 0 { 1.0 } else { -1.0 }).collect()
        };
        let signs = [diag(), diag(), diag()];
        Self { dim, p, signs }
    }

    pub fn padded_dim(&self) -> usize {
        self.p
    }

    pub fn apply(&self, v: &[f32]) -> Vec<f32> {
        assert_eq!(v.len(), self.dim, "vector length != rotation dim");
        let mut out = vec![0.0f32; self.p];
        out[..self.dim].copy_from_slice(v);
        for diag in &self.signs {
            for (x, s) in out.iter_mut().zip(diag) {
                *x *= s;
            }
            fwht(&mut out);
        }
        out
    }
}

pub fn fwht(x: &mut [f32]) {
    let n = x.len();
    debug_assert!(n.is_power_of_two());
    let mut h = 1;
    while h < n {
        for block in x.chunks_mut(2 * h) {
            let (lo, hi) = block.split_at_mut(h);
            for (a, b) in lo.iter_mut().zip(hi.iter_mut()) {
                let (u, v) = (*a, *b);
                *a = u + v;
                *b = u - v;
            }
        }
        h *= 2;
    }
    let sc = 1.0 / (n as f32).sqrt();
    x.iter_mut().for_each(|v| *v *= sc);
}
