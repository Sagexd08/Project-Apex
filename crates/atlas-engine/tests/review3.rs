//! Review-fix slice S3: float fidelity + idempotent retry, iterative depth cap, op-gate
//! concurrency, bounded Incomplete reason, config-shrink pin, poisoned engine.
mod common;

use atlas_core::{AtlasError, Config, EffectiveScope, Metric, PermSet, VectorId};
use atlas_engine::{BoxedIndex, Engine, Hit, Item};
use atlas_index::{AnnIndex, FlatIndex};
use common::{full, scope};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::{Arc, Barrier};
use std::time::Duration;

fn tmp(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-r3-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn wal_len(dir: &Path) -> u64 {
    std::fs::metadata(dir.join("wal.log")).unwrap().len()
}

/// Deterministic xorshift stream of ordinary doubles in [0, 1000).
fn stream() -> impl Iterator<Item = f64> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    std::iter::repeat_with(move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        1000.0 * ((x >> 11) as f64 / (1u64 << 53) as f64)
    })
}

/// 50 doubles that drift under a shortest-repr serialize -> parse trip when serde_json lacks
/// `float_roundtrip` (found by scanning up to 100_000 candidates). With the feature on none
/// drift, so fall back to the first 50 values of the same stream (the tests stay meaningful).
fn fifty_floats() -> Vec<f64> {
    let drift: Vec<f64> = stream()
        .take(100_000)
        .filter(|v| serde_json::from_str::<f64>(&serde_json::to_string(v).unwrap()).unwrap() != *v)
        .take(50)
        .collect();
    if drift.len() == 50 {
        drift
    } else {
        stream().take(50).collect()
    }
}

/// Known-drifting literal from the review (985.6906946328695 -> ...696 without the feature).
const REVIEW_VALUE: f64 = 985.6906946328695;

fn md_for(x: f64) -> Value {
    json!({"x": x, "nested": [x, {"y": x}], "n": 1})
}

/// Metadata cases: edge doubles and integers, then the scanned drifting values.
/// JSON-number semantics pinned here: serde_json keeps the sign of -0.0 ("-0.0"), `Value ==`
/// treats 0.0 and -0.0 as equal, so every comparison also checks the serialized text.
fn cases() -> Vec<Value> {
    let mut floats = vec![
        0.0,
        -0.0,
        f64::MIN_POSITIVE,
        f64::MIN_POSITIVE / 2.0,
        5e-324,
        f64::MAX,
        f64::MIN,
        f64::EPSILON,
        1e21,
        1e20,
        1e-7,
        f64::from_bits(1e-6f64.to_bits() - 1),
        0.1 + 0.2,
        1.0 / 3.0,
        9007199254740992.0,
        9007199254740994.0,
        REVIEW_VALUE,
    ];
    floats.extend(fifty_floats());
    let mut v: Vec<Value> = floats.into_iter().map(md_for).collect();
    v.push(json!({
        "i_min": i64::MIN, "i_max": i64::MAX, "u_max": u64::MAX,
        "b53": (1u64 << 53) + 1, "neg53": -((1i64 << 53) + 1)
    }));
    v
}

fn same(a: &Value, b: &Value) -> bool {
    a == b && serde_json::to_string(a).unwrap() == serde_json::to_string(b).unwrap()
}

// ---------------- F1 ----------------

#[test]
fn f1_metadata_comes_back_exactly_live_and_after_reopen() {
    let dir = tmp("f1fetch");
    let e = Engine::open(&dir, Config::new(2)).unwrap();
    let a = full("a");
    let cases = cases();
    for (i, md) in cases.iter().enumerate() {
        e.insert(&a, VectorId(i as u64), &[1.0, 0.0], md.clone())
            .unwrap();
    }
    let check = |e: &Engine| {
        for (i, md) in cases.iter().enumerate() {
            let got = e.fetch(&a, VectorId(i as u64)).unwrap();
            assert!(same(&got.metadata, md), "fetch id {i}: {md}");
        }
        let mut seen = 0;
        let mut sink = |it: Item| {
            assert!(same(&it.metadata, &cases[it.id.0 as usize]), "snapshot");
            seen += 1;
            Ok(())
        };
        e.snapshot(&a, &mut sink).unwrap();
        e.export(&a, &mut sink).unwrap();
        assert_eq!(seen, 2 * cases.len());
    };
    check(&e);
    drop(e);
    check(&Engine::open(&dir, Config::reopen()).unwrap());
}

#[test]
fn f1_insert_op_retry_returns_original_seq_live_and_after_reopen() {
    let dir = tmp("f1retry");
    let e = Engine::open(&dir, Config::new(2)).unwrap();
    let a = full("a");
    let cases = cases();
    let mut seqs = vec![];
    for (i, md) in cases.iter().enumerate() {
        let op = format!("op-{i}");
        let id = VectorId(i as u64);
        let seq = e.insert_op(&a, &op, id, &[1.0, 0.0], md.clone()).unwrap();
        let again = e.insert_op(&a, &op, id, &[1.0, 0.0], md.clone());
        assert_eq!(again.unwrap(), seq, "live retry op {i}: {md}");
        seqs.push(seq);
    }
    let conflict = |e: &Engine, i: usize| {
        let other = json!({ "other": cases[i].clone() });
        let id = VectorId(i as u64);
        e.insert_op(&a, &format!("op-{i}"), id, &[1.0, 0.0], other)
            .unwrap_err()
    };
    assert_eq!(conflict(&e, 0).code(), "OP_ID_CONFLICT");
    let len = e.count(&a).unwrap();
    drop(e);

    let e = Engine::open(&dir, Config::reopen()).unwrap();
    for (i, md) in cases.iter().enumerate() {
        let id = VectorId(i as u64);
        let r = e.insert_op(&a, &format!("op-{i}"), id, &[1.0, 0.0], md.clone());
        assert_eq!(r.unwrap(), seqs[i], "replayed retry op {i}: {md}");
    }
    assert_eq!(conflict(&e, 3).code(), "OP_ID_CONFLICT");
    assert_eq!(e.count(&a).unwrap(), len, "retries must not add records");
}

// ---------------- F2 ----------------

/// Metadata of exactly `depth` levels (scalar = 1, each nested array adds 1), built iteratively.
fn with_depth(depth: usize) -> Value {
    let mut v = json!(0);
    for _ in 1..depth {
        v = Value::Array(vec![v]);
    }
    v
}

const CHILD_ENV: &str = "ATLAS_F2_CHILD";

/// Parent: re-executes this test binary running only the `#[ignore]`d child, so a stack
/// overflow (process abort) cannot take the runner down; asserts clean exit + typed-error marker.
#[test]
fn f2_extreme_depth_runs_in_an_isolated_subprocess_and_is_a_typed_error() {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "f2_child_extreme_depth",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    println!("child exit status: {:?}", out.status);
    assert!(
        out.status.success(),
        "child status {:?}\n{stdout}\n{stderr}",
        out.status
    );
    assert!(
        stdout.contains("F2_TYPED_ERROR_OK insert=INVALID_METADATA insert_op=INVALID_METADATA"),
        "marker missing:\n{stdout}"
    );
    // S6-4: the exit-path results are asserted too, so a child that skips `f1_exit_paths()` (or
    // drops its marker) fails here instead of passing on the exit status alone. Tokens are
    // compared individually: the marker line's internal spacing is not part of the contract.
    let line = stdout
        .lines()
        .find(|l| l.starts_with("F1_EXIT_PATHS_OK"))
        .unwrap_or_else(|| panic!("F1_EXIT_PATHS_OK marker missing:\n{stdout}"));
    let tokens: Vec<&str> = line.split_whitespace().collect();
    for want in [
        "forbidden=FORBIDDEN",
        "no_write_perm=FORBIDDEN",
        "poisoned=WAL_CORRUPT",
        "bad_op_id=INVALID_METADATA",
        "bad_dim=INVALID_METADATA",
        "shallow_bad_op_id=INVALID_OP_ID",
        "shallow_bad_dim=INVALID_VECTOR_DIMENSION",
    ] {
        assert!(tokens.contains(&want), "{want} missing in: {line}");
    }
}

/// Child: the whole public insert path (including rejection and cleanup of the owned input) on a
/// 256 KiB stack. Any recursion over the 200_000-deep value (serialize, clone, drop) aborts.
#[test]
#[ignore = "run only by the subprocess parent"]
fn f2_child_extreme_depth() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let dir = tmp("f2");
    let e = Arc::new(Engine::open(&dir, Config::new(2)).unwrap());
    let a = full("a");
    e.insert(&a, VectorId(1), &[1.0, 0.0], json!(null)).unwrap();
    let before = wal_len(&dir);
    let (e2, a2) = (e.clone(), a.clone());
    let h = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            let r1 = e2.insert(&a2, VectorId(2), &[1.0, 0.0], with_depth(200_000));
            let r2 = e2.insert_op(&a2, "op-d", VectorId(3), &[1.0, 0.0], with_depth(200_000));
            (r1.unwrap_err().code(), r2.unwrap_err().code())
        })
        .unwrap();
    let (c1, c2) = h.join().unwrap();
    assert_eq!(wal_len(&dir), before, "WAL untouched");
    e.insert_op(&a, "op-d", VectorId(3), &[1.0, 0.0], json!(1))
        .unwrap();
    assert_eq!(e.count(&a).unwrap(), 2);
    println!("F2_TYPED_ERROR_OK insert={c1} insert_op={c2}");
    f1_exit_paths();
}

/// S5/F1: each non-success exit of insert/insert_op with a 200_000-deep owned value, on a 256 KiB
/// stack. Before the fix `authz(..)?` / `check_poison()?` dropped the value recursively (abort).
/// `bad_op_id` / `bad_dim` with the deep value are rejected by the depth cap first (also an
/// iterative drop); the `shallow_*` variants exercise the later returns with a depth-100 value.
fn f1_exit_paths() {
    let dir = tmp("f1exit");
    let e = Arc::new(
        Engine::open_with_index(&dir, Config::new(2), |d| -> BoxedIndex {
            Box::new(Refuses(FlatIndex::new(d)))
        })
        .unwrap(),
    );
    let a = full("a");
    let read_only = scope("a", PermSet::SEARCH);
    let (e2, a2) = (e.clone(), a.clone());
    let h = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            let deep = || with_depth(200_000);
            let code = |r: Result<u64, AtlasError>| r.unwrap_err().code();
            let none = EffectiveScope::default();
            let forbidden = code(e2.insert(&none, VectorId(1), &[1.0, 0.0], deep()));
            let forbidden_op = code(e2.insert_op(&none, "o", VectorId(1), &[1.0, 0.0], deep()));
            assert_eq!(forbidden, forbidden_op);
            let no_perm = code(e2.insert(&read_only, VectorId(1), &[1.0, 0.0], deep()));
            let no_perm_op = code(e2.insert_op(&read_only, "o", VectorId(1), &[1.0, 0.0], deep()));
            assert_eq!(no_perm, no_perm_op);
            let bad_op = code(e2.insert_op(&a2, "", VectorId(1), &[1.0, 0.0], deep()));
            let bad_dim = code(e2.insert(&a2, VectorId(1), &[1.0], deep()));
            let shallow_op = code(e2.insert_op(&a2, "", VectorId(1), &[1.0, 0.0], with_depth(100)));
            let shallow_dim = code(e2.insert(&a2, VectorId(1), &[1.0], with_depth(100)));
            // Poison the engine through the supported failure path, then insert again.
            let _ = e2.insert(&a2, VectorId(666), &[1.0, 0.0], json!(null));
            let poisoned = code(e2.insert(&a2, VectorId(1), &[1.0, 0.0], deep()));
            let poisoned_op = code(e2.insert_op(&a2, "o", VectorId(1), &[1.0, 0.0], deep()));
            assert_eq!(poisoned, poisoned_op);
            (
                forbidden,
                no_perm,
                poisoned,
                bad_op,
                bad_dim,
                shallow_op,
                shallow_dim,
            )
        })
        .unwrap();
    let (f, n, p, bo, bd, so, sd) = h.join().unwrap();
    println!(
        "F1_EXIT_PATHS_OK forbidden={f} no_write_perm={n} poisoned={p} bad_op_id={bo}          bad_dim={bd} shallow_bad_op_id={so} shallow_bad_dim={sd}"
    );
}

/// Rejected writes leave NO trace: no WAL bytes, no seq, no record, no op-map entry.
#[test]
fn f3_rejected_metadata_leaves_no_trace() {
    let dir = tmp("notrace");
    let mut cfg = Config::new(2);
    cfg.max_record_len = 1000;
    let e = Engine::open(&dir, cfg).unwrap();
    let a = full("a");
    let seq0 = e.insert(&a, VectorId(1), &[1.0, 0.0], json!(null)).unwrap();
    let (len0, cnt0, total0) = (wal_len(&dir), e.count(&a).unwrap(), e.len());
    type Build = Box<dyn Fn() -> Value>;
    let bad: Vec<(&str, Build, &str)> = vec![
        (
            "depth 101",
            Box::new(|| with_depth(101)),
            "INVALID_METADATA",
        ),
        (
            "depth 200000",
            Box::new(|| with_depth(200_000)),
            "INVALID_METADATA",
        ),
        (
            "oversized",
            Box::new(|| json!({"p": "x".repeat(5000)})),
            "RECORD_TOO_LARGE",
        ),
    ];
    for (name, md, code) in bad {
        let r = e.insert(&a, VectorId(9), &[1.0, 0.0], md());
        assert_eq!(r.unwrap_err().code(), code, "insert {name}");
        let r = e.insert_op(&a, "rej", VectorId(9), &[1.0, 0.0], md());
        assert_eq!(r.unwrap_err().code(), code, "insert_op {name}");
        assert_eq!(wal_len(&dir), len0, "{name}: WAL bytes");
        assert_eq!(e.count(&a).unwrap(), cnt0, "{name}: records");
        assert_eq!(e.len(), total0, "{name}: partitions");
    }
    // No seq consumed; no op-map entry (a different payload under "rej" is not a conflict).
    assert_eq!(
        e.insert(&a, VectorId(2), &[1.0, 0.0], json!(1)).unwrap(),
        seq0 + 1
    );
    let s = e.insert_op(&a, "rej", VectorId(3), &[1.0, 0.0], json!(2));
    assert_eq!(s.unwrap(), seq0 + 2);
}

// ---------------- F3 ----------------

#[test]
fn f3_concurrent_retries_of_one_op_id_make_exactly_one_record() {
    let dir = tmp("f3same");
    let e = Arc::new(Engine::open(&dir, Config::new(2)).unwrap());
    let a = full("a");
    let hs: Vec<_> = (0..16)
        .map(|_| {
            let (e, a) = (e.clone(), a.clone());
            std::thread::spawn(move || {
                e.insert_op(
                    &a,
                    "same-op",
                    VectorId(7),
                    &[1.0, 0.0],
                    md_for(REVIEW_VALUE),
                )
                .unwrap()
            })
        })
        .collect();
    let seqs: Vec<u64> = hs.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(seqs.windows(2).all(|w| w[0] == w[1]), "{seqs:?}");
    assert_eq!(e.count(&a).unwrap(), 1);
    drop(e);
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(e.count(&a).unwrap(), 1);
}

#[test]
fn f3_concurrent_same_op_id_different_payloads_exactly_one_wins() {
    let dir = tmp("f3diff");
    let e = Arc::new(Engine::open(&dir, Config::new(2)).unwrap());
    let a = full("a");
    let seq0 = e
        .insert(&a, VectorId(10_000), &[1.0, 0.0], json!(0))
        .unwrap();
    const ITERS: u64 = 200;
    const T: usize = 4;
    let mut winners = vec![];
    for it in 0..ITERS {
        let barrier = Arc::new(Barrier::new(T));
        let hs: Vec<_> = (0..T)
            .map(|t| {
                let (e, a, b) = (e.clone(), a.clone(), barrier.clone());
                std::thread::spawn(move || {
                    b.wait();
                    let op = format!("op-{it}");
                    (
                        t,
                        e.insert_op(&a, &op, VectorId(it), &[1.0, 0.0], json!({ "w": t })),
                    )
                })
            })
            .collect();
        let res: Vec<_> = hs.into_iter().map(|h| h.join().unwrap()).collect();
        let wins: Vec<_> = res.iter().filter(|(_, r)| r.is_ok()).collect();
        assert_eq!(wins.len(), 1, "iteration {it}: {res:?}");
        for (_, r) in &res {
            if let Err(err) = r {
                assert_eq!(err.code(), "OP_ID_CONFLICT", "iteration {it}");
            }
        }
        let w = wins[0].0;
        assert_eq!(
            e.fetch(&a, VectorId(it)).unwrap().metadata,
            json!({ "w": w })
        );
        winners.push(w);
    }
    // Exactly ITERS WAL records were written: the next seq is seq0 + ITERS + 1.
    assert_eq!(e.count(&a).unwrap() as u64, ITERS + 1);
    let next = e
        .insert(&a, VectorId(10_001), &[1.0, 0.0], json!(0))
        .unwrap();
    assert_eq!(
        next,
        seq0 + ITERS + 1,
        "a loser must not have consumed a seq"
    );
    drop(e);
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    for (it, w) in winners.iter().enumerate() {
        let got = e.fetch(&a, VectorId(it as u64)).unwrap().metadata;
        assert_eq!(got, json!({ "w": w }), "after replay, iteration {it}");
    }
}

#[test]
fn f3_many_tenants_with_distinct_op_ids_all_succeed() {
    let dir = tmp("f3many");
    let e = Arc::new(Engine::open(&dir, Config::new(2)).unwrap());
    let hs: Vec<_> = (0..12)
        .map(|t| {
            let e = e.clone();
            std::thread::spawn(move || {
                let s = full(&format!("t{t}"));
                let op = format!("op-{t}");
                let seq = e
                    .insert_op(&s, &op, VectorId(1), &[1.0, 0.0], json!({ "t": t }))
                    .unwrap();
                (s, seq)
            })
        })
        .collect();
    let mut all = vec![];
    for h in hs {
        let (s, seq) = h.join().unwrap();
        assert_eq!(e.count(&s).unwrap(), 1);
        all.push(seq);
    }
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 12, "each tenant got its own seq");
    assert_eq!(e.len(), 12);
}

// ---------------- F5 ----------------

#[test]
fn f5_incomplete_reason_is_bounded_for_a_huge_sink_error() {
    let dir = tmp("f5");
    let e = Engine::open(&dir, Config::new(2)).unwrap();
    let a = full("a");
    e.insert(&a, VectorId(1), &[1.0, 0.0], json!(null)).unwrap();
    let huge = "\u{20ac}".repeat(1 << 20); // ~1M chars, 3 MB
    let mut sink = |_i: Item| Err(AtlasError::Codec(huge.clone()));
    match e.snapshot(&a, &mut sink) {
        Err(AtlasError::Incomplete { emitted, reason }) => {
            assert_eq!(emitted, 0);
            assert!(reason.chars().count() <= 300, "reason len {}", reason.len());
            assert!(reason.contains("truncated"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
}

// ---------------- F6 ----------------

#[test]
fn f6_reopen_with_smaller_max_record_len_fails_with_wal_corrupt() {
    let dir = tmp("f6");
    let e = Engine::open(&dir, Config::new(2)).unwrap();
    let a = full("a");
    e.insert(
        &a,
        VectorId(1),
        &[1.0, 0.0],
        json!({"pad": "x".repeat(2000)}),
    )
    .unwrap();
    drop(e);
    let mut cfg = Config::reopen();
    cfg.max_record_len = 200;
    let err = Engine::open(&dir, cfg).err().expect("must fail");
    assert_eq!(err.code(), "WAL_CORRUPT");
    assert!(err.to_string().contains("exceeds configured max"), "{err}");
    // The operator fix (restore the limit) recovers everything.
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(e.count(&a).unwrap(), 1);
}

// ---------------- extras ----------------

#[test]
fn same_id_written_twice_in_one_group_commit_batch_snapshots_coherently() {
    let dir = tmp("samebatch");
    let mut cfg = Config::new(2);
    cfg.batch_window = Duration::from_millis(15);
    cfg.max_batch = 1024;
    let e = Arc::new(Engine::open(&dir, cfg).unwrap());
    let a = full("a");
    const IDS: u64 = 6;
    for id in 0..IDS {
        let md = json!({"id": id, "w": 0});
        e.insert(&a, VectorId(id), &[id as f32, 0.0], md).unwrap();
    }
    let done = Arc::new(AtomicBool::new(false));
    let writers: Vec<_> = (1..=2u64)
        .map(|w| {
            let (e, a) = (e.clone(), a.clone());
            std::thread::spawn(move || {
                for _ in 0..20 {
                    for id in 0..IDS {
                        let md = json!({"id": id, "w": w});
                        e.insert(&a, VectorId(id), &[id as f32, w as f32], md)
                            .unwrap();
                    }
                }
            })
        })
        .collect();
    let snapper = {
        let (e, a, done) = (e.clone(), a.clone(), done.clone());
        std::thread::spawn(move || {
            let mut snaps = 0;
            while !done.load(SeqCst) || snaps < 3 {
                let mut items = vec![];
                let mut sink = |i: Item| {
                    items.push(i);
                    Ok(())
                };
                e.snapshot(&a, &mut sink).unwrap();
                let mut seen: HashMap<u64, ()> = HashMap::new();
                for it in &items {
                    assert!(seen.insert(it.id.0, ()).is_none(), "duplicate {}", it.id.0);
                    let w = it.metadata["w"].as_u64().unwrap();
                    assert_eq!(it.vector, vec![it.id.0 as f32, w as f32], "torn pair");
                }
                assert_eq!(items.len() as u64, IDS);
                snaps += 1;
            }
        })
    };
    for w in writers {
        w.join().unwrap();
    }
    done.store(true, SeqCst);
    snapper.join().unwrap();
}

/// Index that refuses id 666: stands in for a failing `AnnIndex` so the LIVE applier fails.
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

#[test]
fn live_applier_failure_poisons_writes_but_not_reads_and_reopen_recovers() {
    let dir = tmp("poison");
    let e = Engine::open_with_index(&dir, Config::new(2), |d| -> BoxedIndex {
        Box::new(Refuses(FlatIndex::new(d)))
    })
    .unwrap();
    let a = full("a");
    e.insert(&a, VectorId(1), &[1.0, 0.0], json!(null)).unwrap();
    let err = e
        .insert(&a, VectorId(666), &[1.0, 0.0], json!(null))
        .unwrap_err();
    assert_eq!(err.code(), "WAL_CORRUPT");
    assert!(err.to_string().contains("index refused"), "{err}");
    // Writes of every tenant now fail; applied data stays readable.
    for t in ["a", "b"] {
        let r = e.insert(&full(t), VectorId(2), &[1.0, 0.0], json!(null));
        assert_eq!(r.unwrap_err().code(), "WAL_CORRUPT", "tenant {t}");
    }
    assert_eq!(e.count(&a).unwrap(), 1);
    drop(e);
    // Reopen with a healthy index: the admitted record 666 replays fine.
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(e.count(&a).unwrap(), 2);
}
