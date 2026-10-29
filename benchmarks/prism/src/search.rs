
use crate::lut::Lut;
use crate::quant::{self, CLIP, PLANES};
use crate::store::{Epoch, Snapshot};

pub type Hit = (u64, f32);

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub survivors: Vec<usize>,
    pub bits_prefix: u64,
    pub bits_incr: u64,
    pub reranks: usize,
}

#[derive(Clone, Debug)]
pub struct Calib {
    pub sigma: [f32; 5],
}

pub fn theory_sigma(p: usize, b: usize) -> f64 {
    CLIP as f64 / ((3.0 * p as f64).sqrt() * (1u32 << b) as f64)
}

impl Calib {
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

pub fn search_exact(snap: &Snapshot, q: &[f32], k: usize) -> Vec<Hit> {
    rerank(snap, q, &all_cands(snap), k)
}

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

#[derive(Clone, Copy, Debug)]
pub enum Gate {
    Global(f32),
    PerVector(f32),
    Deterministic,
}

pub fn pv_const(p: usize, delta: f32) -> f32 {
    (2.0 * (2.0 / delta).ln() / p as f32).sqrt()
}

fn dot8(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let tail: f32 = ca.remainder().iter().zip(cb.remainder()).map(|(x, y)| x * y).sum();
    for (x, y) in ca.zip(cb) {
        for ((a, u), v) in acc.iter_mut().zip(x).zip(y) {
            *a += u * v;
        }
    }
    acc.iter().sum::<f32>() + tail
}

pub fn exact_scores(snap: &Snapshot, q: &[f32]) -> Vec<f32> {
    let mut v = Vec::with_capacity(snap.len());
    for seg in &snap.segments {
        v.extend((0..seg.len()).map(|i| dot8(q, seg.vec(i))));
    }
    v
}

pub fn search_exact_fast(snap: &Snapshot, q: &[f32], k: usize) -> Vec<Hit> {
    let ids = snap.segments.iter().flat_map(|s| s.ids.iter().copied());
    let mut h: Vec<Hit> = ids.zip(exact_scores(snap, q)).collect();
    let cmp = |a: &Hit, b: &Hit| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0));
    if k > 0 && h.len() > k {
        h.select_nth_unstable_by(k - 1, cmp);
        h.truncate(k);
    }
    h.sort_by(cmp);
    h
}

pub fn search_exact_stream(snap: &Snapshot, q: &[f32], k: usize) -> Vec<Hit> {
    let cmp = |a: &Hit, b: &Hit| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0));
    let mut top: Vec<Hit> = Vec::with_capacity(k + 1);
    if k == 0 {
        return top;
    }
    for seg in &snap.segments {
        for i in 0..seg.len() {
            let score = dot8(q, seg.vec(i));
            if top.len() == k && score.total_cmp(&top[k - 1].1).is_lt() {
                continue;
            }
            top.push((seg.ids[i], score));
            top.sort_by(cmp);
            top.truncate(k);
        }
    }
    top
}

pub fn score_all_ref(ep: &Epoch, snap: &Snapshot, q: &[f32], b: usize) -> Vec<f32> {
    score_cands(ep, snap, &ep.rotate(q), &all_cands(snap), b)
}

pub fn score_all(ep: &Epoch, snap: &Snapshot, q: &[f32], bs: &[usize]) -> Vec<Vec<f32>> {
    let lut = Lut::new(ep, &ep.rotate(q));
    let maxb = bs.iter().copied().max().unwrap_or(0);
    let mut out = vec![Vec::with_capacity(snap.len()); bs.len()];
    for seg in &snap.segments {
        for i in 0..seg.len() {
            let mut d = [0.0f32; 4];
            for (p, dp) in d.iter_mut().enumerate().take(maxb) {
                *dp = lut.plane_dot(&seg.planes[p], i * seg.words);
            }
            for (o, &b) in out.iter_mut().zip(bs) {
                o.push(lut.score(&d, b));
            }
        }
    }
    out
}

pub fn residuals(snap: &Snapshot, b: usize) -> Vec<f32> {
    snap.segments.iter().flat_map(|s| s.resid.iter().skip(b - 1).step_by(PLANES).copied()).collect()
}

pub fn search_flat_lut(
    ep: &Epoch,
    snap: &Snapshot,
    q: &[f32],
    k: usize,
    bits: usize,
    rerank_n: usize,
) -> (Vec<Hit>, Stats) {
    let cands = all_cands(snap);
    let sc = score_all(ep, snap, q, &[bits]).swap_remove(0);
    let mut order: Vec<usize> = (0..cands.len()).collect();
    if rerank_n > 0 && order.len() > rerank_n {
        order.select_nth_unstable_by(rerank_n - 1, |&a, &b| sc[b].total_cmp(&sc[a]));
        order.truncate(rerank_n);
    }
    let top: Vec<Cand> = order.iter().map(|&i| cands[i]).collect();
    let bits_read = (cands.len() * bits * ep.p) as u64;
    let st = Stats { survivors: vec![cands.len()], bits_prefix: bits_read, bits_incr: bits_read, reranks: top.len() };
    (rerank(snap, q, &top, k), st)
}

pub fn search_cascade_lut(
    ep: &Epoch,
    snap: &Snapshot,
    q: &[f32],
    k: usize,
    schedule: &[usize],
    gate: Gate,
    calib: &Calib,
) -> (Vec<Hit>, Stats) {
    let lut = Lut::new(ep, &ep.rotate(q));
    let mut cands = all_cands(snap);
    let mut ds = vec![[0.0f32; 4]; cands.len()];
    let mut st = Stats::default();
    let mut prev = 0usize;
    for &b in schedule {
        let mut sc = Vec::with_capacity(cands.len());
        for (&(s, i), d) in cands.iter().zip(ds.iter_mut()) {
            let seg = &snap.segments[s as usize];
            for (dp, plane) in d.iter_mut().zip(&seg.planes).take(b).skip(prev) {
                *dp = lut.plane_dot(plane, i as usize * seg.words);
            }
            sc.push(lut.score(d, b));
        }
        st.bits_prefix += (cands.len() * b * ep.p) as u64;
        st.bits_incr += (cands.len() * (b - prev) * ep.p) as u64;
        prev = b;
        let hs: Vec<f32> = match gate {
            Gate::Global(z) => vec![z * calib.sigma[b]; cands.len()],
            Gate::PerVector(delta) => {
                let c = pv_const(ep.p, delta);
                cands.iter().map(|&(s, i)| snap.segments[s as usize].resid[i as usize * PLANES + b - 1] * c).collect()
            }
            Gate::Deterministic => {
                cands.iter().map(|&(s, i)| snap.segments[s as usize].resid[i as usize * PLANES + b - 1]).collect()
            }
        };
        let lcb: Vec<f32> = sc.iter().zip(&hs).map(|(s, h)| s - h).collect();
        let tau = kth_largest(&lcb, k);
        let mask: Vec<bool> = sc.iter().zip(&hs).map(|(s, h)| s + h >= tau).collect();
        let mut m = mask.iter();
        cands.retain(|_| *m.next().unwrap_or(&true));
        let mut m = mask.iter();
        ds.retain(|_| *m.next().unwrap_or(&true));
        st.survivors.push(cands.len());
    }
    st.reranks = cands.len();
    (rerank(snap, q, &cands, k), st)
}
