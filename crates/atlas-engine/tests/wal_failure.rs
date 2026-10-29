mod common;
use atlas_core::{Config, Metric, VectorId};
use atlas_engine::Engine;
use common::single_tenant_full;
use serde_json::json;
use std::sync::atomic::Ordering::SeqCst;

fn ids(eng: &Engine) -> Vec<u64> {
    let mut v: Vec<u64> = eng
        .search(&single_tenant_full(), &[1.0; 4], 100, Metric::Dot)
        .unwrap()
        .iter()
        .map(|h| h.id.0)
        .collect();
    v.sort();
    v
}

#[test]
fn flush_and_truncate_failure_enters_failed_state() {
    let dir = std::env::temp_dir().join(format!("atlas-fail2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let eng = Engine::open(&dir, Config::new(4)).unwrap();
    eng.insert(&single_tenant_full(), VectorId(1), &[1.0; 4], json!(null))
        .unwrap();
    eng.fail_next_flush_handle().store(true, SeqCst);
    eng.fail_next_truncate_handle().store(true, SeqCst);
    let e = eng
        .insert(&single_tenant_full(), VectorId(2), &[1.0; 4], json!(null))
        .unwrap_err();
    assert_eq!(e.code(), "WAL_FLUSH_FAILED");
    let e = eng
        .insert(&single_tenant_full(), VectorId(3), &[1.0; 4], json!(null))
        .unwrap_err();
    assert_eq!(e.code(), "WAL_FAILED");
    assert_eq!(ids(&eng), vec![1]);
    drop(eng);
    let eng = Engine::open(&dir, Config::new(4)).unwrap();
    assert_eq!(
        ids(&eng),
        vec![1],
        "unacknowledged record must not be visible"
    );
    eng.insert(&single_tenant_full(), VectorId(4), &[1.0; 4], json!(null))
        .unwrap();
    assert_eq!(ids(&eng), vec![1, 4]);
}
