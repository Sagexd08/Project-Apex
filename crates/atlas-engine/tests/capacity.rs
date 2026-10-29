mod common;
use atlas_core::{AtlasError, Config, VectorId};
use atlas_engine::Engine;
use common::{full, single_tenant_full};
use serde_json::json;
use std::path::PathBuf;

fn tmp(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-cap-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn op(e: &Engine, op_id: &str, id: u64) -> Result<u64, AtlasError> {
    e.insert_op(
        &single_tenant_full(),
        op_id,
        VectorId(id),
        &[1.0; 2],
        json!(null),
    )
}

fn is_cap(r: Result<u64, AtlasError>) -> bool {
    matches!(r, Err(AtlasError::CapacityExceeded(_)))
}

#[test]
fn op_map_full_refuses_new_op_ids_but_known_retries_still_succeed() {
    let mut cfg = Config::new(2);
    cfg.max_ops = 2;
    let e = Engine::open(tmp("ops"), cfg).unwrap();
    let s1 = op(&e, "a", 1).unwrap();
    op(&e, "b", 2).unwrap();
    assert!(is_cap(op(&e, "c", 3)));
    assert_eq!(
        op(&e, "a", 1).unwrap(),
        s1,
        "known op id retries at capacity"
    );
    assert_eq!(
        e.count(&single_tenant_full()).unwrap(),
        2,
        "refused write left no trace"
    );
}

#[test]
fn op_map_limit_applies_after_reopen_but_replay_admits_existing() {
    let dir = tmp("reopen");
    let mut cfg = Config::new(2);
    cfg.max_ops = 3;
    let e = Engine::open(&dir, cfg).unwrap();
    for (i, n) in ["a", "b", "c"].iter().enumerate() {
        op(&e, n, i as u64).unwrap();
    }
    drop(e);
    let mut cfg = Config::reopen();
    cfg.max_ops = 1;
    let e = Engine::open(&dir, cfg).unwrap();
    assert!(op(&e, "a", 0).is_ok());
    assert!(is_cap(op(&e, "d", 9)));
}

#[test]
fn tenant_partition_full_refuses_upserts_but_allows_delete_and_other_tenants() {
    let mut cfg = Config::new(2);
    cfg.max_tenant_vectors = 2;
    let e = Engine::open(tmp("tenant"), cfg).unwrap();
    let (a, b) = (full("a"), full("b"));
    e.insert(&a, VectorId(1), &[1.0; 2], json!(null)).unwrap();
    e.insert(&a, VectorId(2), &[1.0; 2], json!(null)).unwrap();
    let r = e.insert(&a, VectorId(3), &[1.0; 2], json!(null));
    assert!(matches!(r, Err(AtlasError::CapacityExceeded(_))), "{r:?}");
    assert!(e
        .insert_op(&a, "x", VectorId(3), &[1.0; 2], json!(null))
        .is_err());
    e.insert(&b, VectorId(1), &[1.0; 2], json!(null)).unwrap();
    assert!(e.delete(&a, VectorId(1)).unwrap());
    e.insert(&a, VectorId(3), &[1.0; 2], json!(null)).unwrap();
}

#[test]
fn capacity_error_has_stable_code() {
    assert_eq!(
        AtlasError::CapacityExceeded("x").code(),
        "CAPACITY_EXCEEDED"
    );
}
