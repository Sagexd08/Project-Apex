mod common;

use atlas_core::{Config, Metric, PermSet, VectorId};
use atlas_engine::{Engine, Filter};
use common::{full, scope};
use serde_json::json;

fn dir(n: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-filter-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn vec_of(i: u64) -> [f32; 2] {
    [(i % 17) as f32 + 1.0, (i % 5) as f32 + 0.5]
}

#[test]
fn search_where_matches_naive_filtering_and_stays_in_tenant() {
    let e = Engine::open(dir("iso"), Config::new(2)).unwrap();
    let (a, b) = (full("a"), full("b"));
    for i in 0..400u64 {
        let color = if i % 3 == 0 { "red" } else { "blue" };
        let md = json!({"c": color, "n": i});
        e.insert(&a, VectorId(i), &vec_of(i), md.clone()).unwrap();
        e.insert(&b, VectorId(10_000 + i), &vec_of(i), md).unwrap();
    }
    let spec = json!({"and": [{"eq": ["c", "red"]}, {"range": {"field": "n", "lt": 200}}]});
    let flt = Filter::from_json(&spec).unwrap();
    let q = [3.0, 2.0];
    let got = e.search_where(&a, &q, 15, Metric::Cosine, &flt).unwrap();
    let want = e
        .search_filtered(&a, &q, 15, Metric::Cosine, &|id| {
            id.0 % 3 == 0 && id.0 < 200
        })
        .unwrap();
    assert_eq!(
        got.iter().map(|h| h.id.0).collect::<Vec<_>>(),
        want.iter().map(|h| h.id.0).collect::<Vec<_>>()
    );
    assert!(!got.is_empty());
    assert!(got
        .iter()
        .all(|h| h.id.0 < 10_000 && h.metadata["c"] == "red"));
    let theirs = e.search_where(&b, &q, 15, Metric::Cosine, &flt).unwrap();
    assert!(!theirs.is_empty());
    assert!(theirs.iter().all(|h| h.id.0 >= 10_000));
}

#[test]
fn search_where_requires_search_permission_and_valid_vector() {
    let e = Engine::open(dir("perm"), Config::new(2)).unwrap();
    let flt = Filter::from_json(&json!({"eq": ["c", "red"]})).unwrap();
    let denied = scope("a", PermSet::FETCH);
    let err = e
        .search_where(&denied, &[1.0, 1.0], 3, Metric::Cosine, &flt)
        .unwrap_err();
    assert_eq!(err.code(), "FORBIDDEN");
    let ok = full("a");
    let err = e
        .search_where(&ok, &[1.0], 3, Metric::Cosine, &flt)
        .unwrap_err();
    assert_eq!(err.code(), "INVALID_VECTOR_DIMENSION");
    let empty = e
        .search_where(&ok, &[1.0, 1.0], 3, Metric::Cosine, &flt)
        .unwrap();
    assert!(empty.is_empty());
}
