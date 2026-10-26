//! Idempotency is decided by exact payload bytes; the 64-bit fingerprint is only an aid.
//! A constant fingerprint (test-support hook) makes every pair of payloads "collide".
mod common;
use atlas_core::{AtlasError, Config, VectorId};
use atlas_engine::Engine;
use common::full;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Barrier};

fn zero(_: &[u8]) -> u64 {
    0
}

fn tmp(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-opbytes-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn op(e: &Engine, tenant: &str, op_id: &str, md: Value) -> Result<u64, AtlasError> {
    e.insert_op(&full(tenant), op_id, VectorId(1), &[1.0; 4], md)
}

fn wal_len(d: &std::path::Path) -> u64 {
    std::fs::metadata(d.join("wal.log")).unwrap().len()
}

#[test]
fn colliding_fingerprints_with_different_payloads_conflict_live_and_after_reopen() {
    let dir = tmp("collide");
    let e = Engine::open_with_fingerprint(&dir, Config::new(4), zero).unwrap();
    let seq = op(&e, "a", "op", json!({"v": 1})).unwrap();
    let len = wal_len(&dir);
    assert_eq!(
        op(&e, "a", "op", json!({"v": 2})).unwrap_err().code(),
        "OP_ID_CONFLICT"
    );
    assert_eq!(wal_len(&dir), len, "conflict must not append");
    drop(e);
    let e = Engine::open_with_fingerprint(&dir, Config::reopen(), zero).unwrap();
    assert_eq!(
        op(&e, "a", "op", json!({"v": 2})).unwrap_err().code(),
        "OP_ID_CONFLICT"
    );
    assert_eq!(op(&e, "a", "op", json!({"v": 1})).unwrap(), seq);
    assert_eq!(wal_len(&dir), len);
}

#[test]
fn identical_retry_with_constant_fingerprint_returns_original_seq() {
    let dir = tmp("retry");
    let e = Engine::open_with_fingerprint(&dir, Config::new(4), zero).unwrap();
    let seq = op(&e, "a", "op", json!({"v": 1})).unwrap();
    assert_eq!(op(&e, "a", "op", json!({"v": 1})).unwrap(), seq);
    assert_eq!(e.len(), 1);
}

#[test]
fn concurrent_same_op_id_different_payloads_have_one_winner_under_collision() {
    for i in 0..200 {
        let dir = tmp(&format!("race{}", i % 4));
        let e = Arc::new(Engine::open_with_fingerprint(&dir, Config::new(4), zero).unwrap());
        let barrier = Arc::new(Barrier::new(4));
        let hs: Vec<_> = (0..4)
            .map(|n| {
                let (e, b) = (e.clone(), barrier.clone());
                std::thread::spawn(move || {
                    b.wait();
                    op(&e, "a", "op", json!({ "n": n }))
                })
            })
            .collect();
        let res: Vec<_> = hs.into_iter().map(|h| h.join().unwrap()).collect();
        let wins: Vec<_> = res.iter().filter_map(|r| r.as_ref().ok()).collect();
        assert_eq!(wins.len(), 1, "iteration {i}: {res:?}");
        for r in res.iter().filter(|r| r.is_err()) {
            assert_eq!(r.as_ref().unwrap_err().code(), "OP_ID_CONFLICT");
        }
        assert_eq!(e.len(), 1);
        // exactly one WAL record / seq consumed: the next plain insert follows the winner
        let next = e
            .insert(&full("a"), VectorId(9), &[1.0; 4], Value::Null)
            .unwrap();
        assert_eq!(next, *wins[0] + 1, "iteration {i}: extra WAL record");
    }
}

#[test]
fn same_op_id_in_two_tenants_is_independent_and_never_compares_foreign_bytes() {
    let dir = tmp("tenants");
    let e = Engine::open_with_fingerprint(&dir, Config::new(4), zero).unwrap();
    let a = op(&e, "a", "op", json!({"v": 1})).unwrap();
    // Tenant b: same op id, different payload: not a conflict with a's bytes.
    let b = op(&e, "b", "op", json!({"v": 2})).unwrap();
    assert_ne!(a, b);
    assert_eq!(op(&e, "a", "op", json!({"v": 1})).unwrap(), a);
    assert_eq!(op(&e, "b", "op", json!({"v": 2})).unwrap(), b);
    // b retrying a's payload is a conflict against b's own bytes, not a hit on a's entry.
    assert_eq!(
        op(&e, "b", "op", json!({"v": 1})).unwrap_err().code(),
        "OP_ID_CONFLICT"
    );
    assert_eq!(e.len(), 2);
}
