use atlas_core::{validate_vector, AtlasError, Metric, VectorId};
use atlas_index::{score, AnnIndex, Filter, FlatIndex, Hit};
use serde_json::Value;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

const EXACT_BELOW: usize = 4096;
const EXACT_ALLOWED_PERMILLE: usize = 40;
const MIN_CAPACITY: usize = 1024;
const EF_ADD: usize = 200;
pub const EF_SEARCH_DEFAULT: usize = 512;
pub const EF_SEARCH_LOW_LATENCY: usize = 256;
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
    graph: Option<Index>,
    metric: Metric,
    ef_search: usize,
}

impl UsearchIndex {
    pub fn new(dim: usize) -> Self {
        Self::with_metric(dim, Metric::Cosine)
    }

    pub fn with_metric(dim: usize, metric: Metric) -> Self {
        Self {
            flat: FlatIndex::new(dim),
            graph: None,
            metric,
            ef_search: EF_SEARCH_DEFAULT,
        }
    }

    pub fn with_ef_search(mut self, ef: usize) -> Self {
        self.ef_search = ef;
        self
    }

    pub fn factory() -> impl Fn(usize) -> Box<dyn AnnIndex + Send + Sync> + Send + Sync + 'static {
        |dim| Box::new(UsearchIndex::new(dim))
    }

    pub fn set_ef_search(&mut self, ef: usize) {
        self.ef_search = ef;
        if let Some(g) = self.graph.as_ref() {
            g.change_expansion_search(ef);
        }
    }

    pub fn graph_serialized_len(&self) -> usize {
        self.graph.as_ref().map_or(0, |g| g.serialized_length())
    }

    fn graph_mut(&mut self) -> Result<&mut Index, AtlasError> {
        if self.graph.is_none() {
            let o = IndexOptions {
                dimensions: self.flat.dim(),
                metric: kind(self.metric),
                quantization: ScalarKind::F32,
                connectivity: CONNECTIVITY,
                expansion_add: EF_ADD,
                expansion_search: self.ef_search,
                multi: false,
            };
            let g = Index::new(&o).map_err(|e| graph_err("new", e))?;
            self.graph = Some(g);
        }
        self.graph
            .as_mut()
            .ok_or_else(|| graph_err("new", "graph missing"))
    }

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
        self.flat.insert(id, v, metadata)
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
        if let Some(g) = self.graph.as_ref() {
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
        let keys: Vec<u64> = m.keys.into_iter().filter(|&x| allow(VectorId(x))).collect();
        Ok(self.rescore(q, k, metric, &keys))
    }
}
