use crate::{score, AnnIndex, Filter, FlatIndex, Hit};
use atlas_core::{validate_vector, AtlasError, Metric, VectorId};
use serde_json::Value;
use std::cell::RefCell;
use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap};

const M_DEFAULT: usize = 16;
const MAX_LEVEL: usize = 14;
pub const EF_CONSTRUCTION: usize = 128;

thread_local! {
    static VISITED: RefCell<(Vec<u32>, u32)> = const { RefCell::new((Vec::new(), 0)) };
}
pub const EF_SEARCH_DEFAULT: usize = 128;
const EXACT_BELOW: usize = 4096;
const EXACT_SMALL_SET: usize = 2500;
const WALK_COST_FACTOR: usize = 6;

pub fn use_exact_scan(total: usize, allowed: usize, ef_search: usize) -> bool {
    allowed <= EXACT_SMALL_SET
        || allowed.saturating_mul(allowed) <= total.saturating_mul(WALK_COST_FACTOR * ef_search)
}

#[derive(Clone, Copy, PartialEq)]
struct Cand(f32, u32);
impl Eq for Cand {}
impl PartialOrd for Cand {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Cand {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.total_cmp(&o.0).then_with(|| o.1.cmp(&self.1))
    }
}

struct Slot {
    id: VectorId,
    live: bool,
    up: Vec<Vec<u32>>,
}

pub struct HnswIndex {
    flat: FlatIndex,
    metric: Metric,
    ef_search: usize,
    exact_below: usize,
    m: usize,
    m0: usize,
    ef_construction: usize,
    l0: Vec<u32>,
    half: bool,
    patience: usize,
    pf_lines: usize,
    data16: Vec<u16>,
    data: Vec<f32>,
    slots: Vec<Slot>,
    live_slot: HashMap<VectorId, u32>,
    entry: Option<u32>,
    top: usize,
}

#[cfg(target_arch = "x86_64")]
mod simd {
    use std::arch::x86_64::*;

    #[target_feature(enable = "avx2,fma")]
    unsafe fn hsum(v: __m256) -> f32 {
        let s = _mm_add_ps(_mm256_castps256_ps128(v), _mm256_extractf128_ps(v, 1));
        let s = _mm_hadd_ps(s, s);
        let s = _mm_hadd_ps(s, s);
        _mm_cvtss_f32(s)
    }

    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn dot(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len().min(b.len());
        let (pa, pb) = (a.as_ptr(), b.as_ptr());
        let mut acc0 = _mm256_setzero_ps();
        let mut acc1 = _mm256_setzero_ps();
        let mut i = 0;
        while i + 16 <= n {
            acc0 = _mm256_fmadd_ps(_mm256_loadu_ps(pa.add(i)), _mm256_loadu_ps(pb.add(i)), acc0);
            acc1 = _mm256_fmadd_ps(
                _mm256_loadu_ps(pa.add(i + 8)),
                _mm256_loadu_ps(pb.add(i + 8)),
                acc1,
            );
            i += 16;
        }
        if i + 8 <= n {
            acc0 = _mm256_fmadd_ps(_mm256_loadu_ps(pa.add(i)), _mm256_loadu_ps(pb.add(i)), acc0);
            i += 8;
        }
        let mut t = hsum(_mm256_add_ps(acc0, acc1));
        while i < n {
            t += *pa.add(i) * *pb.add(i);
            i += 1;
        }
        t
    }

    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn l2_sq(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len().min(b.len());
        let (pa, pb) = (a.as_ptr(), b.as_ptr());
        let mut acc0 = _mm256_setzero_ps();
        let mut acc1 = _mm256_setzero_ps();
        let mut i = 0;
        while i + 16 <= n {
            let d0 = _mm256_sub_ps(_mm256_loadu_ps(pa.add(i)), _mm256_loadu_ps(pb.add(i)));
            let d1 = _mm256_sub_ps(
                _mm256_loadu_ps(pa.add(i + 8)),
                _mm256_loadu_ps(pb.add(i + 8)),
            );
            acc0 = _mm256_fmadd_ps(d0, d0, acc0);
            acc1 = _mm256_fmadd_ps(d1, d1, acc1);
            i += 16;
        }
        if i + 8 <= n {
            let d0 = _mm256_sub_ps(_mm256_loadu_ps(pa.add(i)), _mm256_loadu_ps(pb.add(i)));
            acc0 = _mm256_fmadd_ps(d0, d0, acc0);
            i += 8;
        }
        let mut t = hsum(_mm256_add_ps(acc0, acc1));
        while i < n {
            let d = *pa.add(i) - *pb.add(i);
            t += d * d;
            i += 1;
        }
        t
    }

    pub fn available() -> bool {
        is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma")
    }

    pub fn prefetch_bytes(p: *const u8, bytes: usize, max_lines: usize) {
        let lines = bytes.div_ceil(64).min(max_lines);
        for l in 0..lines {
            unsafe { _mm_prefetch::<_MM_HINT_T0>(p.add(l * 64) as *const i8) };
        }
    }

    #[target_feature(enable = "avx2,fma,f16c")]
    pub unsafe fn dot_h(q: &[f32], x: &[u16]) -> f32 {
        let n = q.len().min(x.len());
        let (pq, px) = (q.as_ptr(), x.as_ptr());
        let mut acc0 = _mm256_setzero_ps();
        let mut acc1 = _mm256_setzero_ps();
        let mut i = 0;
        while i + 16 <= n {
            let a = _mm256_cvtph_ps(_mm_loadu_si128(px.add(i) as *const __m128i));
            let b = _mm256_cvtph_ps(_mm_loadu_si128(px.add(i + 8) as *const __m128i));
            acc0 = _mm256_fmadd_ps(_mm256_loadu_ps(pq.add(i)), a, acc0);
            acc1 = _mm256_fmadd_ps(_mm256_loadu_ps(pq.add(i + 8)), b, acc1);
            i += 16;
        }
        if i + 8 <= n {
            let a = _mm256_cvtph_ps(_mm_loadu_si128(px.add(i) as *const __m128i));
            acc0 = _mm256_fmadd_ps(_mm256_loadu_ps(pq.add(i)), a, acc0);
            i += 8;
        }
        let mut t = hsum(_mm256_add_ps(acc0, acc1));
        while i < n {
            t += *pq.add(i) * super::f16_to_f32(*px.add(i));
            i += 1;
        }
        t
    }

    #[target_feature(enable = "avx2,fma,f16c")]
    pub unsafe fn l2_h(q: &[f32], x: &[u16]) -> f32 {
        let n = q.len().min(x.len());
        let (pq, px) = (q.as_ptr(), x.as_ptr());
        let mut acc0 = _mm256_setzero_ps();
        let mut acc1 = _mm256_setzero_ps();
        let mut i = 0;
        while i + 16 <= n {
            let a = _mm256_cvtph_ps(_mm_loadu_si128(px.add(i) as *const __m128i));
            let b = _mm256_cvtph_ps(_mm_loadu_si128(px.add(i + 8) as *const __m128i));
            let d0 = _mm256_sub_ps(_mm256_loadu_ps(pq.add(i)), a);
            let d1 = _mm256_sub_ps(_mm256_loadu_ps(pq.add(i + 8)), b);
            acc0 = _mm256_fmadd_ps(d0, d0, acc0);
            acc1 = _mm256_fmadd_ps(d1, d1, acc1);
            i += 16;
        }
        if i + 8 <= n {
            let a = _mm256_cvtph_ps(_mm_loadu_si128(px.add(i) as *const __m128i));
            let d0 = _mm256_sub_ps(_mm256_loadu_ps(pq.add(i)), a);
            acc0 = _mm256_fmadd_ps(d0, d0, acc0);
            i += 8;
        }
        let mut t = hsum(_mm256_add_ps(acc0, acc1));
        while i < n {
            let d = *pq.add(i) - super::f16_to_f32(*px.add(i));
            t += d * d;
            i += 1;
        }
        t
    }

    pub fn available_h() -> bool {
        available() && is_x86_feature_detected!("f16c")
    }
}

fn dot8(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    if simd::available() {
        return unsafe { simd::dot(a, b) };
    }
    dot_portable(a, b)
}

fn l2_sq(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    if simd::available() {
        return unsafe { simd::l2_sq(a, b) };
    }
    l2_portable(a, b)
}

fn prefetch_bytes(p: *const u8, bytes: usize, lines: usize) {
    #[cfg(target_arch = "x86_64")]
    simd::prefetch_bytes(p, bytes, lines);
    #[cfg(not(target_arch = "x86_64"))]
    let _ = (p, bytes, lines);
}

pub fn f32_to_f16(x: f32) -> u16 {
    let x = x.clamp(-65504.0, 65504.0);
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xFF) as i32;
    let man = b & 0x007F_FFFF;
    if exp == 0 {
        return sign;
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7BFF;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = man | 0x0080_0000;
        let shift = (14 - e) as u32;
        let mut h = (m >> shift) as u16;
        let rem = m & ((1 << shift) - 1);
        let half = 1 << (shift - 1);
        if rem > half || (rem == half && h & 1 == 1) {
            h += 1;
        }
        return sign | h;
    }
    let mut h = sign | ((e as u16) << 10) | (man >> 13) as u16;
    let rem = man & 0x1FFF;
    if rem > 0x1000 || (rem == 0x1000 && h & 1 == 1) {
        h += 1;
    }
    h
}

pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let exp = ((h >> 10) & 0x1F) as u32;
    let man = (h & 0x03FF) as u32;
    let bits = if exp == 0 {
        if man == 0 {
            sign
        } else {
            let mut e = 127 - 15 + 1;
            let mut m = man;
            while m & 0x0400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | ((e as u32) << 23) | ((m & 0x03FF) << 13)
        }
    } else if exp == 31 {
        sign | 0x7F80_0000 | (man << 13)
    } else {
        sign | ((exp + 127 - 15) << 23) | (man << 13)
    };
    f32::from_bits(bits)
}

fn dot_h(q: &[f32], x: &[u16]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    if simd::available_h() {
        return unsafe { simd::dot_h(q, x) };
    }
    q.iter().zip(x).map(|(a, b)| a * f16_to_f32(*b)).sum()
}

fn l2_h(q: &[f32], x: &[u16]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    if simd::available_h() {
        return unsafe { simd::l2_h(q, x) };
    }
    q.iter()
        .zip(x)
        .map(|(a, b)| {
            let d = a - f16_to_f32(*b);
            d * d
        })
        .sum()
}

fn dot_portable(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    let mut t: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        t += x * y;
    }
    t
}

fn l2_portable(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for i in 0..8 {
            let d = x[i] - y[i];
            acc[i] += d * d;
        }
    }
    let mut t: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        let d = x - y;
        t += d * d;
    }
    t
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl HnswIndex {
    pub fn new(dim: usize) -> Self {
        Self::with_metric(dim, Metric::Cosine)
    }

    pub fn with_metric(dim: usize, metric: Metric) -> Self {
        Self {
            flat: FlatIndex::new(dim),
            metric,
            ef_search: EF_SEARCH_DEFAULT,
            exact_below: EXACT_BELOW,
            m: M_DEFAULT,
            m0: 2 * M_DEFAULT,
            ef_construction: EF_CONSTRUCTION,
            l0: vec![],
            half: false,
            patience: 0,
            pf_lines: 4,
            data16: vec![],
            data: vec![],
            slots: vec![],
            live_slot: HashMap::new(),
            entry: None,
            top: 0,
        }
    }

    pub fn set_patience(&mut self, expansions: usize) {
        self.patience = expansions;
    }

    pub fn set_prefetch_lines(&mut self, lines: usize) {
        self.pf_lines = lines;
    }

    pub fn with_f16_graph(mut self) -> Self {
        self.half = true;
        self
    }

    pub fn set_ef_search(&mut self, ef: usize) {
        self.ef_search = ef.max(1);
    }

    pub fn with_ef_search(mut self, ef: usize) -> Self {
        self.ef_search = ef.max(1);
        self
    }

    pub fn with_graph_params(mut self, m: usize, ef_construction: usize) -> Self {
        self.m = m.max(2);
        self.m0 = 2 * self.m;
        self.ef_construction = ef_construction.max(self.m);
        self
    }

    pub fn with_exact_below(mut self, n: usize) -> Self {
        self.exact_below = n;
        self
    }

    pub fn factory() -> impl Fn(usize) -> Box<dyn AnnIndex + Send + Sync> + Send + Sync + 'static {
        |dim| Box::new(HnswIndex::new(dim))
    }

    fn dimension(&self) -> usize {
        self.flat.dim()
    }

    fn vec_of(&self, s: u32) -> &[f32] {
        let d = self.dimension();
        &self.data[s as usize * d..(s as usize + 1) * d]
    }

    fn vec_f32(&self, s: u32) -> Vec<f32> {
        if self.half {
            let d = self.dimension();
            self.data16[s as usize * d..(s as usize + 1) * d]
                .iter()
                .map(|&h| f16_to_f32(h))
                .collect()
        } else {
            self.vec_of(s).to_vec()
        }
    }

    fn sim_slot(&self, q: &[f32], s: u32) -> f32 {
        if self.half {
            let d = self.dimension();
            let x = &self.data16[s as usize * d..(s as usize + 1) * d];
            match self.metric {
                Metric::Euclidean => -l2_h(q, x),
                _ => dot_h(q, x),
            }
        } else {
            self.sim(q, self.vec_of(s))
        }
    }

    fn prefetch_slot(&self, s: u32) {
        let d = self.dimension();
        if self.half {
            let x = &self.data16[s as usize * d..];
            prefetch_bytes(x.as_ptr() as *const u8, d * 2, self.pf_lines);
        } else {
            let x = &self.data[s as usize * d..];
            prefetch_bytes(x.as_ptr() as *const u8, d * 4, self.pf_lines);
        }
    }

    fn prep(&self, v: &[f32]) -> Vec<f32> {
        let mut o = v.to_vec();
        if self.metric == Metric::Cosine {
            let n = o.iter().map(|x| x * x).sum::<f32>().sqrt();
            if n > 0.0 {
                o.iter_mut().for_each(|x| *x /= n);
            }
        }
        o
    }

    fn sim(&self, a: &[f32], b: &[f32]) -> f32 {
        match self.metric {
            Metric::Euclidean => -l2_sq(a, b),
            _ => dot8(a, b),
        }
    }

    fn links_of(&self, s: u32, layer: usize) -> &[u32] {
        if layer == 0 {
            let b = s as usize * (self.m0 + 1);
            let n = self.l0[b] as usize;
            &self.l0[b + 1..b + 1 + n]
        } else {
            self.slots[s as usize]
                .up
                .get(layer - 1)
                .map_or(&[], |v| v.as_slice())
        }
    }

    fn set_links(&mut self, s: u32, layer: usize, ls: &[u32]) {
        if layer == 0 {
            let b = s as usize * (self.m0 + 1);
            self.l0[b] = ls.len() as u32;
            self.l0[b + 1..b + 1 + ls.len()].copy_from_slice(ls);
        } else {
            self.slots[s as usize].up[layer - 1] = ls.to_vec();
        }
    }

    fn level_for(&self, slot: u64) -> usize {
        let u = ((splitmix(slot) >> 11) as f64 + 1.0) / (1u64 << 53) as f64;
        ((-u.ln() / (self.m as f64).ln()) as usize).min(MAX_LEVEL)
    }

    fn search_layer(
        &self,
        q: &[f32],
        eps: &[Cand],
        ef: usize,
        layer: usize,
        accept: &dyn Fn(u32) -> bool,
        patience: usize,
    ) -> Vec<Cand> {
        VISITED.with(|cell| {
            let mut guard = cell.borrow_mut();
            let (seen, epoch) = &mut *guard;
            if seen.len() < self.slots.len() {
                seen.resize(self.slots.len(), 0);
            }
            *epoch = epoch.wrapping_add(1);
            if *epoch == 0 {
                seen.fill(0);
                *epoch = 1;
            }
            let ep = *epoch;
            let mut mark = |s: u32| {
                let was = seen[s as usize] == ep;
                seen[s as usize] = ep;
                was
            };
            self.layer_walk(q, eps, ef, layer, accept, &mut mark, patience)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn layer_walk(
        &self,
        q: &[f32],
        eps: &[Cand],
        ef: usize,
        layer: usize,
        accept: &dyn Fn(u32) -> bool,
        mark: &mut dyn FnMut(u32) -> bool,
        patience: usize,
    ) -> Vec<Cand> {
        let mut cands: BinaryHeap<Cand> = BinaryHeap::new();
        let mut res: BinaryHeap<Reverse<Cand>> = BinaryHeap::new();
        let mut fresh: Vec<u32> = Vec::with_capacity(self.m0);
        let mut stale = 0usize;
        for &e in eps {
            if !mark(e.1) {
                cands.push(e);
                if accept(e.1) {
                    res.push(Reverse(e));
                }
            }
        }
        while let Some(c) = cands.pop() {
            if res.len() >= ef && c.0 < res.peek().map_or(f32::MIN, |r| r.0 .0) {
                break;
            }
            if patience > 0 && stale >= patience && res.len() >= ef {
                break;
            }
            let mut grew = false;
            fresh.clear();
            for &nb in self.links_of(c.1, layer) {
                if !mark(nb) {
                    self.prefetch_slot(nb);
                    fresh.push(nb);
                }
            }
            for &nb in &fresh {
                let s = self.sim_slot(q, nb);
                let worst = res.peek().map_or(f32::MIN, |r| r.0 .0);
                if res.len() < ef || s > worst {
                    cands.push(Cand(s, nb));
                    if accept(nb) {
                        res.push(Reverse(Cand(s, nb)));
                        grew = true;
                        if res.len() > ef {
                            res.pop();
                        }
                    }
                }
            }
            stale = if grew { 0 } else { stale + 1 };
        }
        let mut out: Vec<Cand> = res.into_iter().map(|r| r.0).collect();
        out.sort_by(|a, b| b.cmp(a));
        out
    }

    fn select(&self, cands: &[Cand], m: usize) -> Vec<u32> {
        let mut kept: Vec<Cand> = vec![];
        let mut rest: Vec<Cand> = vec![];
        for &c in cands {
            if kept.len() >= m {
                break;
            }
            let cv = self.vec_f32(c.1);
            if kept.iter().all(|k| self.sim_slot(&cv, k.1) < c.0) {
                kept.push(c);
            } else {
                rest.push(c);
            }
        }
        for c in rest {
            if kept.len() >= m {
                break;
            }
            kept.push(c);
        }
        kept.into_iter().map(|c| c.1).collect()
    }

    fn graph_add(&mut self, id: VectorId, v: &[f32]) {
        let s = self.slots.len() as u32;
        let level = self.level_for(s as u64 ^ id.0.rotate_left(17));
        let pv = self.prep(v);
        if self.half {
            self.data16.extend(pv.iter().map(|&x| f32_to_f16(x)));
        } else {
            self.data.extend_from_slice(&pv);
        }
        self.slots.push(Slot {
            id,
            live: true,
            up: vec![vec![]; level],
        });
        self.l0.resize(self.l0.len() + self.m0 + 1, 0);
        self.live_slot.insert(id, s);
        let Some(ep) = self.entry else {
            self.entry = Some(s);
            self.top = level;
            return;
        };
        let all = |_: u32| true;
        let mut eps = vec![Cand(self.sim_slot(&pv, ep), ep)];
        for layer in (level + 1..=self.top).rev() {
            eps = self.search_layer(&pv, &eps, 1, layer, &all, 0);
        }
        for layer in (0..=level.min(self.top)).rev() {
            let w = self.search_layer(&pv, &eps, self.ef_construction, layer, &all, 0);
            let mmax = if layer == 0 { self.m0 } else { self.m };
            let chosen = self.select(&w, self.m);
            self.set_links(s, layer, &chosen);
            for nb in chosen {
                let mut ls = self.links_of(nb, layer).to_vec();
                ls.push(s);
                if ls.len() > mmax {
                    let nv = self.vec_f32(nb);
                    let mut cs: Vec<Cand> =
                        ls.iter().map(|&x| Cand(self.sim_slot(&nv, x), x)).collect();
                    cs.sort_by(|a, b| b.cmp(a));
                    ls = self.select(&cs, mmax);
                }
                self.set_links(nb, layer, &ls);
            }
            eps = w;
        }
        if level > self.top {
            self.top = level;
            self.entry = Some(s);
        }
    }

    fn tombstone(&mut self, id: VectorId) {
        if let Some(s) = self.live_slot.remove(&id) {
            self.slots[s as usize].live = false;
        }
        let dead = self.slots.len() - self.live_slot.len();
        if self.slots.len() > 2 * self.exact_below.max(64) && dead * 2 > self.slots.len() {
            self.rebuild();
        }
    }

    fn rebuild(&mut self) {
        let entries = self.flat.entries();
        self.data.clear();
        self.data16.clear();
        self.l0.clear();
        self.slots.clear();
        self.live_slot.clear();
        self.entry = None;
        self.top = 0;
        for (id, v, _) in entries {
            self.graph_add(id, &v);
        }
    }

    fn rescore(
        &self,
        q: &[f32],
        k: usize,
        metric: Metric,
        ids: impl Iterator<Item = VectorId>,
    ) -> Vec<Hit> {
        let mut hits: Vec<Hit> = ids
            .filter_map(|id| {
                let (v, m) = self.flat.get(id)?;
                Some(Hit {
                    id,
                    score: score(metric, q, v),
                    metadata: m.clone(),
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        hits.truncate(k);
        hits
    }

    fn estimate_allowed(&self, allow: &dyn Fn(VectorId) -> bool) -> usize {
        const SAMPLE: usize = 512;
        let ids = self.flat.ids();
        let n = ids.len();
        if n <= SAMPLE {
            return ids.iter().filter(|&&id| allow(id)).count();
        }
        let hits = (0..SAMPLE).filter(|&i| allow(ids[i * n / SAMPLE])).count();
        if hits < 8 {
            0
        } else {
            hits * n / SAMPLE
        }
    }

    fn graph_search(&self, q: &[f32], k: usize, accept: &dyn Fn(u32) -> bool) -> Vec<VectorId> {
        let Some(ep) = self.entry else {
            return vec![];
        };
        let pq = self.prep(q);
        let all = |_: u32| true;
        let mut eps = vec![Cand(self.sim_slot(&pq, ep), ep)];
        for layer in (1..=self.top).rev() {
            eps = self.search_layer(&pq, &eps, 1, layer, &all, 0);
        }
        let ef = self.ef_search.max(k);
        self.search_layer(&pq, &eps, ef, 0, accept, self.patience)
            .into_iter()
            .map(|c| self.slots[c.1 as usize].id)
            .collect()
    }
}

impl AnnIndex for HnswIndex {
    fn dim(&self) -> usize {
        self.flat.dim()
    }

    fn len(&self) -> usize {
        self.flat.len()
    }

    fn insert(&mut self, id: VectorId, v: &[f32], metadata: Value) -> Result<(), AtlasError> {
        validate_vector(self.flat.dim(), v)?;
        self.flat.insert(id, v, metadata)?;
        self.tombstone(id);
        self.graph_add(id, v);
        Ok(())
    }

    fn entries(&self) -> Vec<(VectorId, Vec<f32>, Value)> {
        self.flat.entries()
    }

    fn matching_ids(&self, filter: &Filter) -> Vec<VectorId> {
        self.flat.matching_ids(filter)
    }

    fn remove(&mut self, id: VectorId) -> bool {
        if !self.flat.remove(id) {
            return false;
        }
        self.tombstone(id);
        true
    }

    fn search(&self, q: &[f32], k: usize, metric: Metric) -> Result<Vec<Hit>, AtlasError> {
        self.search_filtered(q, k, metric, &|_| true)
    }

    fn search_filtered(
        &self,
        q: &[f32],
        k: usize,
        metric: Metric,
        allow: &dyn Fn(VectorId) -> bool,
    ) -> Result<Vec<Hit>, AtlasError> {
        validate_vector(self.flat.dim(), q)?;
        let n = self.flat.len();
        if metric != self.metric || n <= self.exact_below || k == 0 {
            return self.flat.search_filtered(q, k, metric, allow);
        }
        if use_exact_scan(n, self.estimate_allowed(allow), self.ef_search) {
            return self.flat.search_filtered(q, k, metric, allow);
        }
        let accept = |s: u32| self.slots[s as usize].live && allow(self.slots[s as usize].id);
        let ids = self.graph_search(q, k, &accept);
        Ok(self.rescore(q, k, metric, ids.into_iter()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn clustered(n: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
        let mut s = seed | 1;
        let mut rnd = move || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 40) as f32 / (1u64 << 24) as f32) - 0.5
        };
        let centers: Vec<Vec<f32>> = (0..12)
            .map(|_| (0..dim).map(|_| rnd() * 4.0).collect())
            .collect();
        (0..n)
            .map(|i| centers[i % 12].iter().map(|c| c + rnd() * 0.6).collect())
            .collect()
    }

    fn build(n: usize) -> (HnswIndex, FlatIndex, Vec<Vec<f32>>) {
        let data = clustered(n, 16, 3);
        let mut h = HnswIndex::new(16).with_exact_below(256);
        let mut f = FlatIndex::new(16);
        for (i, v) in data.iter().enumerate() {
            h.insert(VectorId(i as u64), v, json!({"i": i})).unwrap();
            f.insert(VectorId(i as u64), v, json!({"i": i})).unwrap();
        }
        (h, f, clustered(60, 16, 99))
    }

    fn recall(
        h: &HnswIndex,
        f: &FlatIndex,
        qs: &[Vec<f32>],
        allow: &dyn Fn(VectorId) -> bool,
    ) -> f64 {
        let (mut hit, mut tot) = (0usize, 0usize);
        for q in qs {
            let ids = |r: Vec<Hit>| r.iter().map(|x| x.id).collect::<Vec<_>>();
            let want = ids(f.search_filtered(q, 10, Metric::Cosine, allow).unwrap());
            let got = ids(h.search_filtered(q, 10, Metric::Cosine, allow).unwrap());
            assert!(got.iter().all(|&i| allow(i)), "leaked a disallowed id");
            hit += want.iter().filter(|w| got.contains(w)).count();
            tot += want.len();
        }
        hit as f64 / tot as f64
    }

    #[test]
    fn passes_shared_conformance_suite() {
        crate::conformance::run(|dim| Box::new(HnswIndex::new(dim)));
    }

    #[test]
    fn conformance_with_graph_forced_on() {
        crate::conformance::run(|dim| Box::new(HnswIndex::new(dim).with_exact_below(0)));
    }

    #[test]
    fn graph_recall_is_high_and_scores_are_exact() {
        let (h, f, qs) = build(3000);
        assert!(recall(&h, &f, &qs, &|_| true) >= 0.95);
        for q in qs.iter().take(5) {
            let a = h.search(q, 10, Metric::Cosine).unwrap();
            let b = f.search(q, 10, Metric::Cosine).unwrap();
            for x in &a {
                if let Some(y) = b.iter().find(|y| y.id == x.id) {
                    assert_eq!(x.score, y.score);
                }
            }
        }
    }

    #[test]
    fn f16_conversion_round_trips_within_half_precision() {
        for x in [
            0.0f32, 1.0, -1.0, 0.5, 3.25, -2.75, 1000.5, 6.1e-5, 1e-7, 65504.0, 1e9,
        ] {
            let r = f16_to_f32(f32_to_f16(x));
            let want = x.clamp(-65504.0, 65504.0);
            let tol = want.abs() * 1e-3 + 1e-4;
            assert!((r - want).abs() <= tol, "{x} -> {r}");
        }
        assert_eq!(f16_to_f32(f32_to_f16(0.0)), 0.0);
        assert!(f16_to_f32(f32_to_f16(-0.0)).is_sign_negative());
    }

    #[test]
    fn f16_graph_keeps_recall_and_exact_scores() {
        let data = clustered(3000, 16, 3);
        let mut h = HnswIndex::new(16).with_exact_below(256).with_f16_graph();
        let mut f = FlatIndex::new(16);
        for (i, v) in data.iter().enumerate() {
            h.insert(VectorId(i as u64), v, json!({"i": i})).unwrap();
            f.insert(VectorId(i as u64), v, json!({"i": i})).unwrap();
        }
        let qs = clustered(60, 16, 99);
        assert!(recall(&h, &f, &qs, &|_| true) >= 0.95);
        for q in qs.iter().take(5) {
            let a = h.search(q, 10, Metric::Cosine).unwrap();
            let b = f.search(q, 10, Metric::Cosine).unwrap();
            for x in &a {
                if let Some(y) = b.iter().find(|y| y.id == x.id) {
                    assert_eq!(x.score, y.score);
                }
            }
        }
        assert!(recall(&h, &f, &qs, &|id| id.0 % 3 == 0) >= 0.95);
    }

    #[test]
    fn patience_stop_keeps_results_valid_and_recall_high() {
        let (mut h, f, qs) = build(3000);
        h.set_patience(32);
        assert!(recall(&h, &f, &qs, &|_| true) >= 0.93);
        assert!(recall(&h, &f, &qs, &|id| id.0 % 2 == 0) >= 0.9);
        h.set_patience(1);
        for q in qs.iter().take(10) {
            let hits = h.search(q, 10, Metric::Cosine).unwrap();
            assert!(!hits.is_empty());
            assert!(hits.windows(2).all(|w| w[0].score >= w[1].score));
        }
    }

    #[test]
    fn filtered_graph_search_keeps_recall_and_never_leaks() {
        let (h, f, qs) = build(3000);
        assert!(recall(&h, &f, &qs, &|id| id.0 % 2 == 0) >= 0.95);
        for m in [2u64, 3, 7] {
            let allow = |id: VectorId| id.0.is_multiple_of(m);
            let (mut hit, mut tot) = (0usize, 0usize);
            for q in &qs {
                let want: Vec<VectorId> = f
                    .search_filtered(q, 10, Metric::Cosine, &allow)
                    .unwrap()
                    .iter()
                    .map(|x| x.id)
                    .collect();
                let accept = |s: u32| h.slots[s as usize].live && allow(h.slots[s as usize].id);
                let got = h.graph_search(q, 10, &accept);
                assert!(got.iter().all(|&i| allow(i)), "leaked a disallowed id");
                hit += want.iter().filter(|w| got.contains(w)).count();
                tot += want.len();
            }
            assert!(hit as f64 / tot as f64 >= 0.9, "walk recall for 1/{m}");
        }
    }

    #[test]
    fn removed_and_replaced_ids_behave() {
        let (mut h, mut f, qs) = build(2000);
        for i in (0..2000u64).step_by(2) {
            assert!(h.remove(VectorId(i)) && f.remove(VectorId(i)));
        }
        for q in &qs {
            let r = h.search(q, 10, Metric::Cosine).unwrap();
            assert!(r.iter().all(|x| x.id.0 % 2 == 1));
        }
        let far = vec![9.0f32; 16];
        h.insert(VectorId(1), &far, json!({"moved": true})).unwrap();
        f.insert(VectorId(1), &far, json!({"moved": true})).unwrap();
        let top = h.search(&far, 1, Metric::Cosine).unwrap();
        assert_eq!(top[0].id, VectorId(1));
        assert_eq!(top[0].metadata, json!({"moved": true}));
        assert!(recall(&h, &f, &qs, &|_| true) >= 0.93);
    }

    #[test]
    fn same_insert_sequence_builds_the_same_answers() {
        let (a, _, qs) = build(1500);
        let (b, _, _) = build(1500);
        for q in &qs {
            let x: Vec<_> = a
                .search(q, 10, Metric::Cosine)
                .unwrap()
                .iter()
                .map(|h| h.id)
                .collect();
            let y: Vec<_> = b
                .search(q, 10, Metric::Cosine)
                .unwrap()
                .iter()
                .map(|h| h.id)
                .collect();
            assert_eq!(x, y);
        }
    }
}
