mod common;
use atlas_core::{Config, Durability, Metric, VectorId};
use atlas_engine::Engine;
use common::single_tenant_full;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

const DIM: usize = 8;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-eng-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

struct Lcg(u64);
impl Lcg {
    fn vec(&mut self) -> Vec<f32> {
        (0..DIM)
            .map(|_| {
                self.0 = self
                    .0
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                ((self.0 >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
            })
            .collect()
    }
}

fn reference(data: &[(u64, Vec<f32>)], q: &[f32], k: usize, m: Metric) -> Vec<u64> {
    let f = |v: &[f32]| -> f64 {
        let (mut d, mut na, mut nb, mut sq) = (0f64, 0f64, 0f64, 0f64);
        for (a, b) in q.iter().zip(v) {
            let (a, b) = (*a as f64, *b as f64);
            d += a * b;
            na += a * a;
            nb += b * b;
            sq += (a - b) * (a - b);
        }
        match m {
            Metric::Dot => d,
            Metric::Cosine => d / (na.sqrt() * nb.sqrt()),
            Metric::Euclidean => -sq.sqrt(),
        }
    };
    let mut s: Vec<_> = data.iter().map(|(id, v)| (f(v), *id)).collect();
    s.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    s.into_iter().take(k).map(|x| x.1).collect()
}

fn fill(eng: &Engine, n: u64) -> Vec<(u64, Vec<f32>)> {
    let mut rng = Lcg(42);
    (0..n)
        .map(|id| {
            let v = rng.vec();
            eng.insert(&single_tenant_full(), VectorId(id), &v, json!({ "i": id }))
                .unwrap();
            (id, v)
        })
        .collect()
}

const METRICS: [Metric; 3] = [Metric::Cosine, Metric::Dot, Metric::Euclidean];

fn ids(eng: &Engine, q: &[f32], m: Metric) -> Vec<u64> {
    eng.search(&single_tenant_full(), q, 5, m)
        .unwrap()
        .iter()
        .map(|h| h.id.0)
        .collect()
}

#[test]
fn search_matches_bruteforce_and_survives_reopen() {
    let dir = tmp("brute");
    let eng = Engine::open(&dir, Config::new(DIM)).unwrap();
    let data = fill(&eng, 200);
    let mut rng = Lcg(7);
    let queries: Vec<_> = (0..10).map(|_| rng.vec()).collect();
    let before: Vec<_> = queries
        .iter()
        .flat_map(|q| {
            METRICS.map(|m| {
                (
                    ids(&eng, q, m),
                    eng.search(&single_tenant_full(), q, 5, m).unwrap(),
                )
            })
        })
        .collect();
    for q in &queries {
        for m in METRICS {
            assert_eq!(ids(&eng, q, m), reference(&data, q, 5, m), "{m:?}");
        }
    }
    drop(eng);
    let eng = Engine::open(&dir, Config::new(DIM)).unwrap();
    assert_eq!(eng.len(), 200);
    let after: Vec<_> = queries
        .iter()
        .flat_map(|q| {
            METRICS.map(|m| {
                (
                    ids(&eng, q, m),
                    eng.search(&single_tenant_full(), q, 5, m).unwrap(),
                )
            })
        })
        .collect();
    assert_eq!(before, after);
    assert_eq!(after[0].1[0].metadata, json!({ "i": after[0].1[0].id.0 }));
}

#[test]
fn dimension_mismatch_is_rejected_and_not_logged() {
    let dir = tmp("dim");
    let eng = Engine::open(&dir, Config::new(DIM)).unwrap();
    let e = eng
        .insert(&single_tenant_full(), VectorId(1), &[1.0], json!(null))
        .unwrap_err();
    assert_eq!(e.code(), "INVALID_VECTOR_DIMENSION");
    assert_eq!(
        eng.search(&single_tenant_full(), &[1.0], 1, Metric::Dot)
            .unwrap_err()
            .code(),
        "INVALID_VECTOR_DIMENSION"
    );
    assert!(eng.is_empty());
    drop(eng);
    assert!(Engine::open(&dir, Config::new(DIM)).unwrap().is_empty());
    // reopening with a different dim is refused as a config error
    let eng = Engine::open(&dir, Config::new(DIM)).unwrap();
    eng.insert(&single_tenant_full(), VectorId(1), &[0.5; DIM], json!(null))
        .unwrap();
    drop(eng);
    assert_eq!(
        Engine::open(&dir, Config::new(3)).err().unwrap().code(),
        "DIMENSION_MISMATCH"
    );
}

#[test]
fn concurrent_group_commit_inserts_all_survive_reopen() {
    let dir = tmp("conc");
    let eng = Arc::new(Engine::open(&dir, Config::new(DIM)).unwrap());
    let hs: Vec<_> = (0..8u64)
        .map(|t| {
            let eng = eng.clone();
            std::thread::spawn(move || {
                for i in 0..25u64 {
                    eng.insert(
                        &single_tenant_full(),
                        VectorId(t * 100 + i),
                        &[t as f32; DIM],
                        json!(null),
                    )
                    .unwrap();
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    drop(Arc::into_inner(eng).unwrap());
    let eng = Engine::open(&dir, Config::new(DIM)).unwrap();
    assert_eq!(eng.len(), 200);
    let got: std::collections::HashSet<u64> = eng
        .search(&single_tenant_full(), &[1.0; DIM], 200, Metric::Dot)
        .unwrap()
        .iter()
        .map(|h| h.id.0)
        .collect();
    assert!((0..8u64).all(|t| (0..25u64).all(|i| got.contains(&(t * 100 + i)))));
}

#[test]
fn failed_flush_returns_err_and_record_is_absent_after_reopen() {
    for durability in [Durability::GroupCommit, Durability::SyncPerCommit] {
        let dir = tmp(&format!("fail-{durability:?}"));
        let mut cfg = Config::new(DIM);
        cfg.durability = durability;
        let eng = Engine::open(&dir, cfg.clone()).unwrap();
        eng.insert(&single_tenant_full(), VectorId(1), &[1.0; DIM], json!(null))
            .unwrap();
        eng.fail_next_flush_handle().store(true, Ordering::SeqCst);
        let e = eng
            .insert(&single_tenant_full(), VectorId(2), &[2.0; DIM], json!(null))
            .unwrap_err();
        assert_eq!(e.code(), "WAL_FLUSH_FAILED");
        assert_eq!(eng.len(), 1, "failed commit must not be applied");
        eng.insert(&single_tenant_full(), VectorId(3), &[3.0; DIM], json!(null))
            .unwrap();
        drop(eng);
        let eng = Engine::open(&dir, cfg).unwrap();
        let mut got: Vec<u64> = eng
            .search(&single_tenant_full(), &[1.0; DIM], 10, Metric::Dot)
            .unwrap()
            .iter()
            .map(|h| h.id.0)
            .collect();
        got.sort();
        assert_eq!(got, vec![1, 3]);
    }
}

#[test]
fn custom_index_is_rebuilt_from_wal_and_filters() {
    use atlas_engine::BoxedIndex;
    use atlas_index::FlatIndex;
    let dir = tmp("custom-index");
    let mk = |d| Box::new(FlatIndex::new(d)) as BoxedIndex;
    {
        let e = Engine::open_with_index(&dir, Config::new(2), mk).unwrap();
        for i in 0..4u64 {
            e.insert(
                &single_tenant_full(),
                VectorId(i),
                &[i as f32, 1.0],
                json!(null),
            )
            .unwrap();
        }
    }
    let e = Engine::open_with_index(&dir, Config::new(2), mk).unwrap();
    assert_eq!(e.len(), 4);
    let allow = |id: VectorId| id.0.is_multiple_of(2);
    let hits = e
        .search_filtered(
            &single_tenant_full(),
            &[1.0, 1.0],
            5,
            Metric::Cosine,
            &allow,
        )
        .unwrap();
    assert!(hits.len() == 2 && hits.iter().all(|h| allow(h.id)));
}
