//! Caller-supplied operation ids: retries must not double-apply.
mod common;
use atlas_core::{Config, Metric, RecoverMode, VectorId};
use atlas_engine::Engine;
use common::single_tenant_full;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::Arc;

fn tmp(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-idem-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn plain(eng: &Engine, id: u64) -> u64 {
    eng.insert(&single_tenant_full(), VectorId(id), &[1.0; 4], json!(null))
        .unwrap()
}

fn op(
    eng: &Engine,
    op_id: &str,
    id: u64,
    md: serde_json::Value,
) -> Result<u64, atlas_core::AtlasError> {
    eng.insert_op(&single_tenant_full(), op_id, VectorId(id), &[1.0; 4], md)
}

#[test]
fn retry_after_reopen_does_not_double_apply_and_returns_original_seq() {
    let dir = tmp("reopen");
    let eng = Engine::open(&dir, Config::new(4)).unwrap();
    let seq = op(&eng, "op-1", 1, json!({"v": 1})).unwrap();
    drop(eng);
    let eng = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(op(&eng, "op-1", 1, json!({"v": 1})).unwrap(), seq);
    assert_eq!(
        plain(&eng, 2),
        seq + 1,
        "retry must not have appended a record"
    );
    assert_eq!(eng.len(), 2);
}

#[test]
fn same_op_id_with_different_payload_is_an_explicit_conflict() {
    let dir = tmp("conflict");
    let eng = Engine::open(&dir, Config::new(4)).unwrap();
    op(&eng, "op-1", 1, json!({"v": 1})).unwrap();
    let e = op(&eng, "op-1", 1, json!({"v": 2})).unwrap_err();
    assert_eq!(e.code(), "OP_ID_CONFLICT");
    assert_eq!(
        eng.insert_op(
            &single_tenant_full(),
            "op-1",
            VectorId(2),
            &[1.0; 4],
            json!({"v": 1})
        )
        .unwrap_err()
        .code(),
        "OP_ID_CONFLICT"
    );
    drop(eng);
    let eng = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(
        op(&eng, "op-1", 1, json!({"v": 2})).unwrap_err().code(),
        "OP_ID_CONFLICT"
    );
}

#[test]
fn op_id_length_is_bounded() {
    let dir = tmp("bound");
    let eng = Engine::open(&dir, Config::new(4)).unwrap();
    assert!(op(&eng, &"x".repeat(128), 1, json!(null)).is_ok());
    assert_eq!(
        op(&eng, &"x".repeat(129), 1, json!(null))
            .unwrap_err()
            .code(),
        "INVALID_OP_ID"
    );
    assert_eq!(
        op(&eng, "", 1, json!(null)).unwrap_err().code(),
        "INVALID_OP_ID"
    );
}

#[test]
fn concurrent_retries_of_one_op_id_apply_once() {
    let dir = tmp("conc");
    let eng = Arc::new(Engine::open(&dir, Config::new(4)).unwrap());
    let hs: Vec<_> = (0..8)
        .map(|_| {
            let eng = eng.clone();
            std::thread::spawn(move || op(&eng, "same", 5, json!({"v": 1})).unwrap())
        })
        .collect();
    let seqs: Vec<u64> = hs.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(seqs.iter().all(|s| *s == seqs[0]), "{seqs:?}");
    assert_eq!(plain(&eng, 9), seqs[0] + 1);
}

#[test]
fn retry_after_outcome_unknown_is_safe_whether_or_not_the_record_survived() {
    // 1) record discarded by Repair: the retry applies it exactly once
    let dir = tmp("unknown-absent");
    let eng = Engine::open(&dir, Config::new(4)).unwrap();
    plain(&eng, 1);
    eng.fail_next_flush_handle().store(true, SeqCst);
    eng.fail_next_truncate_handle().store(true, SeqCst);
    eng.fail_next_zero_handle().store(true, SeqCst);
    assert_eq!(
        op(&eng, "k", 2, json!({"v": 1})).unwrap_err().code(),
        "OUTCOME_UNKNOWN"
    );
    drop(eng);
    let mut cfg = Config::reopen();
    cfg.recover_mode = RecoverMode::Repair;
    let eng = Engine::open(&dir, cfg).unwrap();
    assert_eq!(eng.len(), 1);
    assert_eq!(op(&eng, "k", 2, json!({"v": 1})).unwrap(), 2);
    assert_eq!(eng.len(), 2);

    // 2) record survived (nothing could be persisted): the retry returns the original seq
    let dir = tmp("unknown-present");
    let eng = Engine::open(&dir, Config::new(4)).unwrap();
    plain(&eng, 1);
    for h in [
        eng.fail_next_flush_handle(),
        eng.fail_next_truncate_handle(),
        eng.fail_next_zero_handle(),
        eng.fail_next_marker_handle(),
    ] {
        h.store(true, SeqCst);
    }
    assert_eq!(
        op(&eng, "k", 2, json!({"v": 1})).unwrap_err().code(),
        "OUTCOME_UNKNOWN"
    );
    drop(eng);
    let eng = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(eng.len(), 2, "record replayed per protocol");
    assert_eq!(
        op(&eng, "k", 2, json!({"v": 1})).unwrap(),
        2,
        "original seq, not re-applied"
    );
    assert_eq!(plain(&eng, 3), 3);
    assert_eq!(
        eng.search(&single_tenant_full(), &[1.0; 4], 10, Metric::Dot)
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn engine_is_poisoned_after_outcome_unknown() {
    let dir = tmp("poison");
    let eng = Engine::open(&dir, Config::new(4)).unwrap();
    eng.fail_next_flush_handle().store(true, SeqCst);
    eng.fail_next_truncate_handle().store(true, SeqCst);
    eng.fail_next_zero_handle().store(true, SeqCst);
    assert_eq!(
        op(&eng, "a", 1, json!(null)).unwrap_err().code(),
        "OUTCOME_UNKNOWN"
    );
    for _ in 0..3 {
        assert_eq!(
            op(&eng, "b", 2, json!(null)).unwrap_err().code(),
            "WAL_FAILED"
        );
        assert_eq!(
            eng.insert(&single_tenant_full(), VectorId(3), &[1.0; 4], json!(null))
                .unwrap_err()
                .code(),
            "WAL_FAILED"
        );
    }
    assert!(
        eng.search(&single_tenant_full(), &[1.0; 4], 1, Metric::Dot)
            .is_ok(),
        "reads still work"
    );
}
