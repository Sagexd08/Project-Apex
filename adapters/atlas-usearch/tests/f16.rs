//! S14 acceptance tests for the EXPERIMENTAL F16 traversal graph (default stays F32).
//! Exact brute force (FlatIndex) is the ground truth. Synthetic data: not a substitute for the
//! GloVe/NYTimes/SIFT/Fashion runs in benchmarks/h2h.
use atlas_core::{
    AuthContext, Config, EffectiveScope, KeyScope, Metric, PermSet, Principal, TenantId, VectorId,
};
use atlas_engine::Engine;
use atlas_index::{AnnIndex, FlatIndex};
use atlas_usearch::UsearchIndex;
use serde_json::{json, Value};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

const DIM: usize = 32;
const N: u64 = 30_000;
const NQ: u64 = 200;
const K: usize = 10;

fn vec_for(i: u64) -> Vec<f32> {
    let mut z = i.wrapping_mul(0x9E3779B97F4A7C15) ^ 0x5A5A;
    (0..DIM)
        .map(|_| {
            z = z.wrapping_add(0x9E3779B97F4A7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
            ((x ^ (x >> 31)) % 2001) as f32 / 1000.0 - 1.0
        })
        .collect()
}

fn scope(tenant: &str) -> EffectiveScope {
    let t = TenantId::parse(tenant).unwrap();
    AuthContext::for_test(
        Principal { tenant: t.clone(), perms: PermSet::ALL },
        KeyScope { tenant: t, allowed: PermSet::ALL },
    )
    .effective()
}

fn recall(found: &[u64], truth: &[u64]) -> f64 {
    found.iter().filter(|f| truth.contains(f)).count() as f64 / truth.len().max(1) as f64
}

fn build(f16: bool) -> UsearchIndex {
    let mut idx = UsearchIndex::with_metric(DIM, Metric::Cosine);
    if f16 {
        idx = idx.with_f16_graph();
    }
    for i in 0..N {
        idx.insert(VectorId(i), &vec_for(i), Value::Null).unwrap();
    }
    idx
}

fn flat() -> FlatIndex {
    let mut f = FlatIndex::new(DIM);
    for i in 0..N {
        f.insert(VectorId(i), &vec_for(i), Value::Null).unwrap();
    }
    f
}

fn ids(h: Vec<atlas_index::Hit>) -> Vec<u64> {
    h.into_iter().map(|x| x.id.0).collect()
}

#[test]
fn f16_final_recall_meets_floor_and_tracks_f32() {
    let exact = flat();
    let (f32i, f16i) = (build(false), build(true));
    let (mut r32, mut r16) = (0.0, 0.0);
    for q in 0..NQ {
        let qv = vec_for(1_000_000 + q);
        let truth = ids(exact.search(&qv, K, Metric::Cosine).unwrap());
        r32 += recall(&ids(f32i.search(&qv, K, Metric::Cosine).unwrap()), &truth);
        r16 += recall(&ids(f16i.search(&qv, K, Metric::Cosine).unwrap()), &truth);
    }
    let (r32, r16) = (r32 / NQ as f64, r16 / NQ as f64);
    eprintln!("final recall@10 f32={r32:.4} f16={r16:.4}");
    assert!(r16 >= 0.95, "f16 final recall {r16}");
    assert!(r32 - r16 <= 0.01, "f16 loses >0.01 vs f32: {r32} vs {r16}");
}

/// Graph-level recall BEFORE exact rescoring: rescoring cannot recover neighbours the walk missed.
#[test]
fn f16_graph_level_recall_reported_and_bounded() {
    let exact = flat();
    let mk = |q: ScalarKind| {
        let g = Index::new(&IndexOptions {
            dimensions: DIM,
            metric: MetricKind::Cos,
            quantization: q,
            connectivity: 32,
            expansion_add: 200,
            expansion_search: 512,
            multi: false,
        })
        .unwrap();
        g.reserve(N as usize).unwrap();
        for i in 0..N {
            g.add(i, &vec_for(i)).unwrap();
        }
        g
    };
    let (g32, g16) = (mk(ScalarKind::F32), mk(ScalarKind::F16));
    let (mut r32, mut r16) = (0.0, 0.0);
    for q in 0..NQ {
        let qv = vec_for(1_000_000 + q);
        let truth = ids(exact.search(&qv, K, Metric::Cosine).unwrap());
        r32 += recall(&g32.search(&qv, K).unwrap().keys, &truth);
        r16 += recall(&g16.search(&qv, K).unwrap().keys, &truth);
    }
    let (r32, r16) = (r32 / NQ as f64, r16 / NQ as f64);
    eprintln!("graph-level recall@10 (no rescore) f32={r32:.4} f16={r16:.4}");
    assert!(r16 >= 0.95, "f16 graph recall {r16}");
}

#[test]
fn f16_filtered_search_has_no_leaks_and_good_recall_at_all_selectivities() {
    let exact = flat();
    let f16i = build(true);
    for (label, keep) in [("10%", 10u64), ("5%", 20), ("1%", 100), ("0.1%", 1000)] {
        let allow = move |id: VectorId| id.0 % keep == 0;
        let (mut rec, mut leaks) = (0.0, 0usize);
        for q in 0..NQ {
            let qv = vec_for(2_000_000 + q);
            let truth = ids(exact.search_filtered(&qv, K, Metric::Cosine, &allow).unwrap());
            let got = ids(f16i.search_filtered(&qv, K, Metric::Cosine, &allow).unwrap());
            leaks += got.iter().filter(|&&x| x % keep != 0).count();
            rec += recall(&got, &truth);
        }
        let rec = rec / NQ as f64;
        eprintln!("f16 filtered sel={label} recall@10={rec:.4} leaks={leaks}");
        assert_eq!(leaks, 0, "leak at {label}");
        assert!(rec >= 0.95, "recall {rec} at {label}");
    }
}

#[test]
fn f16_engine_isolates_tenants_and_survives_checkpoint_wal_replay_and_reopen() {
    let dir = std::env::temp_dir().join(format!("atlas-usearch-f16-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (a, b) = (scope("tenant-a"), scope("tenant-b"));
    let q = vec_for(777);
    let n = 6000u64;

    let eng = Engine::open_with_index(&dir, Config::new(DIM), UsearchIndex::factory_f16()).unwrap();
    for i in 0..n / 2 {
        eng.insert(&a, VectorId(i), &vec_for(i), json!({ "i": i })).unwrap();
    }
    eng.checkpoint().unwrap();
    for i in n / 2..n {
        eng.insert(&a, VectorId(i), &vec_for(i), json!({ "i": i })).unwrap();
    }
    eng.insert(&b, VectorId(1), &vec_for(777), json!({ "b": true })).unwrap();
    assert_eq!(eng.search(&a, &q, 5, Metric::Cosine).unwrap()[0].id, VectorId(777));
    let tb = eng.search(&b, &q, 5, Metric::Cosine).unwrap();
    assert_eq!(tb.len(), 1, "tenant b sees only its own vector");
    drop(eng);

    // Reopen = segment load + WAL tail replay + index reconstruction through the F16 factory.
    let eng = Engine::open_with_index(&dir, Config::reopen(), UsearchIndex::factory_f16()).unwrap();
    assert_eq!(eng.count(&a).unwrap(), n as usize);
    assert_eq!(eng.search(&a, &q, 5, Metric::Cosine).unwrap()[0].id, VectorId(777));
    let hits = eng.search(&b, &q, 5, Metric::Cosine).unwrap();
    assert!(hits.len() == 1 && hits[0].metadata == json!({ "b": true }), "no cross-tenant leakage");
    let f = eng.search_filtered(&a, &q, 10, Metric::Cosine, &|id| id.0 % 50 == 0).unwrap();
    assert!(!f.is_empty() && f.iter().all(|h| h.id.0 % 50 == 0));
    let _ = std::fs::remove_dir_all(&dir);
}
