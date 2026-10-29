mod common;

use atlas_core::{Config, Metric, PermSet, VectorId};
use atlas_engine::{Engine, Filter};
use common::{full, scope};
use serde_json::json;
use std::time::{Duration, Instant};

fn dir(n: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-explain-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn seeded(name: &str) -> Engine {
    let e = Engine::open(dir(name), Config::new(2)).unwrap();
    let a = full("a");
    for i in 0..100u64 {
        let tag = if i % 10 == 0 { "rare" } else { "common" };
        e.insert(&a, VectorId(i), &[i as f32 + 1.0, 1.0], json!({"t": tag}))
            .unwrap();
    }
    e
}

#[test]
fn explain_reports_selectivity_and_hits() {
    let e = seeded("sel");
    let a = full("a");
    let f = Filter::from_json(&json!({"eq": ["t", "rare"]})).unwrap();
    let r = e
        .search_explain(&a, &[5.0, 1.0], 20, Metric::Cosine, Some(&f), None)
        .unwrap();
    assert_eq!((r.total, r.eligible), (100, 10));
    assert!((r.selectivity - 0.1).abs() < 1e-12);
    assert!(r.filtered && !r.timed_out);
    assert_eq!(r.hits.len(), 10);
    assert!(r.underfilled);
    let plain = e
        .search_explain(&a, &[5.0, 1.0], 5, Metric::Cosine, None, None)
        .unwrap();
    assert_eq!(
        (plain.total, plain.eligible, plain.hits.len()),
        (100, 100, 5)
    );
    assert!(!plain.filtered && !plain.underfilled);
}

#[test]
fn expired_deadline_returns_typed_timeout_not_error() {
    let e = seeded("dl");
    let a = full("a");
    let past = Instant::now() - Duration::from_secs(1);
    let r = e
        .search_explain(&a, &[5.0, 1.0], 5, Metric::Cosine, None, Some(past))
        .unwrap();
    assert!(r.timed_out && r.hits.is_empty());
    assert_eq!(r.total, 100);
    let future = Instant::now() + Duration::from_secs(60);
    let ok = e
        .search_explain(&a, &[5.0, 1.0], 5, Metric::Cosine, None, Some(future))
        .unwrap();
    assert!(!ok.timed_out && ok.hits.len() == 5);
}

#[test]
fn explain_enforces_permission_and_unknown_tenant_is_empty() {
    let e = seeded("perm");
    let denied = scope("a", PermSet::FETCH);
    let err = e
        .search_explain(&denied, &[1.0, 1.0], 3, Metric::Cosine, None, None)
        .unwrap_err();
    assert_eq!(err.code(), "FORBIDDEN");
    let other = full("nobody");
    let r = e
        .search_explain(&other, &[1.0, 1.0], 3, Metric::Cosine, None, None)
        .unwrap();
    assert!(r.hits.is_empty() && r.total == 0 && !r.timed_out);
}
