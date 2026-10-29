mod common;

use atlas_core::{AtlasError, Config, Metric, PermSet, VectorId};
use atlas_engine::{Engine, Item};
use atlas_storage::{write_meta, Wal};
use common::{full, scope};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::Arc;

fn tmp(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-hard-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn wal_len(dir: &Path) -> u64 {
    std::fs::metadata(dir.join("wal.log")).unwrap().len()
}

fn nested(depth: usize) -> Value {
    let mut v = json!(0);
    for _ in 0..depth {
        v = Value::Array(vec![v]);
    }
    v
}

fn drop_nested(mut v: Value) {
    while let Value::Array(mut a) = v {
        v = a.pop().unwrap_or(Value::Null);
    }
}

#[test]
fn deep_metadata_is_rejected_before_the_wal_and_engine_stays_healthy() {
    let dir = tmp("deep");
    let e = Engine::open(&dir, Config::new(2)).unwrap();
    let (a, b) = (full("a"), full("b"));
    let seq0 = e.insert(&a, VectorId(1), &[1.0, 0.0], json!(null)).unwrap();
    let before = wal_len(&dir);

    for (i, md) in [nested(130), nested(1000)].into_iter().enumerate() {
        let err = e.insert(&a, VectorId(10), &[1.0, 0.0], md).unwrap_err();
        assert_eq!(err.code(), "INVALID_METADATA", "plain insert #{i}");
        let md = nested(130 + i);
        let err = e
            .insert_op(&a, "op-deep", VectorId(11), &[1.0, 0.0], md)
            .unwrap_err();
        assert_eq!(err.code(), "INVALID_METADATA", "insert_op #{i}");
    }
    assert_eq!(wal_len(&dir), before, "WAL bytes must be untouched");

    assert_eq!(
        e.insert(&b, VectorId(1), &[0.0, 1.0], json!({"ok": true}))
            .unwrap(),
        seq0 + 1
    );
    assert_eq!(e.search(&b, &[0.0, 1.0], 5, Metric::Dot).unwrap().len(), 1);
    e.insert(&a, VectorId(2), &[1.0, 0.0], json!(null)).unwrap();
    e.insert_op(&a, "op-deep", VectorId(11), &[1.0, 0.0], json!(null))
        .unwrap();
    drop(e);

    let e = Engine::open(&dir, Config::reopen()).expect("reopen must succeed");
    assert_eq!(e.count(&a).unwrap(), 3);
    assert_eq!(e.count(&b).unwrap(), 1);
}

#[test]
fn accepted_metadata_depths_replay_after_reopen() {
    let dir = tmp("boundary");
    let e = Engine::open(&dir, Config::new(2)).unwrap();
    let a = full("a");
    let accepted: Vec<usize> = (1..=100).collect();
    for &depth in &accepted {
        e.insert(&a, VectorId(depth as u64), &[1.0, 0.0], nested(depth - 1))
            .unwrap_or_else(|err| panic!("depth {depth}: {err}"));
    }
    for depth in 101..=130usize {
        let err = e
            .insert(&a, VectorId(depth as u64), &[1.0, 0.0], nested(depth - 1))
            .unwrap_err();
        assert_eq!(err.code(), "INVALID_METADATA", "depth {depth}");
    }
    drop(e);

    let e = Engine::open(&dir, Config::reopen()).expect("replay of accepted depths");
    assert_eq!(e.count(&a).unwrap(), accepted.len());
    let got = e.fetch(&a, VectorId(100)).unwrap();
    assert_eq!(got.metadata, nested(99));
    drop_nested(got.metadata);
}

#[test]
fn oversized_metadata_is_a_typed_error_and_leaves_the_wal_alone() {
    let dir = tmp("big");
    let mut cfg = Config::new(2);
    cfg.max_record_len = 4096;
    let e = Engine::open(&dir, cfg).unwrap();
    let a = full("a");
    e.insert(&a, VectorId(1), &[1.0, 0.0], json!(null)).unwrap();
    let before = wal_len(&dir);
    let big = json!("x".repeat(10_000));
    let err = e
        .insert(&a, VectorId(2), &[1.0, 0.0], big.clone())
        .unwrap_err();
    assert_eq!(err.code(), "RECORD_TOO_LARGE");
    let err = e
        .insert_op(&a, "op-big", VectorId(2), &[1.0, 0.0], big)
        .unwrap_err();
    assert_eq!(err.code(), "RECORD_TOO_LARGE");
    assert_eq!(wal_len(&dir), before);
    assert_eq!(e.count(&a).unwrap(), 1);
}

const MB: usize = 1 << 20;

#[test]
fn huge_untrusted_strings_are_not_echoed_in_errors() {
    let long = "t".repeat(MB);
    let msg = atlas_core::TenantId::parse(&long).unwrap_err().to_string();
    assert!(msg.len() < 200, "tenant error echoed {} bytes", msg.len());
    assert!(msg.contains("truncated"), "{msg}");
    let wide = "\u{20ac}".repeat(MB / 3);
    let msg = atlas_core::TenantId::parse(&wide).unwrap_err().to_string();
    assert!(
        msg.len() < 1024 && msg.contains("truncated"),
        "{}",
        msg.len()
    );
    let msg = atlas_core::TenantId::parse("a b").unwrap_err().to_string();
    assert!(msg.contains("a b") && !msg.contains("truncated"), "{msg}");

    let dir = tmp("echo");
    let e = Engine::open(&dir, Config::new(2)).unwrap();
    let a = full("a");
    let op = "o".repeat(MB);
    let err = e
        .insert_op(&a, &op, VectorId(1), &[1.0, 0.0], json!(null))
        .unwrap_err();
    assert_eq!(err.code(), "INVALID_OP_ID");
    assert!(err.to_string().len() < 200, "{err}");
    let ok_op = "p".repeat(128);
    e.insert_op(&a, &ok_op, VectorId(1), &[1.0, 0.0], json!(1))
        .unwrap();
    let err = e
        .insert_op(&a, &ok_op, VectorId(1), &[1.0, 0.0], json!(2))
        .unwrap_err();
    assert_eq!(err.code(), "OP_ID_CONFLICT");
    assert!(err.to_string().len() < 400, "{}", err.to_string().len());
}

#[test]
fn replayed_bad_tenant_error_is_bounded() {
    let dir = tmp("echo-replay");
    std::fs::create_dir_all(&dir).unwrap();
    write_meta(&dir, 2).unwrap();
    let rec = format!(
        r#"{{"tenant":"{}","id":1,"vector":[1.0,0.0],"metadata":null}}"#,
        "z".repeat(MB)
    );
    let mut cfg = Config::new(2);
    cfg.max_record_len = 8 * MB as u32;
    {
        let (wal, _) = Wal::open(&dir.join("wal.log"), &cfg, Box::new(|_, _| {})).unwrap();
        wal.append(rec.into_bytes()).unwrap();
    }
    let mut cfg = Config::reopen();
    cfg.max_record_len = 8 * MB as u32;
    let err = Engine::open(&dir, cfg).err().expect("must fail");
    assert_eq!(err.code(), "WAL_CORRUPT");
    assert!(err.to_string().len() < 400, "{}", err.to_string().len());
}

#[test]
fn incomplete_carries_the_emitted_count_as_a_typed_field() {
    let dir = tmp("incomplete");
    let e = Engine::open(&dir, Config::new(2)).unwrap();
    let a = full("a");
    for i in 1..=6 {
        e.insert(&a, VectorId(i), &[1.0, 0.0], json!(null)).unwrap();
    }
    for export in [false, true] {
        let mut seen = 0u64;
        let mut sink = |_: Item| {
            seen += 1;
            if seen == 4 {
                Err(AtlasError::Io(std::io::Error::other("disk full")))
            } else {
                Ok(())
            }
        };
        let r = if export {
            e.export(&a, &mut sink)
        } else {
            e.snapshot(&a, &mut sink)
        };
        match r {
            Err(AtlasError::Incomplete { emitted, reason }) => {
                assert_eq!(emitted, 3);
                assert!(reason.contains("disk full"), "{reason}");
            }
            other => panic!("expected typed Incomplete, got {other:?}"),
        }
    }
}

fn collect_all(e: &Engine, s: &atlas_core::EffectiveScope) -> (Vec<Item>, u64) {
    let mut items = vec![];
    let n = e
        .snapshot(s, &mut |i| {
            items.push(i);
            Ok(())
        })
        .unwrap();
    (items, n)
}

#[test]
fn panicking_sink_unwinds_without_result_or_poison() {
    let dir = tmp("panic");
    let e = Engine::open(&dir, Config::new(2)).unwrap();
    let (a, b) = (full("a"), full("b"));
    for i in 1..=5 {
        e.insert(&a, VectorId(i), &[1.0, 0.0], json!(i)).unwrap();
    }
    e.insert(&b, VectorId(1), &[0.0, 1.0], json!(null)).unwrap();
    for export in [false, true] {
        let mut seen = 0;
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut sink = |_: Item| {
                seen += 1;
                if seen == 2 {
                    panic!("sink exploded");
                }
                Ok(())
            };
            if export {
                e.export(&a, &mut sink)
            } else {
                e.snapshot(&a, &mut sink)
            }
        }));
        assert!(r.is_err(), "panic must propagate; no Ok/Err result");
        assert_eq!(seen, 2);
    }
    assert_eq!(e.count(&a).unwrap(), 5);
    assert_eq!(e.count(&b).unwrap(), 1);
    e.insert(&b, VectorId(2), &[0.0, 1.0], json!(null)).unwrap();
    e.insert(&a, VectorId(6), &[1.0, 0.0], json!(6)).unwrap();
    assert_eq!(e.search(&a, &[1.0, 0.0], 10, Metric::Dot).unwrap().len(), 6);
    let (items, n) = collect_all(&e, &a);
    assert_eq!((items.len() as u64, n), (6, 6));
    drop(e);
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(e.count(&a).unwrap(), 6);
}

#[test]
fn snapshot_under_concurrent_writes_is_coherent_and_single_tenant() {
    let dir = tmp("concurrent");
    let e = Arc::new(Engine::open(&dir, Config::new(2)).unwrap());
    let (a, b) = (full("a"), full("b"));
    const IDS: u64 = 20;
    const ROUNDS: u64 = 15;
    let write =
        |e: &Engine, s: &atlas_core::EffectiveScope, t: &str, base: u64, id: u64, r: u64| {
            e.insert(
                s,
                VectorId(base + id),
                &[(base + id) as f32, r as f32],
                json!({"t": t, "id": base + id, "r": r}),
            )
            .unwrap();
        };
    for id in 0..IDS {
        write(&e, &a, "a", 0, id, 0);
    }
    let done = Arc::new(AtomicBool::new(false));
    let mut writers = vec![];
    for (s, t, base) in [(a.clone(), "a", 0u64), (b.clone(), "b", 1000u64)] {
        let e = e.clone();
        writers.push(std::thread::spawn(move || {
            for r in 1..=ROUNDS {
                for id in 0..IDS {
                    e.insert(
                        &s,
                        VectorId(base + id),
                        &[(base + id) as f32, r as f32],
                        json!({"t": t, "id": base + id, "r": r}),
                    )
                    .unwrap();
                }
            }
        }));
    }
    let snapper = {
        let (e, a, done) = (e.clone(), a.clone(), done.clone());
        std::thread::spawn(move || {
            let mut snaps = 0;
            while !done.load(SeqCst) || snaps < 3 {
                let (items, n) = collect_all(&e, &a);
                assert_eq!(n as usize, items.len());
                let mut seen: HashMap<u64, ()> = HashMap::new();
                for it in &items {
                    assert!(it.id.0 < 1000, "foreign tenant id {}", it.id.0);
                    assert!(seen.insert(it.id.0, ()).is_none(), "duplicate {}", it.id.0);
                    let md = &it.metadata;
                    assert_eq!(md["t"], "a");
                    assert_eq!(md["id"], it.id.0);
                    let r = md["r"].as_u64().unwrap();
                    assert!(r <= ROUNDS);
                    assert_eq!(it.vector, vec![it.id.0 as f32, r as f32]);
                }
                assert_eq!(items.len() as u64, IDS);
                snaps += 1;
            }
            snaps
        })
    };
    for w in writers {
        w.join().unwrap();
    }
    done.store(true, SeqCst);
    assert!(snapper.join().unwrap() >= 3);
    let (items, n) = collect_all(&e, &a);
    assert_eq!((items.len() as u64, n), (IDS, IDS));
    assert!(items.iter().all(|i| i.metadata["r"] == ROUNDS));
}

#[test]
fn case_distinct_tenants_are_distinct_partitions_and_lookalikes_fail_replay() {
    let dir = tmp("case");
    let e = Engine::open(&dir, Config::new(2)).unwrap();
    let (up, lo) = (full("A"), full("a"));
    e.insert(&up, VectorId(1), &[1.0, 0.0], json!("upper"))
        .unwrap();
    e.insert(&lo, VectorId(1), &[0.0, 1.0], json!("lower"))
        .unwrap();
    e.insert(&lo, VectorId(2), &[0.0, 1.0], json!("lower2"))
        .unwrap();
    assert_eq!(e.count(&up).unwrap(), 1);
    assert_eq!(e.count(&lo).unwrap(), 2);
    assert_eq!(e.fetch(&up, VectorId(1)).unwrap().metadata, json!("upper"));
    assert_eq!(e.fetch(&lo, VectorId(1)).unwrap().metadata, json!("lower"));
    assert_eq!(e.fetch(&up, VectorId(2)).unwrap_err().code(), "NOT_FOUND");
    drop(e);
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(e.count(&up).unwrap(), 1);
    assert_eq!(e.count(&lo).unwrap(), 2);
    assert_eq!(
        e.count(&scope("A", PermSet::COUNT)).unwrap(),
        1,
        "scoped lookup"
    );
    drop(e);

    let lookalikes = ["\u{430}", "a\u{200b}", "\u{ff41}", "a ", " a", "a\0", "a\n"];
    let long = "a".repeat(65);
    for (n, t) in lookalikes
        .iter()
        .copied()
        .chain([long.as_str()])
        .enumerate()
    {
        let d = tmp(&format!("look-{n}"));
        std::fs::create_dir_all(&d).unwrap();
        write_meta(&d, 2).unwrap();
        let rec = serde_json::to_vec(&json!({
            "tenant": t, "id": 1, "vector": [1.0, 0.0], "metadata": null
        }))
        .unwrap();
        {
            let (wal, _) =
                Wal::open(&d.join("wal.log"), &Config::new(2), Box::new(|_, _| {})).unwrap();
            wal.append(rec).unwrap();
        }
        let err = Engine::open(&d, Config::reopen())
            .err()
            .expect("must reject");
        assert_eq!(err.code(), "WAL_CORRUPT", "{t:?}");
    }
}
