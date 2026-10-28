//! `AnnIndex` backed by a USearch HNSW graph, with `FlatIndex` as the exact store and fallback.
//!
//! The graph is DERIVED data: it is rebuilt from the WAL on open and never persisted or trusted.
//! `FlatIndex` holds the authoritative vectors and metadata (so memory is about 2x the vectors;
//! ponytail: drop the duplicate by reading vectors back from the graph when memory matters).
//!
//! Routing (every path returns scores computed by `atlas_index::score`, ties by ascending id):
//! - metric differs from the graph metric, or the index is small: exact scan (`FlatIndex`);
//! - unfiltered, large: HNSW candidates, rescored exactly;
//! - filtered: count the allowed set; at or below 4% of the index an exact scan over only the
//!   allowed ids (native HNSW filtering degrades at low selectivity, see docs/evidence), above it
//!   a filtered HNSW walk.
use atlas_core::{validate_vector, AtlasError, Metric, VectorId};
use atlas_index::{score, AnnIndex, FlatIndex, Hit};
use serde_json::Value;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

/// Below this many vectors an exact scan is faster than a graph walk and exactly right.
const EXACT_BELOW: usize = 4096;
/// Exact-scan the allowed set when it is at most this share of the index (permille). Measured
/// crossover vs native filtered HNSW on SIFT1M, GloVe-100, NYTimes-256 and Fashion-MNIST is
/// about 4 to 5% (benchmarks/ann-spike/results/real_*.log).
const EXACT_ALLOWED_PERMILLE: usize = 40;
const MIN_CAPACITY: usize = 1024;
// Chosen from the M/efc/ef sweep on GloVe-100 and NYTimes-256 (benchmarks/ann-spike/results):
// M=32/efc=200 reaches recall@10 0.93-0.94 at ef=256 and 0.96 at ef=512 on the hard sets, at about
// half the build time of M=48/efc=400.
const EF_ADD: usize = 200;
const EF_SEARCH: usize = 256;
const CONNECTIVITY: usize = 32;

fn graph_err(what: &str, e: impl std::fmt::Display) -> AtlasError {
    AtlasError::Codec(format!("usearch {what}: {e}"))
}

fn kind(m: Metric) -> MetricKind {
    match m {
        Metric::Cosine => MetricKind::Cos,
        Metric::Dot => MetricKind::IP,
        Metric::Euclidean => MetricKind::L2sq,
    }
}

pub struct UsearchIndex {
    flat: FlatIndex,
    /// Created on first insert so construction cannot fail.
    graph: Option<Index>,
    metric: Metric,
}

impl UsearchIndex {
    /// Cosine graph (the common case); other metrics fall back to exact scans.
    pub fn new(dim: usize) -> Self {
        Self::with_metric(dim, Metric::Cosine)
    }

    pub fn with_metric(dim: usize, metric: Metric) -> Self {
        Self {
            flat: FlatIndex::new(dim),
            graph: None,
            metric,
        }
    }

    /// Factory for `Engine::open_with_index`.
    pub fn factory() -> impl Fn(usize) -> Box<dyn AnnIndex + Send + Sync> + Send + Sync + 'static {
        |dim| Box::new(UsearchIndex::new(dim))
    }

    fn graph_mut(&mut self) -> Result<&mut Index, AtlasError> {
        if self.graph.is_none() {
            let o = IndexOptions {
                dimensions: self.flat.dim(),
                metric: kind(self.metric),
                quantization: ScalarKind::F32,
                connectivity: CONNECTIVITY,
                expansion_add: EF_ADD,
                expansion_search: EF_SEARCH,
                multi: false,
            };
            let g = Index::new(&o).map_err(|e| graph_err("new", e))?;
            self.graph = Some(g);
        }
        self.graph
            .as_mut()
            .ok_or_else(|| graph_err("new", "graph missing"))
    }

    /// Rescore graph candidates exactly and order them like every other path.
    fn rescore(&self, q: &[f32], k: usize, metric: Metric, keys: &[u64]) -> Vec<Hit> {
        let mut hits: Vec<Hit> = keys
            .iter()
            .filter_map(|&key| {
                let (v, m) = self.flat.get(VectorId(key))?;
                Some(Hit {
                    id: VectorId(key),
                    score: score(metric, q, v),
                    metadata: m.clone(),
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        hits.truncate(k);
        hits
    }

    fn graph_usable(&self, metric: Metric) -> Option<&Index> {
        if metric != self.metric || self.flat.len() <= EXACT_BELOW {
            return None;
        }
        self.graph.as_ref()
    }
}

impl AnnIndex for UsearchIndex {
    fn dim(&self) -> usize {
        self.flat.dim()
    }

    fn len(&self) -> usize {
        self.flat.len()
    }

    fn insert(&mut self, id: VectorId, v: &[f32], metadata: Value) -> Result<(), AtlasError> {
        validate_vector(self.flat.dim(), v)?;
        let replace = self.flat.get(id).is_some();
        let (size, cap) = {
            let g = self.graph_mut()?;
            (g.size(), g.capacity())
        };
        if size + 1 > cap {
            let g = self.graph_mut()?;
            g.reserve((cap * 2).max(MIN_CAPACITY))
                .map_err(|e| graph_err("reserve", e))?;
        }
        let g = self.graph_mut()?;
        if replace {
            g.remove(id.0).map_err(|e| graph_err("remove", e))?;
        }
        g.add(id.0, v).map_err(|e| graph_err("add", e))?;
        // The exact store is updated last: a graph failure above leaves it untouched.
        self.flat.insert(id, v, metadata)
    }

    fn entries(&self) -> Vec<(VectorId, Vec<f32>, Value)> {
        self.flat.entries()
    }

    fn remove(&mut self, id: VectorId) -> bool {
        if !self.flat.remove(id) {
            return false;
        }
        if let Some(g) = self.graph.as_ref() {
            // A failed graph removal cannot resurrect the id: results are rescored from `flat`,
            // which no longer has it (rescore drops unknown keys).
            let _ = g.remove(id.0);
        }
        true
    }

    fn search(&self, q: &[f32], k: usize, metric: Metric) -> Result<Vec<Hit>, AtlasError> {
        validate_vector(self.flat.dim(), q)?;
        let Some(g) = self.graph_usable(metric) else {
            return self.flat.search(q, k, metric);
        };
        let m = g.search(q, k).map_err(|e| graph_err("search", e))?;
        Ok(self.rescore(q, k, metric, &m.keys))
    }

    fn search_filtered(
        &self,
        q: &[f32],
        k: usize,
        metric: Metric,
        allow: &dyn Fn(VectorId) -> bool,
    ) -> Result<Vec<Hit>, AtlasError> {
        validate_vector(self.flat.dim(), q)?;
        let Some(g) = self.graph_usable(metric) else {
            return self.flat.search_filtered(q, k, metric, allow);
        };
        let n = self.flat.len();
        let allowed = self.flat.ids().iter().filter(|&&id| allow(id)).count();
        if allowed * 1000 <= n * EXACT_ALLOWED_PERMILLE {
            return self.flat.search_filtered(q, k, metric, allow);
        }
        let m = g
            .filtered_search(q, k, |key| allow(VectorId(key)))
            .map_err(|e| graph_err("filtered_search", e))?;
        // Belt and braces for the zero-leak rule: never return an id the predicate rejects.
        let keys: Vec<u64> = m.keys.into_iter().filter(|&x| allow(VectorId(x))).collect();
        Ok(self.rescore(q, k, metric, &keys))
    }
}
