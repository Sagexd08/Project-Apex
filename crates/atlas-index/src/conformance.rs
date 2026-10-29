use crate::{AnnIndex, FlatIndex, Hit};
use atlas_core::{Metric, VectorId};
use serde_json::json;
use std::sync::{Arc, RwLock};

pub type Boxed = Box<dyn AnnIndex + Send + Sync>;
type Allow = fn(VectorId) -> bool;
const DIM: usize = 8;
const METRICS: [Metric; 3] = [Metric::Cosine, Metric::Dot, Metric::Euclidean];

#[derive(Clone)]
enum Op {
    Put(u64, Vec<f32>),
    Del(u64),
}

fn vecs(n: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut s = seed | 1;
    (0..n)
        .map(|_| {
            (0..DIM)
                .map(|_| {
                    s = s
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    ((s >> 40) as f32 / (1u64 << 24) as f32) - 0.5
                })
                .collect()
        })
        .collect()
}

fn apply(ix: &mut dyn AnnIndex, ops: &[Op]) {
    for op in ops {
        match op {
            Op::Put(id, v) => ix.insert(VectorId(*id), v, json!({ "id": id })).unwrap(),
            Op::Del(id) => {
                ix.remove(VectorId(*id));
            }
        }
    }
}

fn same(a: &[Hit], b: &[Hit], what: &str) {
    let ids = |h: &[Hit]| h.iter().map(|x| x.id).collect::<Vec<_>>();
    assert_eq!(ids(a), ids(b), "{what}: ids/order differ");
    for (x, y) in a.iter().zip(b) {
        assert!((x.score - y.score).abs() < 1e-5, "{what}: score differs");
        assert_eq!(x.metadata, y.metadata, "{what}: metadata differs");
    }
}

fn agree(ix: &dyn AnnIndex, oracle: &FlatIndex, what: &str) {
    assert_eq!(ix.len(), oracle.len(), "{what}: len");
    assert_eq!(ix.dim(), DIM);
    for q in vecs(5, 99) {
        for m in METRICS {
            same(
                &ix.search(&q, 10, m).unwrap(),
                &oracle.search(&q, 10, m).unwrap(),
                what,
            );
        }
    }
}

pub fn run(make: impl Fn(usize) -> Boxed) {
    let mut oracle = FlatIndex::new(DIM);
    let mut ix = make(DIM);
    assert!(ix.is_empty());
    let data = vecs(60, 1);

    let mut log: Vec<Op> = (0..60)
        .map(|i| Op::Put(i, data[i as usize].clone()))
        .collect();
    log.extend(
        vecs(10, 2)
            .into_iter()
            .enumerate()
            .map(|(i, v)| Op::Put(i as u64 * 3, v)),
    );
    log.extend((0..60).step_by(4).map(Op::Del));
    log.push(Op::Del(1000));
    apply(&mut *ix, &log);
    apply(&mut oracle, &log);
    assert!(!ix.remove(VectorId(0)), "already removed");
    assert!(ix.remove(VectorId(1)) && oracle.remove(VectorId(1)));
    agree(&*ix, &oracle, "after insert/update/delete");
    let hits = ix.search(&data[3], 60, Metric::Dot).unwrap();
    assert!(hits.iter().all(|h| h.id.0 % 4 != 0));
    assert!(ix.insert(VectorId(7), &[1.0], json!(null)).is_err());
    assert!(ix.search(&[1.0], 1, Metric::Dot).is_err());

    let mut full = log.clone();
    full.push(Op::Del(1));
    let mut rebuilt = make(DIM);
    apply(&mut *rebuilt, &full);
    agree(&*rebuilt, &oracle, "rebuild");

    let mut t = make(DIM);
    for id in [9u64, 2, 7, 4, 1] {
        t.insert(VectorId(id), &data[0], json!(null)).unwrap();
    }
    for m in METRICS {
        let got: Vec<u64> = t
            .search(&data[0], 5, m)
            .unwrap()
            .iter()
            .map(|h| h.id.0)
            .collect();
        assert_eq!(got, [1, 2, 4, 7, 9], "tie-break {m:?}");
    }

    let filters: [(&str, Allow); 3] = [
        ("even", |id| id.0.is_multiple_of(2)),
        ("sparse", |id| id.0.is_multiple_of(17)),
        ("none", |_| false),
    ];
    for (name, allow) in filters {
        for q in vecs(4, 7) {
            for m in METRICS {
                let got = ix.search_filtered(&q, 5, m, &allow).unwrap();
                assert!(got.iter().all(|h| allow(h.id)), "{name}: leaked id");
                same(
                    &got,
                    &oracle.search_filtered(&q, 5, m, &allow).unwrap(),
                    name,
                );
            }
        }
    }

    let shared = Arc::new(RwLock::new(make(DIM)));
    let writer = {
        let s = shared.clone();
        std::thread::spawn(move || {
            for (i, v) in vecs(200, 5).iter().enumerate() {
                let mut g = s.write().unwrap();
                g.insert(VectorId(i as u64 % 50), v, json!(null)).unwrap();
                if i % 7 == 0 {
                    g.remove(VectorId(i as u64 % 50));
                }
            }
        })
    };
    let readers: Vec<_> = (0..3)
        .map(|r| {
            let s = shared.clone();
            std::thread::spawn(move || {
                for q in vecs(100, 10 + r) {
                    let g = s.read().unwrap();
                    let hits = g.search(&q, 5, Metric::Cosine).unwrap();
                    assert!(hits.len() <= 5 && hits.len() <= g.len());
                    assert!(hits.windows(2).all(|w| w[0].score >= w[1].score));
                }
            })
        })
        .collect();
    writer.join().unwrap();
    for h in readers {
        h.join().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use crate::{AnnIndex, FlatIndex, Hit};
    use atlas_core::{AtlasError, Metric, VectorId};
    use serde_json::Value;

    #[test]
    fn flat_index_conforms() {
        super::run(|d| Box::new(FlatIndex::new(d)));
    }

    struct Plain(FlatIndex);
    impl AnnIndex for Plain {
        fn dim(&self) -> usize {
            self.0.dim()
        }
        fn len(&self) -> usize {
            self.0.len()
        }
        fn insert(&mut self, id: VectorId, v: &[f32], m: Value) -> Result<(), AtlasError> {
            self.0.insert(id, v, m)
        }
        fn entries(&self) -> Vec<(VectorId, Vec<f32>, Value)> {
            self.0.entries()
        }
        fn remove(&mut self, id: VectorId) -> bool {
            self.0.remove(id)
        }
        fn search(&self, q: &[f32], k: usize, m: Metric) -> Result<Vec<Hit>, AtlasError> {
            self.0.search(q, k, m)
        }
    }

    #[test]
    fn default_search_filtered_conforms() {
        super::run(|d| Box::new(Plain(FlatIndex::new(d))));
    }
}
