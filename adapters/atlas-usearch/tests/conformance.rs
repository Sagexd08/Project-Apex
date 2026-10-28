//! The adapter must pass the shared AnnIndex conformance suite and, on larger data, keep recall
//! against the exact FlatIndex oracle and never return an id the filter rejects.
use atlas_core::{Metric, VectorId};
use atlas_index::{AnnIndex, FlatIndex};
use atlas_usearch::UsearchIndex;
use serde_json::json;

#[test]
fn passes_shared_conformance_suite() {
    atlas_index::conformance::run(|dim| Box::new(UsearchIndex::new(dim)));
}

/// Deterministic pseudo-random vectors (splitmix-style), no extra dependency.
fn vec_for(i: u64, dim: usize) -> Vec<f32> {
    let mut z = i.wrapping_mul(0x9E3779B97F4A7C15) ^ 0xA5A5;
    (0..dim)
        .map(|_| {
            z = z.wrapping_add(0x9E3779B97F4A7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
            ((x ^ (x >> 31)) % 2001) as f32 / 1000.0 - 1.0
        })
        .collect()
}

#[test]
fn large_index_recall_and_zero_leak_vs_oracle() {
    let (n, dim) = (6000u64, 32);
    let mut ix = UsearchIndex::new(dim);
    let mut oracle = FlatIndex::new(dim);
    for i in 0..n {
        let v = vec_for(i, dim);
        ix.insert(VectorId(i), &v, json!({ "i": i })).unwrap();
        oracle.insert(VectorId(i), &v, json!({ "i": i })).unwrap();
    }
    assert!(
        ix.len() > 4096,
        "must exercise the graph path, not the exact fallback"
    );
    let allow_of = |pm: u64| move |id: VectorId| id.0 % 1000 < pm;
    let (mut hit, mut total) = (0usize, 0usize);
    for qi in 0..40u64 {
        let q = vec_for(10_000 + qi, dim);
        let want = oracle.search(&q, 10, Metric::Cosine).unwrap();
        let got = ix.search(&q, 10, Metric::Cosine).unwrap();
        hit += got
            .iter()
            .filter(|h| want.iter().any(|w| w.id == h.id))
            .count();
        total += want.len();
        for pm in [500u64, 100, 10] {
            let allow = allow_of(pm);
            let got = ix.search_filtered(&q, 10, Metric::Cosine, &allow).unwrap();
            assert!(got.iter().all(|h| allow(h.id)), "leak at {pm}/1000");
            if pm <= 40 {
                // at or below 4% selectivity the path is an exact scan
                let want = oracle
                    .search_filtered(&q, 10, Metric::Cosine, &allow)
                    .unwrap();
                assert_eq!(got, want, "exact path must equal the oracle at {pm}/1000");
            }
        }
    }
    let recall = hit as f64 / total as f64;
    assert!(recall >= 0.90, "unfiltered recall@10 {recall:.3} < 0.90");
}
