mod filter;
pub use filter::{
    Filter, MAX_FIELD_CHARS, MAX_FILTER_DEPTH, MAX_FILTER_NODES, MAX_IN_VALUES, MAX_PATH_SEGMENTS,
};

use atlas_core::{validate_vector, AtlasError, Metric, VectorId};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: VectorId,
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

pub trait AnnIndex {
    fn dim(&self) -> usize;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn insert(&mut self, id: VectorId, v: &[f32], metadata: Value) -> Result<(), AtlasError>;
    fn entries(&self) -> Vec<(VectorId, Vec<f32>, Value)>;
    fn matching_ids(&self, filter: &Filter) -> Vec<VectorId> {
        self.entries()
            .into_iter()
            .filter(|(_, _, m)| filter.matches(m))
            .map(|(id, _, _)| id)
            .collect()
    }
    fn remove(&mut self, id: VectorId) -> bool;
    fn search(&self, q: &[f32], k: usize, metric: Metric) -> Result<Vec<Hit>, AtlasError>;
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

    pub fn ids(&self) -> &[VectorId] {
        &self.ids
    }

    pub fn get(&self, id: VectorId) -> Option<(&[f32], &Value)> {
        let &p = self.pos.get(&id)?;
        Some((&self.data[p * self.dim..(p + 1) * self.dim], &self.meta[p]))
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

    fn matching_ids(&self, filter: &Filter) -> Vec<VectorId> {
        self.ids
            .iter()
            .zip(&self.meta)
            .filter(|(_, m)| filter.matches(m))
            .map(|(id, _)| *id)
            .collect()
    }

    fn entries(&self) -> Vec<(VectorId, Vec<f32>, Value)> {
        (0..self.ids.len())
            .map(|i| {
                let v = self.data[i * self.dim..(i + 1) * self.dim].to_vec();
                (self.ids[i], v, self.meta[i].clone())
            })
            .collect()
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
        let ids = &self.ids;
        let cmp = |a: &(f32, usize), b: &(f32, usize)| {
            b.0.total_cmp(&a.0).then_with(|| ids[a.1].cmp(&ids[b.1]))
        };
        if k > 0 && hits.len() > k {
            hits.select_nth_unstable_by(k - 1, cmp);
        }
        hits.truncate(k);
        hits.sort_by(cmp);
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

    fn oracle(
        ix: &FlatIndex,
        q: &[f32],
        k: usize,
        m: Metric,
        allow: &dyn Fn(VectorId) -> bool,
    ) -> Vec<VectorId> {
        let mut all: Vec<(f32, VectorId)> = ix
            .entries()
            .into_iter()
            .filter(|(id, _, _)| allow(*id))
            .map(|(id, v, _)| (score(m, q, &v), id))
            .collect();
        all.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        all.into_iter().take(k).map(|(_, id)| id).collect()
    }

    #[test]
    fn filtered_top_k_matches_oracle_at_every_selectivity() {
        let mut s = 0x2545_F491_4F6C_DD1Du64;
        let mut rnd = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 40) as f32 / (1u64 << 23) as f32 - 1.0
        };
        let mut ix = FlatIndex::new(8);
        for id in 0..3000u64 {
            let v: Vec<f32> = (0..8).map(|_| rnd()).collect();
            ix.insert(VectorId(id), &v, json!(null)).unwrap();
        }
        let dup = vec![0.5; 8];
        for id in 3000..3010u64 {
            ix.insert(VectorId(id), &dup, json!(null)).unwrap();
        }
        let q: Vec<f32> = (0..8).map(|_| rnd()).collect();
        for modulus in [1u64, 10, 100, 1000] {
            let allow = |id: VectorId| id.0.is_multiple_of(modulus);
            for metric in [Metric::Cosine, Metric::Dot, Metric::Euclidean] {
                for k in [0usize, 1, 10, 5000] {
                    let got: Vec<VectorId> = ix
                        .search_filtered(&q, k, metric, &allow)
                        .unwrap()
                        .into_iter()
                        .map(|h| h.id)
                        .collect();
                    assert_eq!(got, oracle(&ix, &q, k, metric, &allow), "1/{modulus} k={k}");
                    assert!(
                        got.iter().all(|id| allow(*id)),
                        "result outside allowed set"
                    );
                }
            }
        }
        let none = |_: VectorId| false;
        assert!(ix
            .search_filtered(&q, 10, Metric::Cosine, &none)
            .unwrap()
            .is_empty());
    }

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
