use atlas_core::{AtlasError, Metric, VectorId};
use atlas_index::{AnnIndex, FlatIndex};
use serde_json::{json, Value};
use std::time::Instant;

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn unit(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }
}

pub struct Dataset {
    pub dim: usize,
    pub base: Vec<Vec<f32>>,
    pub queries: Vec<Vec<f32>>,
}

pub fn synthetic(seed: u64, n: usize, nq: usize, dim: usize) -> Dataset {
    let mut r = Rng::new(seed);
    let mut gen = |count: usize| -> Vec<Vec<f32>> {
        (0..count)
            .map(|_| (0..dim).map(|_| r.unit()).collect())
            .collect()
    };
    let base = gen(n);
    let queries = gen(nq);
    Dataset { dim, base, queries }
}

pub fn recall_at_k(found: &[VectorId], truth: &[VectorId], k: usize) -> f64 {
    let t = &truth[..truth.len().min(k)];
    if t.is_empty() {
        return 1.0;
    }
    let hit = found.iter().take(k).filter(|f| t.contains(f)).count();
    hit as f64 / t.len() as f64
}

pub fn mrr(found: &[VectorId], truth: &[VectorId]) -> f64 {
    found
        .iter()
        .position(|f| truth.contains(f))
        .map_or(0.0, |p| 1.0 / (p as f64 + 1.0))
}

pub fn ndcg_at_k(found: &[VectorId], truth: &[VectorId], k: usize) -> f64 {
    let rel = |id: &VectorId| truth.contains(id) as u8 as f64;
    let dcg: f64 = found
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, id)| rel(id) / ((i + 2) as f64).log2())
        .sum();
    let ideal: f64 = (0..truth.len().min(k))
        .map(|i| 1.0 / ((i + 2) as f64).log2())
        .sum();
    if ideal == 0.0 {
        1.0
    } else {
        dcg / ideal
    }
}

pub fn percentile(sorted_ms: &[f64], q: f64) -> f64 {
    if sorted_ms.is_empty() {
        return 0.0;
    }
    let i = ((sorted_ms.len() as f64 * q).ceil() as usize).clamp(1, sorted_ms.len()) - 1;
    sorted_ms[i]
}

pub fn evaluate<I: AnnIndex>(
    index: &I,
    data: &Dataset,
    metric: Metric,
    k: usize,
    seed: u64,
) -> Result<Value, AtlasError> {
    let mut exact = FlatIndex::new(data.dim);
    for (i, v) in data.base.iter().enumerate() {
        exact.insert(VectorId(i as u64), v, Value::Null)?;
    }
    let (mut rec, mut nd, mut rr) = (0.0, 0.0, 0.0);
    let mut lat = Vec::with_capacity(data.queries.len());
    for q in &data.queries {
        let truth: Vec<VectorId> = exact
            .search(q, k, metric)?
            .into_iter()
            .map(|h| h.id)
            .collect();
        let t = Instant::now();
        let hits = index.search(q, k, metric)?;
        lat.push(t.elapsed().as_secs_f64() * 1e3);
        let found: Vec<VectorId> = hits.into_iter().map(|h| h.id).collect();
        rec += recall_at_k(&found, &truth, k);
        nd += ndcg_at_k(&found, &truth, k);
        rr += mrr(&found, &truth);
    }
    lat.sort_by(|a, b| a.total_cmp(b));
    let n = data.queries.len().max(1) as f64;
    Ok(json!({
        "seed": seed,
        "n": data.base.len(),
        "queries": data.queries.len(),
        "dim": data.dim,
        "k": k,
        "metric": format!("{metric:?}"),
        "recall_at_k": rec / n,
        "ndcg_at_k": nd / n,
        "mrr": rr / n,
        "latency_ms": {
            "p50": percentile(&lat, 0.50),
            "p95": percentile(&lat, 0.95),
            "p99": percentile(&lat, 0.99),
        },
        "env": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "cargo_pkg_version": env!("CARGO_PKG_VERSION"),
        },
    }))
}
