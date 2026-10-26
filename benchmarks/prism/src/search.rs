//! Error-gated cascade search plus exact and flat-fixed baselines.

use crate::quant::{self, CLIP};
use crate::store::{Epoch, Snapshot};

/// A search hit: `(id, exact cosine)`.
pub type Hit = (u64, f32);

/// Per-query counters.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// Candidates alive after each stage.
    pub survivors: Vec<usize>,
    /// Bits read if each stage re-reads the whole prefix.
    pub bits_prefix: u64,
    /// Bits read if each stage reads only the new planes.
    pub bits_incr: u64,
    /// Exact reranks performed.
    pub reranks: usize,
}

/// Score-error standard deviations per bit width (index 1..=4; index 0 unused).
#[derive(Clone, Debug)]
pub struct Calib {
    /// `sigma[b]` = std of `estimate_b - true cosine`.
    pub sigma: [f32; 5],
}

/// Theoretical sigma: `CLIP / (sqrt(3P) * 2^b)`.
pub fn theory_sigma(p: usize, b: usize) -> f64 {
    CLIP as f64 / ((3.0 * p as f64).sqrt() * (1u32 << b) as f64)
}

impl Calib {
    /// Calibration filled from the theoretical formula.
    pub fn theory(ep: &Epoch) -> Self {
        let mut sigma = [0.0f32; 5];
        for (b, s) in sigma.iter_mut().enumerate().skip(1) {
            *s = theory_sigma(ep.p, b) as f32;
        }
        Calib { sigma }
    }
}

type Cand = (u32, u32);

fn all_cands(snap: &Snapshot) -> Vec<Cand> {
    let mut v = Vec::with_capacity(snap.len());
    for (si, seg) in snap.segments.iter().enumerate() {
        v.extend((0..seg.len() as u32).map(|i| (si as u32, i)));
    }
    v
}

fn score_cands(ep: &Epoch, snap: &Snapshot, qr: &[f32], c: &[Cand], b: usize) -> Vec<f32> {
    let tab = &ep.tabs[b];
    c.iter()
        .map(|&(s, i)| {
            let seg = &snap.segments[s as usize];
            quant::score(qr, &seg.planes, i as usize * seg.words, b, tab)
        })
        .collect()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn sort_hits(h: &mut [Hit]) {
    h.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
}

fn rerank(snap: &Snapshot, q: &[f32], c: &[Cand], k: usize) -> Vec<Hit> {
    let mut h: Vec<Hit> = c
        .iter()
        .map(|&(s, i)| {
            let seg = &snap.segments[s as usize];
            (seg.ids[i as usize], dot(q, seg.vec(i as usize)))
        })
        .collect();
    sort_hits(&mut h);
    h.truncate(k);
    h
}

fn kth_largest(scores: &[f32], k: usize) -> f32 {
    if k == 0 || scores.len() < k {
        return f32::NEG_INFINITY;
    }
    let mut v = scores.to_vec();
    v.select_nth_unstable_by(k - 1, |a, b| b.total_cmp(a));
    v[k - 1]
}

/// Brute-force oracle: exact cosine over everything, ties by ascending id.
pub fn search_exact(snap: &Snapshot, q: &[f32], k: usize) -> Vec<Hit> {
    rerank(snap, q, &all_cands(snap), k)
}

/// Score everything at `bits`, exactly rerank the top `rerank_n`.
pub fn search_flat_fixed(
    ep: &Epoch,
    snap: &Snapshot,
    q: &[f32],
    k: usize,
    bits: usize,
    rerank_n: usize,
) -> (Vec<Hit>, Stats) {
    let qr = ep.rotate(q);
    let cands = all_cands(snap);
    let sc = score_cands(ep, snap, &qr, &cands, bits);
    let mut order: Vec<usize> = (0..cands.len()).collect();
    let id_of = |i: usize| snap.segments[cands[i].0 as usize].ids[cands[i].1 as usize];
    order.sort_by(|&a, &b| sc[b].total_cmp(&sc[a]).then(id_of(a).cmp(&id_of(b))));
    order.truncate(rerank_n);
    let top: Vec<Cand> = order.iter().map(|&i| cands[i]).collect();
    let bits_read = (cands.len() * bits * ep.p) as u64;
    let st = Stats {
        survivors: vec![cands.len()],
        bits_prefix: bits_read,
        bits_incr: bits_read,
        reranks: top.len(),
    };
    (rerank(snap, q, &top, k), st)
}

/// Error-gated cascade: at each stage keep candidates with `s_hat >= tau - 2 z sigma_b`
/// (equivalently `s_hat + z sigma >= tau - z sigma`), then exact-rerank the survivors.
pub fn search_cascade(
    ep: &Epoch,
    snap: &Snapshot,
    q: &[f32],
    k: usize,
    schedule: &[usize],
    z: f32,
    calib: &Calib,
) -> (Vec<Hit>, Stats) {
    let qr = ep.rotate(q);
    let mut cands = all_cands(snap);
    let mut st = Stats::default();
    let mut prev = 0usize;
    for &b in schedule {
        let sc = score_cands(ep, snap, &qr, &cands, b);
        st.bits_prefix += (cands.len() * b * ep.p) as u64;
        st.bits_incr += (cands.len() * (b - prev) * ep.p) as u64;
        prev = b;
        let thr = kth_largest(&sc, k) - 2.0 * z * calib.sigma[b];
        cands = cands.iter().zip(&sc).filter(|(_, &s)| s >= thr).map(|(c, _)| *c).collect();
        st.survivors.push(cands.len());
    }
    st.reranks = cands.len();
    (rerank(snap, q, &cands, k), st)
}

/// Measure sigma_b = std(estimate_b - exact cosine) on a strided sample of
/// `per_query` vectors for each sample query.
pub fn calibrate(ep: &Epoch, snap: &Snapshot, queries: &[Vec<f32>], per_query: usize) -> Calib {
    let cands = all_cands(snap);
    let step = (cands.len() / per_query.max(1)).max(1);
    let (mut sum, mut sq, mut n) = ([0f64; 5], [0f64; 5], 0f64);
    for (qi, q) in queries.iter().enumerate() {
        let qr = ep.rotate(q);
        let sample: Vec<Cand> = cands.iter().skip(qi % step).step_by(step).copied().collect();
        let exact: Vec<f32> =
            sample.iter().map(|&(s, i)| dot(q, snap.segments[s as usize].vec(i as usize))).collect();
        for b in 1..=4 {
            for (e, x) in score_cands(ep, snap, &qr, &sample, b).iter().zip(&exact) {
                let d = (e - x) as f64;
                sum[b] += d;
                sq[b] += d * d;
            }
        }
        n += sample.len() as f64;
    }
    let mut sigma = [0.0f32; 5];
    for b in 1..=4 {
        let m = sum[b] / n;
        sigma[b] = (sq[b] / n - m * m).max(0.0).sqrt() as f32;
    }
    Calib { sigma }
}
