mod common;

use atlas_core::{Config, Metric, VectorId};
use atlas_engine::Engine;
use common::full;
use serde_json::json;
use std::path::{Path, PathBuf};

fn dir(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-ckpt-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn v(i: u64) -> [f32; 2] {
    [(i % 13) as f32 + 1.0, (i % 7) as f32 + 0.5]
}

fn wal_len(d: &Path) -> u64 {
    std::fs::metadata(d.join("wal.log")).unwrap().len()
}

fn segment_files(d: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(d)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("seg-"))
        .collect();
    v.sort();
    v
}

fn snapshot_of(e: &Engine, t: &str) -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    let s = full(t);
    let mut out = Vec::new();
    let mut sink = |it: atlas_engine::Item| {
        out.push((it.id.0, it.vector, it.metadata));
        Ok(())
    };
    e.export(&s, &mut sink).unwrap();
    out.sort_by_key(|x| x.0);
    out
}

fn crc32(b: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &x in b {
        c ^= x as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

#[test]
fn state_survives_checkpoint_more_writes_and_reopen_and_the_wal_shrinks() {
    let d = dir("state");
    let (a, b) = (full("a"), full("b"));
    let (before, expect_a, expect_b);
    {
        let e = Engine::open(&d, Config::new(2)).unwrap();
        for i in 0..200u64 {
            e.insert(&a, VectorId(i), &v(i), json!({"n": i})).unwrap();
            e.insert(&b, VectorId(i), &v(i + 1), json!({"t": "b"}))
                .unwrap();
        }
        for i in (0..200u64).step_by(5) {
            e.delete(&a, VectorId(i)).unwrap();
        }
        e.insert(&a, VectorId(1), &v(99), json!({"n": "updated"}))
            .unwrap();
        before = wal_len(&d);
        let seq = e.checkpoint().unwrap();
        assert_eq!(seq, 441);
        assert_eq!(segment_files(&d).len(), 1);
        for i in 200..230u64 {
            e.insert(&a, VectorId(i), &v(i), json!({"n": i})).unwrap();
        }
        e.delete(&b, VectorId(3)).unwrap();
        expect_a = snapshot_of(&e, "a");
        expect_b = snapshot_of(&e, "b");
    }
    let e = Engine::open(&d, Config::new(2)).unwrap();
    assert_eq!(snapshot_of(&e, "a"), expect_a);
    assert_eq!(snapshot_of(&e, "b"), expect_b);
    assert!(
        wal_len(&d) < before,
        "the log must be shorter after the cut"
    );
    let hdr = std::fs::read(d.join("wal.log")).unwrap();
    assert_eq!(&hdr[8..12], &3u32.to_le_bytes());
    e.insert(&a, VectorId(1000), &v(1), json!({})).unwrap();
    drop(e);
    let e = Engine::open(&d, Config::new(2)).unwrap();
    assert_eq!(e.count(&a).unwrap(), expect_a.len() + 1);
    assert!(!e.search(&a, &v(1), 3, Metric::Cosine).unwrap().is_empty());
}

#[test]
fn idempotent_retries_still_work_after_the_log_prefix_is_gone() {
    let d = dir("ops");
    let a = full("a");
    let first;
    {
        let e = Engine::open(&d, Config::new(2)).unwrap();
        first = e
            .insert_op(&a, "op-1", VectorId(1), &v(1), json!({"k": 1}))
            .unwrap();
        for i in 2..30u64 {
            e.insert(&a, VectorId(i), &v(i), json!({})).unwrap();
        }
        e.checkpoint().unwrap();
    }
    for _ in 0..2 {
        let e = Engine::open(&d, Config::new(2)).unwrap();
        let again = e
            .insert_op(&a, "op-1", VectorId(1), &v(1), json!({"k": 1}))
            .unwrap();
        assert_eq!(again, first, "retry must return the original sequence");
        let err = e
            .insert_op(&a, "op-1", VectorId(1), &v(2), json!({"k": 2}))
            .unwrap_err();
        assert_eq!(err.code(), "OP_ID_CONFLICT");
        assert_eq!(e.count(&a).unwrap(), 29);
    }
}

#[test]
fn checkpoint_on_an_empty_engine_writes_nothing_and_repeats_keep_one_segment() {
    let d = dir("empty");
    let a = full("a");
    let e = Engine::open(&d, Config::new(2)).unwrap();
    assert_eq!(e.checkpoint().unwrap(), 0);
    assert!(segment_files(&d).is_empty());
    e.insert(&a, VectorId(1), &v(1), json!({})).unwrap();
    let s1 = e.checkpoint().unwrap();
    e.insert(&a, VectorId(2), &v(2), json!({})).unwrap();
    let s2 = e.checkpoint().unwrap();
    assert!(s2 > s1);
    assert_eq!(segment_files(&d).len(), 1);
}

#[test]
fn damaged_or_missing_pieces_fail_closed() {
    let d = dir("fail");
    let a = full("a");
    {
        let e = Engine::open(&d, Config::new(2)).unwrap();
        for i in 0..20u64 {
            e.insert(&a, VectorId(i), &v(i), json!({})).unwrap();
        }
        e.checkpoint().unwrap();
    }
    let seg = segment_files(&d).remove(0);
    let good = std::fs::read(&seg).unwrap();

    let mut bad = good.clone();
    let mid = bad.len() / 2;
    bad[mid] ^= 0x55;
    std::fs::write(&seg, &bad).unwrap();
    assert_eq!(
        Engine::open(&d, Config::new(2)).err().unwrap().code(),
        "WAL_CORRUPT"
    );

    std::fs::write(&seg, &good).unwrap();
    let wal = d.join("wal.log");
    let wal_bytes = std::fs::read(&wal).unwrap();
    std::fs::remove_file(&wal).unwrap();
    assert_eq!(
        Engine::open(&d, Config::new(2)).err().unwrap().code(),
        "WAL_CORRUPT"
    );
    std::fs::write(&wal, &wal_bytes).unwrap();
    assert_eq!(
        Engine::open(&d, Config::new(2)).unwrap().count(&a).unwrap(),
        20
    );
}

#[test]
fn log_that_starts_after_the_newest_segment_is_refused() {
    let d = dir("gap");
    let a = full("a");
    {
        let e = Engine::open(&d, Config::new(2)).unwrap();
        for i in 0..10u64 {
            e.insert(&a, VectorId(i), &v(i), json!({})).unwrap();
        }
        e.checkpoint().unwrap();
        for i in 10..20u64 {
            e.insert(&a, VectorId(i), &v(i), json!({})).unwrap();
        }
        e.checkpoint().unwrap();
    }
    drop(Engine::open(&d, Config::new(2)).unwrap());
    let segs = segment_files(&d);
    assert_eq!(segs.len(), 1);
    let mut forged = std::fs::read(&segs[0]).unwrap();
    std::fs::remove_file(&segs[0]).unwrap();
    forged[12..20].copy_from_slice(&5u64.to_le_bytes());
    let body_end = forged.len() - 12;
    let crc = crc32(&forged[..body_end]);
    let len = forged.len();
    forged[len - 4..].copy_from_slice(&crc.to_le_bytes());
    std::fs::write(d.join("seg-00000000000000000005.atlas"), forged).unwrap();
    assert_eq!(
        Engine::open(&d, Config::new(2)).err().unwrap().code(),
        "WAL_CORRUPT"
    );
}

#[test]
fn checkpoints_taken_under_concurrent_writes_recover_to_the_exact_final_state() {
    let d = dir("conc");
    let e = std::sync::Arc::new(Engine::open(&d, Config::new(2)).unwrap());
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut writers = Vec::new();
    for w in 0..4u64 {
        let (e, name) = (e.clone(), format!("t{}", w % 2));
        writers.push(std::thread::spawn(move || {
            let s = full(&name);
            for i in 0..150u64 {
                let id = VectorId(w * 1000 + i);
                if i % 3 == 0 {
                    e.insert_op(&s, &format!("op-{w}-{i}"), id, &v(i), json!({"w": w}))
                        .unwrap();
                } else {
                    e.insert(&s, id, &v(i), json!({"w": w})).unwrap();
                }
                if i % 10 == 9 {
                    e.delete(&s, VectorId(w * 1000 + i - 5)).unwrap();
                }
            }
        }));
    }
    let cp = {
        let (e, stop) = (e.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut n = 0;
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                e.checkpoint().unwrap();
                n += 1;
            }
            n
        })
    };
    for h in writers {
        h.join().unwrap();
    }
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let rounds = cp.join().unwrap();
    assert!(rounds > 0);
    let (t0, t1) = (snapshot_of(&e, "t0"), snapshot_of(&e, "t1"));
    drop(e);
    let e = Engine::open(&d, Config::new(2)).unwrap();
    assert_eq!(snapshot_of(&e, "t0"), t0);
    assert_eq!(snapshot_of(&e, "t1"), t1);
    let s0 = full("t0");
    let again = e
        .insert_op(&s0, "op-0-0", VectorId(0), &v(0), json!({"w": 0}))
        .unwrap();
    assert!(again >= 1);
    assert_eq!(snapshot_of(&e, "t0"), t0);
    assert_eq!(segment_files(&d).len(), 1);
}

#[test]
fn leftover_temp_segment_from_a_crash_is_cleaned_and_ignored() {
    let d = dir("tmpseg");
    let a = full("a");
    {
        let e = Engine::open(&d, Config::new(2)).unwrap();
        e.insert(&a, VectorId(1), &v(1), json!({})).unwrap();
        e.checkpoint().unwrap();
    }
    std::fs::write(d.join("seg-00000000000000000099.atlas.tmp"), b"torn").unwrap();
    let e = Engine::open(&d, Config::new(2)).unwrap();
    assert_eq!(e.count(&a).unwrap(), 1);
    assert!(!d.join("seg-00000000000000000099.atlas.tmp").exists());
}
