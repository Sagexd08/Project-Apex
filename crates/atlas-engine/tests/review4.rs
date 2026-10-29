mod common;

use atlas_core::{AtlasError, Config, Metric, VectorId};
use atlas_engine::{BoxedIndex, Engine, Hit};
use atlas_index::{AnnIndex, FlatIndex};
use common::full;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const SAFEGUARD: Duration = Duration::from_secs(30);

fn tmp(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-r4-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

struct Refuses(FlatIndex);

impl AnnIndex for Refuses {
    fn dim(&self) -> usize {
        self.0.dim()
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn insert(&mut self, id: VectorId, v: &[f32], md: Value) -> Result<(), AtlasError> {
        if id.0 == 666 {
            return Err(AtlasError::Codec("index refused".into()));
        }
        self.0.insert(id, v, md)
    }
    fn entries(&self) -> Vec<(VectorId, Vec<f32>, Value)> {
        self.0.entries()
    }
    fn remove(&mut self, id: VectorId) -> bool {
        self.0.remove(id)
    }
    fn search(&self, q: &[f32], k: usize, m: Metric) -> Result<Vec<Hit>, AtlasError> {
        self.0.search(q, k, m)
    }
}

fn open_refusing(dir: &Path, cfg: Config) -> Arc<Engine> {
    Arc::new(
        Engine::open_with_index(dir, cfg, |d| -> BoxedIndex {
            Box::new(Refuses(FlatIndex::new(d)))
        })
        .unwrap(),
    )
}

#[test]
fn f2_request_queued_behind_the_poisoning_batch_is_rejected_before_write() {
    let dir = tmp("queued");
    let e = open_refusing(&dir, Config::new(2));
    let a = full("a");
    e.insert(&a, VectorId(1), &[1.0, 0.0], json!(null)).unwrap();
    let hold = e.commit_hold_handle();
    hold.arm();
    let (e1, a1) = (e.clone(), a.clone());
    let h = std::thread::spawn(move || e1.insert(&a1, VectorId(666), &[1.0, 0.0], json!(null)));
    assert!(
        hold.wait_held(SAFEGUARD),
        "HANG/TIMEOUT (not a correctness failure): committer never reached the hold"
    );
    let (e2, a2) = (e.clone(), a.clone());
    let l = std::thread::spawn(move || {
        e2.insert_op(&a2, "late", VectorId(7), &[1.0, 0.0], json!({"k": 1}))
    });
    assert!(
        hold.wait_enqueued(3, SAFEGUARD),
        "HANG/TIMEOUT (not a correctness failure): late never reached the queue"
    );
    hold.release();
    let late = l.join().unwrap();
    assert_eq!(h.join().unwrap().unwrap_err().code(), "WAL_CORRUPT");
    assert_eq!(late.unwrap_err().code(), "WAL_CORRUPT", "queued request");
    assert_eq!(
        e.count(&a).unwrap(),
        1,
        "the rejected record was not applied"
    );
    drop(e);
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(e.count(&a).unwrap(), 2, "late must not be durable");
    let first = e
        .insert_op(&a, "late", VectorId(7), &[1.0, 0.0], json!({"k": 1}))
        .unwrap();
    assert_eq!(first, 3);
    let again = e
        .insert_op(&a, "late", VectorId(7), &[1.0, 0.0], json!({"k": 1}))
        .unwrap();
    assert_eq!(again, first);
    assert_eq!(e.count(&a).unwrap(), 3, "no duplicate execution");
}

#[test]
fn f2_record_written_in_the_poisoning_batch_is_outcome_unknown_and_retry_is_idempotent() {
    let dir = tmp("samebatch");
    let a = full("a");
    let first = Engine::open(&dir, Config::new(2)).unwrap();
    first
        .insert(&a, VectorId(1), &[1.0, 0.0], json!(null))
        .unwrap();
    drop(first);
    let mut cfg = Config::reopen();
    cfg.max_batch = 2;
    cfg.batch_window = Duration::from_secs(60);
    let e = open_refusing(&dir, cfg);
    let hold = e.commit_hold_handle();
    let (e1, a1) = (e.clone(), a.clone());
    let h = std::thread::spawn(move || e1.insert(&a1, VectorId(666), &[1.0, 0.0], json!(null)));
    assert!(
        hold.wait_enqueued(1, SAFEGUARD),
        "HANG/TIMEOUT (not a correctness failure): 666 never reached the queue"
    );
    let late = e.insert_op(&a, "late", VectorId(7), &[1.0, 0.0], json!({"k": 1}));
    assert_eq!(
        h.join().unwrap().unwrap_err().code(),
        "WAL_CORRUPT",
        "culprit"
    );
    assert_eq!(late.unwrap_err().code(), "OUTCOME_UNKNOWN");
    drop(e);
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(
        e.count(&a).unwrap(),
        3,
        "1, 666 and the written late record"
    );
    let seq = e
        .insert_op(&a, "late", VectorId(7), &[1.0, 0.0], json!({"k": 1}))
        .unwrap();
    assert_eq!(seq, 3, "original seq");
    assert_eq!(e.count(&a).unwrap(), 3, "no duplicate execution");
}

#[test]
fn f3_retries_of_a_huge_op_do_not_stall_unrelated_inserts() {
    let dir = tmp("hugeop");
    let e = Arc::new(Engine::open(&dir, Config::new(2)).unwrap());
    let a = full("a");
    let big = || Value::String("x".repeat(8 * 1024 * 1024));
    let seq = e
        .insert_op(&a, "big", VectorId(1), &[1.0, 0.0], big())
        .unwrap();
    let (e2, a2) = (e.clone(), a.clone());
    let retrier = std::thread::spawn(move || {
        for _ in 0..5 {
            let r = e2.insert_op(&a2, "big", VectorId(1), &[1.0, 0.0], big());
            assert_eq!(r.unwrap(), seq);
        }
    });
    let mut worst = Duration::ZERO;
    for id in 10..30u64 {
        let t = std::time::Instant::now();
        e.insert(&a, VectorId(id), &[1.0, 0.0], json!(null))
            .unwrap();
        worst = worst.max(t.elapsed());
    }
    retrier.join().unwrap();
    assert!(
        worst < Duration::from_secs(5),
        "worst unrelated insert {worst:?}"
    );
}

fn legacy_wal(name: &str, records: &[&str]) -> PathBuf {
    let dir = tmp(name);
    std::fs::create_dir_all(&dir).unwrap();
    atlas_storage::write_meta(&dir, 2).unwrap();
    let (wal, _) =
        atlas_storage::Wal::open(&dir.join("wal.log"), &Config::new(2), Box::new(|_, _| {}))
            .unwrap();
    for r in records {
        wal.append(r.as_bytes().to_vec()).unwrap();
    }
    dir
}

#[test]
fn f6_pin_duplicate_op_id_records_in_a_wal_both_apply_first_entry_wins() {
    let r1 = r#"{"tenant":"a","id":1,"vector":[1.0,0.0],"metadata":{"v":1},"op_id":"dup"}"#;
    let r2 = r#"{"tenant":"a","id":2,"vector":[1.0,0.0],"metadata":{"v":2},"op_id":"dup"}"#;
    let dir = legacy_wal("dupop", &[r1, r2]);
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    let a = full("a");
    assert_eq!(e.count(&a).unwrap(), 2, "both records applied");
    let same_as_first = e.insert_op(&a, "dup", VectorId(1), &[1.0, 0.0], json!({"v": 1}));
    assert_eq!(same_as_first.unwrap(), 1, "first entry kept");
    let same_as_second = e.insert_op(&a, "dup", VectorId(2), &[1.0, 0.0], json!({"v": 2}));
    assert_eq!(same_as_second.unwrap_err().code(), "OP_ID_CONFLICT");
    assert_eq!(e.count(&a).unwrap(), 2);
}

#[test]
fn f6_pin_non_canonical_wal_payload_gives_a_false_conflict_on_semantic_retry() {
    let raw = r#"{"op_id":"x","tenant":"a","id":1,"vector":[1.0,0.0],"metadata":null}"#;
    let dir = legacy_wal("noncanon", &[raw]);
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    let a = full("a");
    assert_eq!(e.count(&a).unwrap(), 1);
    let retry = e.insert_op(&a, "x", VectorId(1), &[1.0, 0.0], Value::Null);
    assert_eq!(retry.unwrap_err().code(), "OP_ID_CONFLICT");
    assert_eq!(e.count(&a).unwrap(), 1, "nothing was written");
}
