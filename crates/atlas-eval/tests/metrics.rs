use atlas_core::{Metric, VectorId};
use atlas_eval::{evaluate, mrr, ndcg_at_k, percentile, recall_at_k, synthetic};
use atlas_index::{AnnIndex, FlatIndex};
use serde_json::Value;

fn ids(v: &[u64]) -> Vec<VectorId> {
    v.iter().map(|&i| VectorId(i)).collect()
}

#[test]
fn recall_hand_computed() {
    assert_eq!(
        recall_at_k(&ids(&[1, 2, 9]), &ids(&[1, 2, 3]), 3),
        2.0 / 3.0
    );
    assert_eq!(recall_at_k(&ids(&[]), &ids(&[1]), 1), 0.0);
    assert_eq!(recall_at_k(&ids(&[5]), &ids(&[]), 3), 1.0);
    assert_eq!(recall_at_k(&ids(&[1, 2, 3]), &ids(&[1, 2, 3]), 2), 1.0);
}

#[test]
fn mrr_hand_computed() {
    assert_eq!(mrr(&ids(&[9, 8, 1]), &ids(&[1])), 1.0 / 3.0);
    assert_eq!(mrr(&ids(&[1]), &ids(&[1])), 1.0);
    assert_eq!(mrr(&ids(&[9]), &ids(&[1])), 0.0);
}

#[test]
fn ndcg_hand_computed() {
    assert!((ndcg_at_k(&ids(&[1, 2]), &ids(&[1, 2]), 2) - 1.0).abs() < 1e-12);
    let dcg = 1.0 / 3f64.log2();
    let ideal = 1.0 + 1.0 / 3f64.log2();
    assert!((ndcg_at_k(&ids(&[9, 2]), &ids(&[1, 2]), 2) - dcg / ideal).abs() < 1e-12);
    assert_eq!(ndcg_at_k(&ids(&[9]), &ids(&[]), 2), 1.0);
}

#[test]
fn percentile_nearest_rank() {
    let v: Vec<f64> = (1..=100).map(|x| x as f64).collect();
    assert_eq!(percentile(&v, 0.50), 50.0);
    assert_eq!(percentile(&v, 0.95), 95.0);
    assert_eq!(percentile(&v, 0.99), 99.0);
    assert_eq!(percentile(&[], 0.5), 0.0);
}

#[test]
fn same_seed_same_numbers_and_flat_is_exact() {
    let run = |seed| {
        let d = synthetic(seed, 300, 20, 8);
        let mut ix = FlatIndex::new(8);
        for (i, v) in d.base.iter().enumerate() {
            ix.insert(VectorId(i as u64), v, Value::Null).unwrap();
        }
        evaluate(&ix, &d, Metric::Cosine, 10, seed).unwrap()
    };
    let (a, b, c) = (run(7), run(7), run(8));
    for key in ["recall_at_k", "ndcg_at_k", "mrr", "seed", "n"] {
        assert_eq!(a[key], b[key]);
    }
    assert_eq!(a["recall_at_k"], 1.0);
    assert_eq!(a["mrr"], 1.0);
    assert_ne!(a["seed"], c["seed"]);
}
