mod common;

use atlas_core::{AtlasError, Config, Metric, VectorId};
use atlas_engine::{BoxedIndex, Engine, Hit};
use atlas_index::{AnnIndex, FlatIndex};
use common::full;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

const SAFEGUARD: Duration = Duration::from_secs(30);
const PANIC_MARK: &str = "S6-PANIC-MARKER-7f3a";
const ERR_MARK: &str = "S6-ERR-MARKER-91c2";
const HANG: &str = "caller hung: committer died without waking waiters";

fn tmp(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-s6-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Err,
    Panic,
    PanicAfterMutate,
    PanicRemove,
}

struct Hostile {
    ix: FlatIndex,
    mode: Mode,
}

impl AnnIndex for Hostile {
    fn dim(&self) -> usize {
        self.ix.dim()
    }
    fn len(&self) -> usize {
        self.ix.len()
    }
    fn insert(&mut self, id: VectorId, v: &[f32], md: Value) -> Result<(), AtlasError> {
        if id.0 == 666 {
            match self.mode {
                Mode::Err => return Err(AtlasError::Codec(ERR_MARK.into())),
                Mode::Panic => panic!("{PANIC_MARK}"),
                Mode::PanicAfterMutate => {
                    self.ix.insert(id, v, md)?;
                    panic!("{PANIC_MARK}");
                }
                Mode::PanicRemove => {}
            }
        }
        self.ix.insert(id, v, md)
    }
    fn entries(&self) -> Vec<(VectorId, Vec<f32>, Value)> {
        self.ix.entries()
    }
    fn remove(&mut self, id: VectorId) -> bool {
        if id.0 == 777 && self.mode == Mode::PanicRemove {
            panic!("{PANIC_MARK}");
        }
        self.ix.remove(id)
    }
    fn search(&self, q: &[f32], k: usize, m: Metric) -> Result<Vec<Hit>, AtlasError> {
        self.ix.search(q, k, m)
    }
}

fn factory(mode: Mode) -> impl Fn(usize) -> BoxedIndex + Send + Sync + 'static {
    move |d| -> BoxedIndex {
        Box::new(Hostile {
            ix: FlatIndex::new(d),
            mode,
        })
    }
}

type Res = Result<u64, AtlasError>;
type Calls = Vec<(&'static str, Box<dyn FnOnce(&Engine) -> Res + Send>)>;

fn wal_len(dir: &Path) -> u64 {
    std::fs::metadata(dir.join("wal.log")).unwrap().len()
}

fn spawn_call(
    e: &Arc<Engine>,
    tx: &Sender<(&'static str, Res)>,
    name: &'static str,
    f: Box<dyn FnOnce(&Engine) -> Res + Send>,
) {
    let (e, tx) = (e.clone(), tx.clone());
    std::thread::spawn(move || {
        let r = f(&e);
        let _ = tx.send((name, r));
    });
}

fn collect(
    rx: &std::sync::mpsc::Receiver<(&'static str, Res)>,
    n: usize,
) -> HashMap<&'static str, Res> {
    (0..n)
        .map(|_| {
            rx.recv_timeout(SAFEGUARD)
                .unwrap_or_else(|_| panic!("{HANG} (HANG/TIMEOUT, not a correctness failure)"))
        })
        .collect()
}

fn ins(t: &'static str, id: u64) -> Box<dyn FnOnce(&Engine) -> Res + Send> {
    Box::new(move |e| e.insert(&full(t), VectorId(id), &[1.0, 0.0], json!(null)))
}

fn ins_op(t: &'static str, op: &'static str, id: u64) -> Box<dyn FnOnce(&Engine) -> Res + Send> {
    Box::new(move |e| e.insert_op(&full(t), op, VectorId(id), &[1.0, 0.0], json!({"k": id})))
}

fn submit_in_order(e: &Arc<Engine>, tx: &Sender<(&'static str, Res)>, calls: Calls) {
    let hold = e.commit_hold_handle();
    for (name, f) in calls {
        let want = hold.enqueued() + 1;
        spawn_call(e, tx, name, f);
        assert!(
            hold.wait_enqueued(want, SAFEGUARD),
            "HANG: {name} never reached the queue"
        );
    }
}

struct Run {
    dir: PathBuf,
    e: Arc<Engine>,
    res: HashMap<&'static str, Res>,
    len_before: u64,
}

fn scenario(name: &str, mode: Mode, op_culprit: bool, setup: &dyn Fn(&Arc<Engine>)) -> Run {
    let dir = tmp(name);
    drop(Engine::open(&dir, Config::new(2)).unwrap());
    let mut cfg = Config::reopen();
    cfg.max_batch = 4;
    cfg.batch_window = Duration::from_secs(60);
    let e = Arc::new(Engine::open_with_index(&dir, cfg, factory(mode)).unwrap());
    setup(&e);
    let (tx, rx) = channel();
    submit_in_order(
        &e,
        &tx,
        vec![
            ("ack_a", ins_op("a", "ack-a", 1)),
            ("ack_b", ins("b", 2)),
            ("ack_c", ins("c", 3)),
            ("ack_777", ins("a", 777)),
        ],
    );
    let acked = collect(&rx, 4);
    assert!(
        acked.values().all(|r| r.is_ok()),
        "phase 1 must be acked: {acked:?}"
    );
    let hold = e.commit_hold_handle();
    hold.arm();
    submit_in_order(
        &e,
        &tx,
        vec![
            ("before", ins("a", 4)),
            (
                "culprit",
                if op_culprit {
                    ins_op("a", "p-op", 666)
                } else {
                    ins("a", 666)
                },
            ),
            (
                "after_x",
                if op_culprit {
                    ins("b", 5)
                } else {
                    ins_op("b", "b-op", 5)
                },
            ),
            ("after_plain", ins("c", 6)),
        ],
    );
    assert!(
        hold.wait_held(SAFEGUARD),
        "HANG: committer never held the full batch"
    );
    submit_in_order(&e, &tx, vec![("queued", ins("b", 7))]);
    let len_before = wal_len(&dir);
    hold.release();
    let res = collect(&rx, 5);
    Run {
        dir,
        e,
        res,
        len_before,
    }
}

fn code(r: &Res) -> &'static str {
    match r {
        Ok(_) => "OK",
        Err(e) => e.code(),
    }
}

fn msg(r: &Res) -> String {
    r.as_ref().unwrap_err().to_string()
}

fn no_unwind<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::spawn(f)
        .join()
        .unwrap_or_else(|_| panic!("caller thread unwound in {what}"))
}

fn assert_clean(what: &str, s: &str) {
    for bad in [PANIC_MARK, ERR_MARK, "index"] {
        assert!(!s.contains(bad), "{what}: leaked {bad:?} in {s:?}");
    }
}

#[test]
fn s6_1a_panic_in_apply_culprit_others_queued_and_nobody_hangs() {
    for mode in [Mode::Panic, Mode::PanicAfterMutate] {
        let r = scenario("a", mode, true, &|_| {});
        let ctx = format!("{mode:?}: {:?}", r.res);
        assert_eq!(code(&r.res["culprit"]), "WAL_CORRUPT", "{ctx}");
        assert!(
            msg(&r.res["culprit"]).contains(PANIC_MARK),
            "culprit may get the detail"
        );
        for k in ["before", "after_x", "after_plain"] {
            assert_eq!(code(&r.res[k]), "OUTCOME_UNKNOWN", "{k}: {ctx}");
        }
        assert_eq!(code(&r.res["queued"]), "WAL_CORRUPT", "queued: {ctx}");
        assert!(
            r.res
                .values()
                .all(|x| code(x) != "OK" && code(x) != "WAL_CLOSED"),
            "{ctx}"
        );
        let layers = r.e.poison_layer_counts();
        assert_eq!(
            (layers.cheap, layers.append_precheck, layers.wal_failed_flag),
            (0, 0, 1),
            "{ctx}"
        );
        assert!(
            wal_len(&r.dir) > r.len_before,
            "the batch itself was written"
        );
        let dir = r.dir.clone();
        drop(r);
        let e = Engine::open(&dir, Config::reopen()).unwrap();
        assert_eq!(
            e.count(&full("a")).unwrap(),
            4,
            "ack-a, 777, before, culprit"
        );
        assert_eq!(e.count(&full("b")).unwrap(), 2, "ack-b, b-op");
        assert_eq!(e.count(&full("c")).unwrap(), 2, "ack-c, 6");
        let q = e.insert(&full("b"), VectorId(7), &[1.0, 0.0], json!(null));
        assert_eq!(
            q.unwrap(),
            9,
            "queued-behind request left no record and consumed no seq"
        );
    }
}

#[test]
fn s6_1b_after_a_panic_every_tenant_fails_closed_without_unwinding() {
    for mode in [Mode::Panic, Mode::PanicAfterMutate] {
        let r = scenario("b", mode, true, &|_| {});
        let mut seen: Vec<(String, String)> = vec![];
        for t in ["a", "b", "c", "never-used"] {
            let e = r.e.clone();
            let outcomes = no_unwind("reads", move || {
                let s = full(t);
                let mut emitted = 0;
                let mut sink = |_i: atlas_engine::Item| {
                    emitted += 1;
                    Ok::<(), AtlasError>(())
                };
                let snap = e.snapshot(&s, &mut sink);
                let mut sink2 = |_i: atlas_engine::Item| Ok::<(), AtlasError>(());
                let exp = e.export(&s, &mut sink2);
                vec![
                    (
                        "search",
                        e.search(&s, &[1.0, 0.0], 5, Metric::Dot)
                            .map(|h| h.len() as u64),
                    ),
                    (
                        "search_filtered",
                        e.search_filtered(&s, &[1.0, 0.0], 5, Metric::Dot, &|_| true)
                            .map(|h| h.len() as u64),
                    ),
                    ("fetch", e.fetch(&s, VectorId(666)).map(|_| 1)),
                    ("fetch_live", e.fetch(&s, VectorId(1)).map(|_| 1)),
                    ("count", e.count(&s).map(|n| n as u64)),
                    ("snapshot", snap),
                    ("export", exp),
                    ("delete", e.delete(&s, VectorId(1)).map(u64::from)),
                    (
                        "insert",
                        e.insert(&s, VectorId(900), &[1.0, 0.0], json!(null)),
                    ),
                    (
                        "insert_op",
                        e.insert_op(&s, "z", VectorId(901), &[1.0, 0.0], json!(null)),
                    ),
                    ("emitted", Err(AtlasError::Codec(emitted.to_string()))),
                ]
            });
            for (what, res) in outcomes {
                if what == "emitted" {
                    assert_eq!(msg(&res), "CODEC: 0", "{mode:?}/{t}: snapshot emitted data");
                    continue;
                }
                let ctx = format!("{mode:?}/{t}/{what}: {res:?}");
                assert_eq!(code(&res), "WAL_CORRUPT", "{ctx}");
                assert_clean(&ctx, &msg(&res));
                seen.push((what.into(), msg(&res)));
            }
        }
        let first = &seen[0].1;
        assert!(
            seen.iter().all(|(_, m)| m == first),
            "one generic poison message everywhere: {seen:?}"
        );
        assert_eq!(&msg(&r.res["queued"]), first);
    }
}

#[test]
fn s6_1c_mixed_tenant_batch_other_tenants_get_the_generic_error_and_acked_ops_survive() {
    for mode in [Mode::Panic, Mode::PanicAfterMutate, Mode::Err] {
        let r = scenario("c", mode, true, &|_| {});
        let mark = if mode == Mode::Err {
            ERR_MARK
        } else {
            PANIC_MARK
        };
        const GENERIC: &str = "WAL_CORRUPT: engine poisoned by an earlier internal failure; no further operations are accepted until the database is reopened";
        const PLAIN_UNKNOWN: &str = "OUTCOME_UNKNOWN: outcome unknown: the record may be durable; reopen the engine to learn the result (plain insert/delete have no operation id and are not idempotently retryable)";
        assert_eq!(
            code(&r.res["culprit"]),
            "WAL_CORRUPT",
            "{mode:?}: culprit (tenant a, insert_op)"
        );
        assert!(
            msg(&r.res["culprit"]).contains(mark),
            "{mode:?}: culprit (tenant a) may see detail"
        );
        assert_eq!(
            code(&r.res["before"]),
            "OUTCOME_UNKNOWN",
            "{mode:?}: tenant a, same batch, before the panic point"
        );
        assert_eq!(
            msg(&r.res["before"]),
            PLAIN_UNKNOWN,
            "{mode:?}: tenant a plain"
        );
        assert_eq!(
            code(&r.res["after_x"]),
            "OUTCOME_UNKNOWN",
            "{mode:?}: tenant b, same batch"
        );
        assert_eq!(
            msg(&r.res["after_x"]),
            PLAIN_UNKNOWN,
            "{mode:?}: tenant b plain"
        );
        assert_eq!(
            code(&r.res["after_plain"]),
            "OUTCOME_UNKNOWN",
            "{mode:?}: tenant c, same batch"
        );
        assert_eq!(
            msg(&r.res["after_plain"]),
            PLAIN_UNKNOWN,
            "{mode:?}: tenant c plain"
        );
        assert_eq!(
            code(&r.res["queued"]),
            "WAL_CORRUPT",
            "{mode:?}: tenant b, queued behind"
        );
        assert_eq!(
            msg(&r.res["queued"]),
            GENERIC,
            "{mode:?}: tenant b, queued behind, generic"
        );
        for k in ["after_x", "after_plain", "queued"] {
            let m = msg(&r.res[k]);
            assert_clean(&format!("{mode:?}/{k}"), &m);
            assert_ne!(code(&r.res[k]), "WAL_CLOSED");
        }
        let late =
            r.e.insert(&full("b"), VectorId(9), &[1.0, 0.0], json!(null));
        assert_eq!(code(&late), "WAL_CORRUPT");
        assert_eq!(msg(&late), msg(&r.res["queued"]), "{mode:?}");
        let dir = r.dir.clone();
        drop(r);
        let e = Engine::open(&dir, Config::reopen()).unwrap();
        for (t, id) in [
            ("a", 1),
            ("b", 2),
            ("c", 3),
            ("a", 777),
            ("a", 4),
            ("a", 666),
            ("b", 5),
            ("c", 6),
        ] {
            assert!(
                e.fetch(&full(t), VectorId(id)).is_ok(),
                "{mode:?}: {t}/{id} lost"
            );
        }
        assert!(
            e.fetch(&full("b"), VectorId(7)).is_err(),
            "queued record must be absent"
        );
    }
}

#[test]
fn s6_1d_dropping_the_poisoned_engine_completes_and_the_directory_reopens() {
    for mode in [Mode::Panic, Mode::PanicAfterMutate] {
        let r = scenario("d", mode, true, &|_| {});
        let dir = r.dir.clone();
        let (done_tx, done_rx) = channel();
        let e = r.e.clone();
        drop(r);
        std::thread::spawn(move || {
            drop(e);
            let _ = done_tx.send(());
        });
        done_rx.recv_timeout(SAFEGUARD).expect(
            "HANG/TIMEOUT (not a correctness failure): dropping the poisoned engine did not finish",
        );
        let e = Engine::open(&dir, Config::reopen()).expect("reopen immediately after drop");
        assert_eq!(e.count(&full("a")).unwrap(), 4);
    }
}

#[test]
fn s6_1e_reopen_recovers_the_durable_record_and_retry_returns_the_original_seq() {
    for mode in [Mode::Panic, Mode::PanicAfterMutate] {
        let r = scenario("e", mode, true, &|_| {});
        let dir = r.dir.clone();
        drop(r);
        let e = Engine::open(&dir, Config::reopen()).unwrap();
        let a = full("a");
        assert!(
            e.fetch(&a, VectorId(666)).is_ok(),
            "durable culprit record applied after replay"
        );
        let again = e.insert_op(&a, "p-op", VectorId(666), &[1.0, 0.0], json!({"k": 666}));
        assert_eq!(again.unwrap(), 6, "{mode:?}: original seq, executed once");
        assert_eq!(e.count(&a).unwrap(), 4, "no duplicate execution");
    }
}

#[test]
fn s6_1e_an_outcome_unknown_insert_op_retried_after_reopen_returns_its_original_seq() {
    let r = scenario("e2", Mode::Panic, false, &|_| {});
    assert_eq!(code(&r.res["after_x"]), "OUTCOME_UNKNOWN");
    let dir = r.dir.clone();
    drop(r);
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    let again = e.insert_op(
        &full("b"),
        "b-op",
        VectorId(5),
        &[1.0, 0.0],
        json!({"k": 5}),
    );
    assert_eq!(
        again.unwrap(),
        7,
        "same op id: original seq, not a second execution"
    );
    assert_eq!(e.count(&full("b")).unwrap(), 2);
}

#[test]
fn s6_1_panic_in_remove_poisons_the_same_way() {
    let dir = tmp("remove");
    let e = Arc::new(
        Engine::open_with_index(&dir, Config::new(2), factory(Mode::PanicRemove)).unwrap(),
    );
    let a = full("a");
    e.insert(&a, VectorId(777), &[1.0, 0.0], json!(null))
        .unwrap();
    let (e2, a2) = (e.clone(), a.clone());
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(e2.delete(&a2, VectorId(777)));
    });
    let r = rx.recv_timeout(SAFEGUARD).expect(HANG).map(u64::from);
    assert_eq!(code(&r), "WAL_CORRUPT", "{r:?}");
    assert_eq!(
        code(&no_unwind("count", move || e.count(&a).map(|n| n as u64))),
        "WAL_CORRUPT"
    );
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    assert!(
        e.fetch(&full("a"), VectorId(777)).is_err(),
        "durable tombstone applied on replay"
    );
}

#[test]
fn s6_2_index_error_poison_gives_non_culprits_a_generic_message_and_keeps_reads() {
    let r = scenario("idx", Mode::Err, true, &|_| {});
    assert_eq!(code(&r.res["culprit"]), "WAL_CORRUPT");
    assert!(msg(&r.res["culprit"]).contains(ERR_MARK));
    for k in ["before", "after_x", "after_plain"] {
        assert_eq!(code(&r.res[k]), "OUTCOME_UNKNOWN", "{k}");
        assert_clean(k, &msg(&r.res[k]));
    }
    assert_eq!(code(&r.res["queued"]), "WAL_CORRUPT");
    assert_clean("queued", &msg(&r.res["queued"]));
    for t in ["a", "b", "never-used"] {
        let w =
            r.e.insert(&full(t), VectorId(950), &[1.0, 0.0], json!(null));
        assert_eq!(code(&w), "WAL_CORRUPT");
        assert_clean("later write", &msg(&w));
        assert_eq!(msg(&w), msg(&r.res["queued"]));
    }
    assert!(r.e.count(&full("a")).is_ok());
}

fn recording_sink(calls: &Arc<Mutex<Vec<String>>>, boom: bool) -> Arc<dyn Fn(&str) + Send + Sync> {
    let calls = calls.clone();
    Arc::new(move |d: &str| {
        calls.lock().unwrap().push(d.to_string());
        if boom {
            panic!("sink boom");
        }
    })
}

#[test]
fn s6_2_sink_receives_the_full_detail_exactly_once() {
    for (mode, mark) in [
        (Mode::Err, ERR_MARK),
        (Mode::Panic, PANIC_MARK),
        (Mode::PanicAfterMutate, PANIC_MARK),
    ] {
        let calls = Arc::new(Mutex::new(vec![]));
        let c2 = calls.clone();
        let r = scenario("sink", mode, true, &move |e| {
            e.set_diagnostics_sink(recording_sink(&c2, false))
        });
        let got = calls.lock().unwrap().clone();
        assert_eq!(got.len(), 1, "{mode:?}: exactly once, got {got:?}");
        assert!(got[0].contains(mark), "{mode:?}: {:?}", got[0]);
        assert_eq!(
            r.e.poison_detail().as_deref(),
            Some(got[0].as_str()),
            "accessor == sink detail"
        );
        let _ =
            r.e.insert(&full("a"), VectorId(1), &[1.0, 0.0], json!(null));
        assert_eq!(calls.lock().unwrap().len(), 1);
    }
}

#[test]
fn s6_2_a_panicking_sink_does_not_break_or_hang_the_engine() {
    for mode in [Mode::Err, Mode::Panic] {
        let calls = Arc::new(Mutex::new(vec![]));
        let c2 = calls.clone();
        let r = scenario("boom", mode, true, &move |e| {
            e.set_diagnostics_sink(recording_sink(&c2, true))
        });
        assert_eq!(
            calls.lock().unwrap().len(),
            1,
            "{mode:?}: sink ran (and panicked) once"
        );
        assert_eq!(code(&r.res["culprit"]), "WAL_CORRUPT");
        for k in ["before", "after_x", "after_plain"] {
            assert_eq!(code(&r.res[k]), "OUTCOME_UNKNOWN", "{mode:?}/{k}");
        }
        assert_eq!(code(&r.res["queued"]), "WAL_CORRUPT");
        let dir = r.dir.clone();
        let e = r.e.clone();
        drop(r);
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            drop(e);
            let _ = tx.send(());
        });
        rx.recv_timeout(SAFEGUARD)
            .expect("HANG/TIMEOUT: drop after a panicking sink");
        let e = Engine::open(&dir, Config::reopen()).unwrap();
        assert_eq!(e.count(&full("a")).unwrap(), 4);
    }
}

#[test]
fn s6_2_sink_runs_outside_every_engine_lock() {
    let (tx, rx) = channel::<String>();
    let tx = Mutex::new(tx);
    let weak: Arc<Mutex<Weak<Engine>>> = Arc::new(Mutex::new(Weak::new()));
    let w2 = weak.clone();
    let r = scenario("lockfree", Mode::Err, true, &move |e| {
        *w2.lock().unwrap() = Arc::downgrade(e);
        let w3 = w2.clone();
        let tx = tx.lock().unwrap().clone();
        e.set_diagnostics_sink(Arc::new(move |_d: &str| {
            let eng = w3
                .lock()
                .unwrap()
                .upgrade()
                .expect("engine alive during the sink");
            let c = eng
                .count(&full("a"))
                .map(|n| n.to_string())
                .unwrap_or_else(|e| e.code().into());
            let _ = tx.send(c);
        }));
    });
    let seen = rx
        .recv_timeout(SAFEGUARD)
        .expect("HANG: sink re-entering the engine deadlocked");
    assert!(
        seen.parse::<usize>().is_ok(),
        "read inside the sink completed: {seen}"
    );
    drop(r);
}

#[test]
fn s6_3_outcome_unknown_text_depends_on_whether_the_call_has_an_op_id() {
    const PLAIN: &str = "OUTCOME_UNKNOWN: outcome unknown: the record may be durable; reopen the engine to learn the result (plain insert/delete have no operation id and are not idempotently retryable)";
    let r = scenario("text", Mode::Err, false, &|_| {});
    assert_eq!(msg(&r.res["before"]), PLAIN, "plain insert");
    assert_eq!(msg(&r.res["after_plain"]), PLAIN, "plain insert");
    let op = msg(&r.res["after_x"]);
    assert!(
        op.contains("retry the same op id"),
        "insert_op keeps the op-id guidance: {op}"
    );
    assert!(op.contains("reopen"), "{op}");
    assert!(!op.contains("plain insert/delete"), "{op}");
    assert!(!msg(&r.res["before"]).contains("retry the same op id"));
}

#[test]
fn s6_3_plain_delete_gets_the_plain_text() {
    let dir = tmp("deltext");
    drop(Engine::open(&dir, Config::new(2)).unwrap());
    let mut cfg = Config::reopen();
    cfg.max_batch = 2;
    cfg.batch_window = Duration::from_secs(60);
    let e = Arc::new(Engine::open_with_index(&dir, cfg, factory(Mode::Err)).unwrap());
    let (tx, rx) = channel();
    submit_in_order(&e, &tx, vec![("s1", ins("c", 3)), ("s2", ins("c", 4))]);
    collect(&rx, 2);
    submit_in_order(
        &e,
        &tx,
        vec![
            (
                "del",
                Box::new(|e: &Engine| e.delete(&full("c"), VectorId(3)).map(u64::from)),
            ),
            ("culprit", ins("a", 666)),
        ],
    );
    let res = collect(&rx, 2);
    assert_eq!(code(&res["culprit"]), "WAL_CORRUPT");
    assert!(
        msg(&res["del"]).contains("plain insert/delete have no operation id"),
        "{res:?}"
    );
    assert!(!msg(&res["del"]).contains("retry the same op id"));
}

#[test]
fn s6_panic_recovery_requires_unwinding() {
    if !cfg!(panic = "unwind") {
        panic!("S6 panic recovery needs panic=unwind; build embedders without panic=abort");
    }
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.toml")).unwrap();
    assert!(
        !manifest.contains("panic = \"abort\""),
        "workspace profile must not abort on panic"
    );
}

fn open_default(name: &str, mode: Mode) -> (PathBuf, Arc<Engine>) {
    let dir = tmp(name);
    let e = Engine::open_with_index(&dir, Config::new(2), factory(mode)).unwrap();
    (dir, Arc::new(e))
}

fn layers(e: &Engine) -> (usize, usize, usize) {
    let l = e.poison_layer_counts();
    (l.cheap, l.append_precheck, l.wal_failed_flag)
}

#[test]
fn layers_a_request_paused_after_the_cheap_check_is_rejected_by_the_append_precheck() {
    let (dir, e) = open_default("layer-pause", Mode::Err);
    let (reached_tx, reached_rx) = channel();
    let (go_tx, go_rx) = channel::<()>();
    let (reached_tx, go_rx) = (Mutex::new(reached_tx), Mutex::new(go_rx));
    e.set_before_gate_hook(move || {
        if std::thread::current().name() == Some("paused") {
            reached_tx.lock().unwrap().send(()).unwrap();
            let _ = go_rx.lock().unwrap().recv_timeout(SAFEGUARD);
        }
    });
    let e2 = e.clone();
    let t = std::thread::Builder::new()
        .name("paused".into())
        .spawn(move || e2.insert(&full("b"), VectorId(5), &[1.0, 0.0], json!(null)))
        .unwrap();
    reached_rx
        .recv_timeout(SAFEGUARD)
        .expect("HANG: T never reached the pause point");
    assert_eq!(
        layers(&e),
        (0, 0, 0),
        "T passed the cheap check without being rejected"
    );
    let culprit = e.insert(&full("a"), VectorId(666), &[1.0, 0.0], json!(null));
    assert_eq!(code(&culprit), "WAL_CORRUPT");
    let len = wal_len(&dir);
    go_tx.send(()).unwrap();
    let r = t.join().unwrap();
    assert_eq!(code(&r), "WAL_CORRUPT");
    assert_eq!(
        layers(&e),
        (0, 1, 0),
        "attributed to the append pre-check, not the cheap check"
    );
    assert_eq!(wal_len(&dir), len, "T wrote nothing");
}

#[test]
fn layers_with_the_engine_checks_bypassed_the_wal_failed_flag_alone_rejects() {
    let (dir, e) = open_default("layer-bypass", Mode::Err);
    let culprit = e.insert(&full("a"), VectorId(666), &[1.0, 0.0], json!(null));
    assert_eq!(code(&culprit), "WAL_CORRUPT");
    let len = wal_len(&dir);
    e.bypass_engine_poison_check(true);
    let r = e.insert(&full("b"), VectorId(5), &[1.0, 0.0], json!(null));
    assert_eq!(code(&r), "WAL_CORRUPT");
    assert_clean("bypassed", &msg(&r));
    assert_eq!(layers(&e), (0, 0, 1), "only the WAL flag layer rejected");
    let r = e.insert_op(&full("b"), "o", VectorId(6), &[1.0, 0.0], json!(null));
    assert_eq!(code(&r), "WAL_CORRUPT");
    assert_eq!(layers(&e), (0, 0, 2));
    assert_eq!(wal_len(&dir), len, "nothing was written");
    e.bypass_engine_poison_check(false);
    let r = e.insert(&full("b"), VectorId(7), &[1.0, 0.0], json!(null));
    assert_eq!(code(&r), "WAL_CORRUPT");
    assert_eq!(layers(&e), (1, 0, 2), "bypass off: the cheap check is back");
}

#[test]
fn s6_1_panic_during_open_time_replay_is_a_typed_error() {
    let dir = tmp("replay");
    let healthy = Engine::open(&dir, Config::new(2)).unwrap();
    healthy
        .insert(&full("a"), VectorId(666), &[1.0, 0.0], json!(null))
        .unwrap();
    drop(healthy);
    let d2 = dir.clone();
    let r = no_unwind("open", move || {
        Engine::open_with_index(&d2, Config::reopen(), factory(Mode::Panic)).map(|_| ())
    });
    let err = r.unwrap_err();
    assert_eq!(err.code(), "WAL_CORRUPT", "{err}");
    assert!(
        Engine::open(&dir, Config::reopen()).is_ok(),
        "a healthy index still opens it"
    );
}
