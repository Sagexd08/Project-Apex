mod filter;
pub mod vecmap;
pub use filter::{
    Filter, MAX_FIELD_CHARS, MAX_FILTER_DEPTH, MAX_FILTER_NODES, MAX_IN_VALUES, MAX_PATH_SEGMENTS,
};
pub use vecmap::{MappedFile, VectorMap};

use atlas_core::{validate_vector, AtlasError, Metric, VectorId};
use serde_json::Value;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy)]
pub struct IdHasher(u64);

impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        let mut z = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(b)).wrapping_mul(0x517C_C1B7_2722_0A95);
        }
    }

    fn write_u64(&mut self, v: u64) {
        self.0 ^= v;
    }
}

#[derive(Clone, Copy)]
pub struct IdState(u64);

impl IdState {
    fn new() -> Self {
        static SALT: AtomicU64 = AtomicU64::new(0);
        let n = SALT.fetch_add(1, Ordering::Relaxed);
        let rs = std::collections::hash_map::RandomState::new();
        let mut h = rs.build_hasher();
        h.write_u64(n);
        Self(h.finish())
    }
}

impl BuildHasher for IdState {
    type Hasher = IdHasher;
    fn build_hasher(&self) -> IdHasher {
        IdHasher(self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: VectorId,
    pub score: f32,
    pub metadata: Value,
}

pub struct FlatIndex {
    dim: usize,
    pos: OnceLock<HashMap<VectorId, usize, IdState>>,
    ids: Vec<VectorId>,
    data: Store,
    meta: Vec<Value>,
}

enum Store {
    Owned(Vec<f32>),
    Layered {
        base: VectorMap,
        base_rows: usize,
        over: HashMap<usize, Vec<f32>>,
        tail: Vec<f32>,
    },
}

impl Store {
    fn row(&self, i: usize, d: usize) -> &[f32] {
        match self {
            Store::Owned(v) => &v[i * d..(i + 1) * d],
            Store::Layered {
                base,
                base_rows,
                over,
                tail,
            } => {
                if i >= *base_rows {
                    let o = (i - base_rows) * d;
                    &tail[o..o + d]
                } else if let Some(r) = over.get(&i) {
                    r
                } else {
                    &base.as_slice()[i * d..(i + 1) * d]
                }
            }
        }
    }

    fn set_row(&mut self, i: usize, d: usize, v: &[f32]) {
        match self {
            Store::Owned(data) => data[i * d..(i + 1) * d].copy_from_slice(v),
            Store::Layered {
                base_rows,
                over,
                tail,
                ..
            } => {
                if i >= *base_rows {
                    let o = (i - *base_rows) * d;
                    tail[o..o + d].copy_from_slice(v);
                } else {
                    over.insert(i, v.to_vec());
                }
            }
        }
    }

    fn push(&mut self, v: &[f32]) {
        match self {
            Store::Owned(data) => data.extend_from_slice(v),
            Store::Layered { tail, .. } => tail.extend_from_slice(v),
        }
    }

    fn truncate_rows(&mut self, rows: usize, d: usize) {
        match self {
            Store::Owned(data) => data.truncate(rows * d),
            Store::Layered {
                base_rows,
                over,
                tail,
                ..
            } => {
                if rows >= *base_rows {
                    tail.truncate((rows - *base_rows) * d);
                } else {
                    tail.clear();
                    *base_rows = rows;
                    over.retain(|&k, _| k < rows);
                }
            }
        }
    }

    fn reserve(&mut self, floats: usize) {
        match self {
            Store::Owned(data) => data.reserve(floats),
            Store::Layered { tail, .. } => tail.reserve(floats),
        }
    }
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

const PAR_SCAN_MIN: usize = 32_768;

struct Scorer<'a> {
    metric: Metric,
    q: &'a [f32],
    qq: f32,
}

impl<'a> Scorer<'a> {
    fn new(metric: Metric, q: &'a [f32]) -> Self {
        Self {
            metric,
            q,
            qq: dot(q, q),
        }
    }

    fn score(&self, row: &[f32]) -> f32 {
        match self.metric {
            Metric::Cosine => {
                let n = (self.qq * dot(row, row)).sqrt();
                if n == 0.0 {
                    0.0
                } else {
                    dot(self.q, row) / n
                }
            }
            m => score(m, self.q, row),
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
    fn reserve(&mut self, _additional: usize) {}
    fn adopt(
        &mut self,
        _ids: &mut Vec<VectorId>,
        _vectors: VectorMap,
        _meta: &mut Vec<Value>,
    ) -> Result<bool, AtlasError> {
        Ok(false)
    }
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
            pos: OnceLock::new(),
            ids: vec![],
            data: Store::Owned(vec![]),
            meta: vec![],
        }
    }

    pub fn ids(&self) -> &[VectorId] {
        &self.ids
    }

    fn pos_map(&self) -> &HashMap<VectorId, usize, IdState> {
        self.pos.get_or_init(|| {
            let mut m = HashMap::with_capacity_and_hasher(self.ids.len(), IdState::new());
            for (i, id) in self.ids.iter().enumerate() {
                m.insert(*id, i);
            }
            m
        })
    }

    fn pos_mut(&mut self) -> &mut HashMap<VectorId, usize, IdState> {
        self.pos_map();
        self.pos.get_mut().expect("pos initialised")
    }

    fn row(&self, i: usize) -> &[f32] {
        self.data.row(i, self.dim)
    }

    pub fn get(&self, id: VectorId) -> Option<(&[f32], &Value)> {
        let &p = self.pos_map().get(&id)?;
        Some((self.row(p), &self.meta[p]))
    }
}

impl AnnIndex for FlatIndex {
    fn dim(&self) -> usize {
        self.dim
    }

    fn reserve(&mut self, additional: usize) {
        self.ids.reserve(additional);
        self.data.reserve(additional.saturating_mul(self.dim));
        self.meta.reserve(additional);
        self.pos_mut().reserve(additional);
    }

    fn adopt(
        &mut self,
        ids: &mut Vec<VectorId>,
        vectors: VectorMap,
        meta: &mut Vec<Value>,
    ) -> Result<bool, AtlasError> {
        if !self.ids.is_empty() || ids.len() != meta.len() {
            return Ok(false);
        }
        if vectors.len() != ids.len().saturating_mul(self.dim) {
            return Err(AtlasError::WalCorrupt("vector map: row count".into()));
        }
        self.ids = std::mem::take(ids);
        self.meta = std::mem::take(meta);
        self.data = Store::Layered {
            base: vectors,
            base_rows: self.ids.len(),
            over: HashMap::new(),
            tail: Vec::new(),
        };
        self.pos = OnceLock::new();
        Ok(true)
    }

    fn len(&self) -> usize {
        self.ids.len()
    }

    fn insert(&mut self, id: VectorId, v: &[f32], metadata: Value) -> Result<(), AtlasError> {
        validate_vector(self.dim, v)?;
        let d = self.dim;
        if let Some(&p) = self.pos_map().get(&id) {
            self.data.set_row(p, d, v);
            self.meta[p] = metadata;
        } else {
            let at = self.ids.len();
            self.pos_mut().insert(id, at);
            self.ids.push(id);
            self.data.push(v);
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
                let v = self.row(i).to_vec();
                (self.ids[i], v, self.meta[i].clone())
            })
            .collect()
    }

    fn remove(&mut self, id: VectorId) -> bool {
        let Some(p) = self.pos_mut().remove(&id) else {
            return false;
        };
        let last = self.ids.len() - 1;
        let d = self.dim;
        if p != last {
            let moved_row = self.data.row(last, d).to_vec();
            self.data.set_row(p, d, &moved_row);
            self.ids[p] = self.ids[last];
            let moved = self.ids[p];
            self.pos_mut().insert(moved, p);
        }
        self.ids.truncate(last);
        self.data.truncate_rows(last, d);
        self.meta.swap_remove(p);
        true
    }

    fn search(&self, q: &[f32], k: usize, metric: Metric) -> Result<Vec<Hit>, AtlasError> {
        validate_vector(self.dim, q)?;
        let n = self.ids.len();
        let threads = std::thread::available_parallelism().map_or(1, |t| t.get());
        if n < PAR_SCAN_MIN * 2 || threads < 2 || k == 0 {
            return self.search_filtered(q, k, metric, &|_| true);
        }
        let parts = threads.min(n / PAR_SCAN_MIN).max(1);
        let chunk = n.div_ceil(parts);
        let ids = &self.ids;
        let cmp = |a: &(f32, usize), b: &(f32, usize)| {
            b.0.total_cmp(&a.0).then_with(|| ids[a.1].cmp(&ids[b.1]))
        };
        let scorer = Scorer::new(metric, q);
        let mut merged: Vec<(f32, usize)> = std::thread::scope(|sc| {
            let handles: Vec<_> = (0..n)
                .step_by(chunk)
                .map(|lo| {
                    let hi = (lo + chunk).min(n);
                    let scorer = &scorer;
                    sc.spawn(move || {
                        let mut v: Vec<(f32, usize)> =
                            (lo..hi).map(|i| (scorer.score(self.row(i)), i)).collect();
                        if v.len() > k {
                            v.select_nth_unstable_by(k - 1, cmp);
                            v.truncate(k);
                        }
                        v
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().expect("scan worker"))
                .collect()
        });
        if merged.len() > k {
            merged.select_nth_unstable_by(k - 1, cmp);
            merged.truncate(k);
        }
        merged.sort_by(cmp);
        Ok(merged
            .into_iter()
            .map(|(s, i)| Hit {
                id: self.ids[i],
                score: s,
                metadata: self.meta[i].clone(),
            })
            .collect())
    }

    fn search_filtered(
        &self,
        q: &[f32],
        k: usize,
        metric: Metric,
        allow: &dyn Fn(VectorId) -> bool,
    ) -> Result<Vec<Hit>, AtlasError> {
        validate_vector(self.dim, q)?;
        let scorer = Scorer::new(metric, q);
        let mut hits: Vec<(f32, usize)> = (0..self.ids.len())
            .filter(|&i| allow(self.ids[i]))
            .map(|i| (scorer.score(self.row(i)), i))
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
pub mod hnsw;
pub use hnsw::HnswIndex;

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
    fn parallel_scan_matches_serial_scan() {
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        let mut rnd = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 40) as f32 / (1u64 << 23) as f32 - 1.0
        };
        let mut ix = FlatIndex::new(4);
        for id in 0..70_000u64 {
            let v: Vec<f32> = (0..4).map(|_| rnd()).collect();
            ix.insert(VectorId(id), &v, json!(null)).unwrap();
        }
        let dup = vec![0.25; 4];
        for id in 70_000..70_020u64 {
            ix.insert(VectorId(id), &dup, json!(null)).unwrap();
        }
        let q: Vec<f32> = (0..4).map(|_| rnd()).collect();
        for metric in [Metric::Cosine, Metric::Dot, Metric::Euclidean] {
            for k in [1usize, 10, 100, 70_100] {
                let par = ix.search(&q, k, metric).unwrap();
                let ser = ix.search_filtered(&q, k, metric, &|_| true).unwrap();
                assert_eq!(par, ser, "{metric:?} k={k}");
            }
        }
    }

    #[test]
    fn layered_store_behaves_like_owned_store_under_mutation() {
        let dim = 3usize;
        let rows: Vec<Vec<f32>> = (0..6u64)
            .map(|i| (0..dim).map(|j| (i * 10 + j as u64) as f32).collect())
            .collect();
        let path = std::env::temp_dir().join(format!("atlas-layered-{}.bin", std::process::id()));
        let bytes: Vec<u8> = rows
            .iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        std::fs::write(&path, &bytes).unwrap();
        let file = MappedFile::open(&path).unwrap();
        let mut mapped = FlatIndex::new(dim);
        let mut ids: Vec<VectorId> = (0..6u64).map(VectorId).collect();
        let mut meta = vec![json!(null); 6];
        let ok = mapped
            .adopt(
                &mut ids,
                file.floats(0, rows.len() * dim).unwrap(),
                &mut meta,
            )
            .unwrap();
        assert!(ok);
        let mut owned = FlatIndex::new(dim);
        for (i, r) in rows.iter().enumerate() {
            owned.insert(VectorId(i as u64), r, json!(null)).unwrap();
        }
        let ops: Vec<(&str, u64, f32)> = vec![
            ("put", 2, 100.0),
            ("put", 9, 200.0),
            ("del", 0, 0.0),
            ("put", 3, 300.0),
            ("del", 5, 0.0),
            ("put", 11, 400.0),
            ("del", 9, 0.0),
            ("del", 4, 0.0),
            ("put", 4, 500.0),
        ];
        for (op, id, x) in ops {
            for ix in [&mut mapped, &mut owned] {
                if op == "put" {
                    ix.insert(VectorId(id), &[x, x + 1.0, x + 2.0], json!(null))
                        .unwrap();
                } else {
                    ix.remove(VectorId(id));
                }
            }
            assert_eq!(mapped.entries(), owned.entries(), "after {op} {id}");
            for e in owned.entries() {
                assert_eq!(mapped.get(e.0).map(|g| g.0.to_vec()), Some(e.1));
            }
        }
        drop(mapped);
        drop(file);
        let _ = std::fs::remove_file(path);
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
