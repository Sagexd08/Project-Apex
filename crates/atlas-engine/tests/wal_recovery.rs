//! Slice 2: fault injection through the engine and explicit recovery.
mod common;
use atlas_core::{Config, Metric, RecoverMode, VectorId};
use atlas_engine::Engine;
use common::single_tenant_full;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::Ordering::SeqCst;

fn tmp(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-rec-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

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

fn put(eng: &Engine, id: u64) -> Result<u64, atlas_core::AtlasError> {
    eng.insert(&single_tenant_full(), VectorId(id), &[1.0; 4], json!(null))
}

#[test]
fn interrupted_write_returns_err_and_leaves_nothing() {
    let dir = tmp("partial");
    let eng = Engine::open(&dir, Config::new(4)).unwrap();
    put(&eng, 1).unwrap();
    eng.partial_write_handle().store(true, SeqCst);
    assert_eq!(put(&eng, 2).unwrap_err().code(), "WAL_FLUSH_FAILED");
    assert_eq!(ids(&eng), vec![1]);
    put(&eng, 3).unwrap();
    drop(eng);
    let eng = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(ids(&eng), vec![1, 3]);
}

fn fail_everything(dir: &std::path::Path) {
    let eng = Engine::open(dir, Config::new(4)).unwrap();
    put(&eng, 1).unwrap();
    eng.fail_next_flush_handle().store(true, SeqCst);
    eng.fail_next_truncate_handle().store(true, SeqCst);
    eng.fail_next_zero_handle().store(true, SeqCst);
    assert_eq!(put(&eng, 2).unwrap_err().code(), "OUTCOME_UNKNOWN");
    assert_eq!(put(&eng, 3).unwrap_err().code(), "WAL_FAILED");
}

#[test]
fn unprovable_wal_requires_explicit_recovery() {
    let dir = tmp("unprovable");
    fail_everything(&dir);
    let mut cfg = Config::reopen();
    let e = Engine::open(&dir, cfg.clone()).err().unwrap();
    assert_eq!(e.code(), "RECOVERY_REQUIRED");
    // normal open does not clear the state
    assert_eq!(
        Engine::open(&dir, cfg.clone()).err().unwrap().code(),
        "RECOVERY_REQUIRED"
    );
    cfg.recover_mode = RecoverMode::Repair;
    let eng = Engine::open(&dir, cfg).unwrap();
    assert_eq!(
        ids(&eng),
        vec![1],
        "failed batch discarded, acked data kept"
    );
    assert_eq!(put(&eng, 4).unwrap(), 2);
    drop(eng);
    let eng = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(ids(&eng), vec![1, 4]);
}

#[test]
fn repair_without_marker_is_a_plain_open_and_corrupt_marker_is_refused() {
    let dir = tmp("repair-noop");
    let eng = Engine::open(&dir, Config::new(4)).unwrap();
    put(&eng, 1).unwrap();
    drop(eng);
    let mut cfg = Config::reopen();
    cfg.recover_mode = RecoverMode::Repair;
    assert_eq!(ids(&Engine::open(&dir, cfg.clone()).unwrap()), vec![1]);

    let dir = tmp("repair-bad-marker");
    fail_everything(&dir);
    std::fs::write(dir.join("recovery.required"), b"garbage").unwrap();
    assert_eq!(Engine::open(&dir, cfg).err().unwrap().code(), "WAL_CORRUPT");
}

#[test]
fn same_id_updates_replay_in_seq_order_and_nothing_applies_twice() {
    let dir = tmp("semantics");
    let eng = Engine::open(&dir, Config::new(4)).unwrap();
    eng.insert(
        &single_tenant_full(),
        VectorId(1),
        &[1.0; 4],
        json!({"v": 1}),
    )
    .unwrap();
    eng.insert(
        &single_tenant_full(),
        VectorId(2),
        &[1.0; 4],
        json!({"v": 1}),
    )
    .unwrap();
    eng.insert(
        &single_tenant_full(),
        VectorId(1),
        &[1.0; 4],
        json!({"v": 2}),
    )
    .unwrap();
    drop(eng);
    for _ in 0..2 {
        let eng = Engine::open(&dir, Config::reopen()).unwrap();
        assert_eq!(eng.len(), 2);
        let hit = |id| {
            eng.search(&single_tenant_full(), &[1.0; 4], 10, Metric::Dot)
                .unwrap()
                .into_iter()
                .find(|h| h.id == VectorId(id))
                .unwrap()
                .metadata
        };
        assert_eq!(hit(1), json!({"v": 2}));
        assert_eq!(hit(2), json!({"v": 1}));
    }
    let eng = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(
        put(&eng, 9).unwrap(),
        4,
        "seq continues, no replayed op re-logged"
    );
}

fn hook(flag: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    flag.store(true, SeqCst);
}

#[test]
fn marker_is_written_before_destructive_rollback_steps() {
    // Process "dies" right after the marker is written, before truncate/zero: the batch
    // bytes are still in the file, so reopen must refuse instead of replaying them.
    let dir = tmp("marker-first");
    let eng = Engine::open(&dir, Config::new(4)).unwrap();
    put(&eng, 1).unwrap();
    hook(eng.fail_next_flush_handle());
    hook(eng.die_after_marker_handle());
    assert_eq!(put(&eng, 2).unwrap_err().code(), "OUTCOME_UNKNOWN");
    drop(eng);
    let e = Engine::open(&dir, Config::reopen()).err().unwrap();
    assert_eq!(e.code(), "RECOVERY_REQUIRED");
    let mut cfg = Config::reopen();
    cfg.recover_mode = RecoverMode::Repair;
    assert_eq!(ids(&Engine::open(&dir, cfg).unwrap()), vec![1]);
}

/// (truncate fails, zeroing fails, marker write fails) -> expected reopen outcome.
#[test]
fn rollback_failure_matrix_has_a_deterministic_reopen_outcome() {
    for t in [false, true] {
        for z in [false, true] {
            for m in [false, true] {
                let dir = tmp(&format!("matrix-{t}-{z}-{m}"));
                let eng = Engine::open(&dir, Config::new(4)).unwrap();
                put(&eng, 1).unwrap();
                hook(eng.fail_next_flush_handle());
                if t {
                    hook(eng.fail_next_truncate_handle());
                }
                if z {
                    hook(eng.fail_next_zero_handle());
                }
                if m {
                    hook(eng.fail_next_marker_handle());
                }
                let want = if t && z {
                    "OUTCOME_UNKNOWN"
                } else {
                    "WAL_FLUSH_FAILED"
                };
                assert_eq!(put(&eng, 2).unwrap_err().code(), want, "t={t} z={z} m={m}");
                drop(eng);
                let reopened = Engine::open(&dir, Config::reopen());
                let label = format!("t={t} z={z} m={m}");
                match (t, z, m) {
                    (true, true, false) => {
                        assert_eq!(
                            reopened.err().unwrap().code(),
                            "RECOVERY_REQUIRED",
                            "{label}"
                        );
                    }
                    // documented residual: nothing could be persisted, the complete valid
                    // batch is replayed; the writer saw Err = outcome unknown.
                    (true, true, true) => {
                        assert_eq!(ids(&reopened.unwrap()), vec![1, 2], "{label}");
                    }
                    _ => assert_eq!(ids(&reopened.unwrap()), vec![1], "{label}"),
                }
            }
        }
    }
}

#[test]
fn slow_flush_hook_delays_the_ack() {
    let dir = tmp("slow");
    let eng = Engine::open(&dir, Config::new(4)).unwrap();
    eng.flush_delay_handle().store(200, SeqCst);
    let t = std::time::Instant::now();
    put(&eng, 1).unwrap();
    assert!(t.elapsed() >= std::time::Duration::from_millis(200));
}
