//! `AnnIndex` seam plus the exact flat-scan index (correctness oracle and fallback).
//! ponytail: FlatIndex is O(n log n) per query; ANN impls plug in via `AnnIndex`.
use atlas_core::{validate_vector, AtlasError, Metric, VectorId};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: VectorId,
    /// Higher is better. Euclidean is the negated distance.
    pub score: f32,
    pub metadata: Value,
}

pub struct FlatIndex {
    dim: usize,
    pos: HashMap<VectorId, usize>,
    ids: Vec<VectorId>,
    data: Vec<f32>,
    meta: Vec<Value>,
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub fn score(metric: Metric, a: &[f32], b: &[f32]) -> f32 {
    match metric {
        Metric::Dot => dot(a, b),
        Metric::Euclidean => -a
            .iter()
            .zip(b)
            .map(|(x, y)| (x - y) * (x - y))
            .sum::<f32>()
            .sqrt(),
        Metric::Cosine => {
            let n = (dot(a, a) * dot(b, b)).sqrt();
            if n == 0.0 {
                0.0
            } else {
                dot(a, b) / n
            }
        }
    }
}

/// Pluggable nearest-neighbour index. A derived structure: the WAL is the source of truth and
/// implementations are rebuilt from it on open (never trust persisted index files).
pub trait AnnIndex {
    fn dim(&self) -> usize;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Insert or replace (same id, new vector/metadata).
    fn insert(&mut self, id: VectorId, v: &[f32], metadata: Value) -> Result<(), AtlasError>;
    /// Returns whether the id was present.
    fn remove(&mut self, id: VectorId) -> bool;
    /// Top-k, best first; ties break by ascending id.
    fn search(&self, q: &[f32], k: usize, metric: Metric) -> Result<Vec<Hit>, AtlasError>;
    /// Top-k among ids where `allow` is true. Default is exact (full search, then filter);
    /// ANN impls should override with a real filtered walk / pre-filter.
    fn search_filtered(
        &self,
        q: &[f32],
        k: usize,
        metric: Metric,
        allow: &dyn Fn(VectorId) -> bool,
    ) -> Result<Vec<Hit>, AtlasError> {
        let mut hits = self.search(q, self.len(), metric)?;
        hits.retain(|h| allow(h.id));
        hits.truncate(k);
        Ok(hits)
    }
}

/// Pluggable nearest-neighbour index. A derived structure: the WAL is the source of truth and
/// implementations are rebuilt from it on open (never trust persisted index files).
pub trait AnnIndex {
    fn dim(&self) -> usize;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Insert or replace (same id, new vector/metadata).
    fn insert(&mut self, id: VectorId, v: &[f32], metadata: Value) -> Result<(), AtlasError>;
    /// Returns whether the id was present.
    fn remove(&mut self, id: VectorId) -> bool;
    /// Top-k, best first; ties break by ascending id.
    fn search(&self, q: &[f32], k: usize, metric: Metric) -> Result<Vec<Hit>, AtlasError>;
    /// Top-k among ids where `allow` is true. Default is exact (full search, then filter);
    /// ANN impls should override with a real pre-filter / filtered walk.
    fn search_filtered(
        &self,
        q: &[f32],
        k: usize,
        metric: Metric,
        allow: &dyn Fn(VectorId) -> bool,
    ) -> Result<Vec<Hit>, AtlasError> {
        let mut hits = self.search(q, self.len(), metric)?;
        hits.retain(|h| allow(h.id));
        hits.truncate(k);
        Ok(hits)
    }
}

impl FlatIndex {
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            pos: HashMap::new(),
            ids: vec![],
            data: vec![],
            meta: vec![],
        }
    }
}

impl AnnIndex for FlatIndex {
    fn dim(&self) -> usize {
        self.dim
    }

    fn len(&self) -> usize {
        self.ids.len()
    }

    fn insert(&mut self, id: VectorId, v: &[f32], metadata: Value) -> Result<(), AtlasError> {
        validate_vector(self.dim, v)?;
        if let Some(&p) = self.pos.get(&id) {
            self.data[p * self.dim..(p + 1) * self.dim].copy_from_slice(v);
            self.meta[p] = metadata;
        } else {
            self.pos.insert(id, self.ids.len());
            self.ids.push(id);
            self.data.extend_from_slice(v);
            self.meta.push(metadata);
        }
        Ok(())
    }

    fn remove(&mut self, id: VectorId) -> bool {
        let Some(p) = self.pos.remove(&id) else {
            return false;
        };
        let last = self.ids.len() - 1;
        let d = self.dim;
        if p != last {
            self.data.copy_within(last * d..(last + 1) * d, p * d);
            self.ids[p] = self.ids[last];
            self.pos.insert(self.ids[p], p);
        }
        self.ids.truncate(last);
        self.data.truncate(last * d);
        self.meta.swap_remove(p);
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
        validate_vector(self.dim, q)?;
        let mut hits: Vec<(f32, usize)> = (0..self.ids.len())
            .filter(|&i| allow(self.ids[i]))
            .map(|i| {
                (
                    score(metric, q, &self.data[i * self.dim..(i + 1) * self.dim]),
                    i,
                )
            })
            .collect();
        hits.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then_with(|| self.ids[a.1].cmp(&self.ids[b.1]))
        });
        hits.truncate(k);
        Ok(hits
            .into_iter()
            .map(|(s, i)| Hit {
                id: self.ids[i],
                score: s,
                metadata: self.meta[i].clone(),
            })
            .collect())
    }
}

pub mod conformance;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn metrics_rank_and_replace() {
        let mut ix = FlatIndex::new(2);
        assert!(ix.is_empty());
        ix.insert(VectorId(1), &[1.0, 0.0], json!({"a": 1}))
            .unwrap();
        ix.insert(VectorId(2), &[0.0, 2.0], json!(null)).unwrap();
        ix.insert(VectorId(3), &[0.0, 0.0], json!(null)).unwrap();
        let top = |m| ix.search(&[0.0, 1.0], 1, m).unwrap()[0].id;
        assert_eq!(top(Metric::Cosine), VectorId(2));
        assert_eq!(top(Metric::Dot), VectorId(2));
        assert_eq!(top(Metric::Euclidean), VectorId(2));
        assert_eq!(ix.search(&[0.0, 0.0], 3, Metric::Cosine).unwrap().len(), 3);
        ix.insert(VectorId(1), &[0.0, 1.0], json!({"a": 2}))
            .unwrap();
        assert_eq!(ix.len(), 3);
        let h = ix.search(&[0.0, 1.0], 2, Metric::Euclidean).unwrap();
        assert_eq!(h[0].id, VectorId(1));
        assert_eq!(h[0].metadata, json!({"a": 2}));
        assert_eq!(
            ix.insert(VectorId(9), &[1.0], json!(null))
                .unwrap_err()
                .code(),
            "INVALID_VECTOR_DIMENSION"
        );
        assert!(ix.search(&[1.0], 1, Metric::Dot).is_err());
    }
}
