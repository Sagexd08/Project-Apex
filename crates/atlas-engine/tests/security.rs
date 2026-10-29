mod common;

use atlas_core::{
    AtlasError, AuthContext, Config, EffectiveScope, KeyScope, Metric, PermSet, Principal,
    RecoverMode, TenantId, VectorId,
};
use atlas_engine::{Engine, Hit, Item};
use atlas_storage::{write_meta, Wal};
use common::{full, scope};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn tmp(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-sec-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn open(n: &str) -> (Engine, PathBuf) {
    let dir = tmp(n);
    (Engine::open(&dir, Config::new(2)).unwrap(), dir)
}

fn ids(hits: &[Hit]) -> Vec<u64> {
    let mut v: Vec<u64> = hits.iter().map(|h| h.id.0).collect();
    v.sort();
    v
}

fn put(e: &Engine, s: &EffectiveScope, id: u64, v: [f32; 2], md: Value) {
    e.insert(s, VectorId(id), &v, md).unwrap();
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug)]
enum Op {
    Insert,
    InsertOp,
    Search,
    SearchFiltered,
    Fetch,
    Count,
    Delete,
    Snapshot,
    Export,
}

const OPS: [(Op, PermSet); 9] = [
    (Op::Insert, PermSet::WRITE),
    (Op::InsertOp, PermSet::WRITE),
    (Op::Search, PermSet::SEARCH),
    (Op::SearchFiltered, PermSet::SEARCH),
    (Op::Fetch, PermSet::FETCH),
    (Op::Count, PermSet::COUNT),
    (Op::Delete, PermSet::DELETE),
    (Op::Snapshot, PermSet::SNAPSHOT),
    (Op::Export, PermSet::EXPORT),
];

fn run_op(e: &Engine, s: &EffectiveScope, op: Op) -> Result<(), AtlasError> {
    let q = [1.0, 0.0];
    let mut sink = |_: Item| Ok(());
    match op {
        Op::Insert => e.insert(s, VectorId(900), &q, json!(null)).map(drop),
        Op::InsertOp => e
            .insert_op(s, "op-x", VectorId(901), &q, json!(null))
            .map(drop),
        Op::Search => e.search(s, &q, 5, Metric::Dot).map(drop),
        Op::SearchFiltered => e
            .search_filtered(s, &q, 5, Metric::Dot, &|_| true)
            .map(drop),
        Op::Fetch => e.fetch(s, VectorId(1)).map(drop),
        Op::Count => e.count(s).map(drop),
        Op::Delete => e.delete(s, VectorId(1)).map(drop),
        Op::Snapshot => e.snapshot(s, &mut sink).map(drop),
        Op::Export => e.export(s, &mut sink).map(drop),
    }
}

fn forbidden(r: Result<(), AtlasError>, what: &str) {
    match r {
        Err(e) => assert_eq!(e.code(), "FORBIDDEN", "{what}: wrong error {e:?}"),
        Ok(()) => panic!("{what}: expected FORBIDDEN, got Ok"),
    }
}

fn seeded(n: &str) -> Engine {
    let (e, _) = open(n);
    let a = full("a");
    for i in 1..=3 {
        put(&e, &a, i, [1.0, 0.0], json!({"i": i}));
    }
    e
}

#[test]
fn effective_scope_is_intersection() {
    let t = TenantId::parse("a").unwrap();
    let ctx = AuthContext::for_test(
        Principal {
            tenant: t.clone(),
            perms: PermSet::SEARCH | PermSet::FETCH | PermSet::WRITE,
        },
        KeyScope {
            tenant: t.clone(),
            allowed: PermSet::FETCH | PermSet::WRITE | PermSet::DELETE,
        },
    );
    let s = ctx.effective();
    assert_eq!(s.tenant(), Some(&t));
    assert_eq!(s.perms(), PermSet::FETCH | PermSet::WRITE);
    assert!(!s.allows(PermSet::SEARCH), "key does not allow search");
    assert!(!s.allows(PermSet::DELETE), "principal does not hold delete");
}

#[test]
fn tenant_mismatch_yields_empty_scope() {
    let ctx = AuthContext::for_test(
        Principal {
            tenant: TenantId::parse("a").unwrap(),
            perms: PermSet::ALL,
        },
        KeyScope {
            tenant: TenantId::parse("b").unwrap(),
            allowed: PermSet::ALL,
        },
    );
    let s = ctx.effective();
    assert_eq!(
        s,
        EffectiveScope::default(),
        "mismatch must be the empty scope"
    );
    let e = seeded("mismatch");
    for (op, _) in OPS {
        forbidden(run_op(&e, &s, op), &format!("{op:?}"));
    }
}

#[test]
fn default_scope_denies_everything() {
    let e = seeded("default-scope");
    let s = EffectiveScope::default();
    assert!(s.tenant().is_none());
    assert_eq!(s.perms(), PermSet::EMPTY);
    for (op, _) in OPS {
        forbidden(run_op(&e, &s, op), &format!("{op:?}"));
    }
    assert_eq!(e.len(), 3, "a denied call must not change state");
}

#[test]
fn same_id_two_tenants_no_overwrite() {
    let (e, _) = open("same-id");
    let (a, b) = (full("a"), full("b"));
    put(&e, &a, 1, [1.0, 0.0], json!({"o": "A"}));
    put(&e, &b, 1, [0.0, 1.0], json!({"o": "B"}));
    let fa = e.fetch(&a, VectorId(1)).unwrap();
    let fb = e.fetch(&b, VectorId(1)).unwrap();
    assert_eq!(
        (fa.vector, fa.metadata),
        (vec![1.0, 0.0], json!({"o": "A"}))
    );
    assert_eq!(
        (fb.vector, fb.metadata),
        (vec![0.0, 1.0], json!({"o": "B"}))
    );
    assert_eq!(e.count(&a).unwrap(), 1);
    assert_eq!(e.count(&b).unwrap(), 1);
    let h = e.search(&a, &[1.0, 0.0], 10, Metric::Dot).unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].metadata, json!({"o": "A"}));
}

#[test]
fn search_isolated() {
    let (e, _) = open("search");
    let (a, b) = (full("a"), full("b"));
    for i in 1..=3 {
        put(&e, &a, i, [0.0, 1.0 + i as f32 * 0.1], json!(null));
    }
    for i in 11..=15 {
        put(&e, &b, i, [1.0, 0.0], json!(null));
    }
    let q = [1.0, 0.0];
    for k in [1, 2, 3, 10, 100] {
        let h = e.search(&a, &q, k, Metric::Dot).unwrap();
        assert_eq!(h.len(), k.min(3), "k={k}: want min(k,|A|) hits");
        assert!(
            h.iter().all(|x| (1..=3).contains(&x.id.0)),
            "k={k}: leaked {:?}",
            ids(&h)
        );
    }
    let h = e
        .search_filtered(&a, &q, 10, Metric::Dot, &|_| true)
        .unwrap();
    assert_eq!(ids(&h), vec![1, 2, 3]);
    let hb = e.search(&b, &q, 10, Metric::Dot).unwrap();
    assert_eq!(ids(&hb), vec![11, 12, 13, 14, 15]);
}

#[test]
fn count_isolated() {
    let (e, _) = open("count");
    let (a, b) = (full("a"), full("b"));
    for i in 1..=3 {
        put(&e, &a, i, [1.0, 0.0], json!(null));
    }
    for i in 1..=5 {
        put(&e, &b, i, [1.0, 0.0], json!(null));
    }
    put(&e, &a, 1, [0.5, 0.5], json!(null));
    assert_eq!(e.count(&a).unwrap(), 3);
    assert_eq!(e.count(&b).unwrap(), 5);
}

#[test]
fn fetch_cross_tenant_is_not_found_indistinguishable() {
    let (e, _) = open("fetch-nf");
    let (a, b) = (full("a"), full("b"));
    put(&e, &a, 7, [1.0, 0.0], json!({"s": 1}));
    assert!(e.fetch(&a, VectorId(7)).is_ok());
    let cross = e.fetch(&b, VectorId(7)).unwrap_err();
    let never = e.fetch(&b, VectorId(999)).unwrap_err();
    assert_eq!(cross.code(), "NOT_FOUND");
    assert_eq!(never.code(), "NOT_FOUND");
    assert_eq!(format!("{cross:?}"), format!("{never:?}"));
    assert_eq!(cross.to_string(), never.to_string());
}

#[test]
fn delete_cross_tenant_noop() {
    let (e, _) = open("delete-x");
    let (a, b) = (full("a"), full("b"));
    put(&e, &a, 7, [1.0, 0.0], json!({"s": 1}));
    assert!(
        !e.delete(&b, VectorId(7)).unwrap(),
        "other tenant's id: Ok(false)"
    );
    assert!(
        !e.delete(&b, VectorId(999)).unwrap(),
        "missing id: Ok(false)"
    );
    assert!(e.fetch(&a, VectorId(7)).is_ok(), "A's record must survive");
    assert_eq!(e.count(&a).unwrap(), 1);
    assert!(e.delete(&a, VectorId(7)).unwrap());
    assert!(!e.delete(&a, VectorId(7)).unwrap());
}

#[test]
fn delete_then_search_count_fetch() {
    let (e, _) = open("delete-sc");
    let (a, b) = (full("a"), full("b"));
    for i in 1..=3 {
        put(&e, &a, i, [1.0, 0.0], json!(null));
    }
    put(&e, &b, 2, [1.0, 0.0], json!({"b": 2}));
    assert!(e.delete(&a, VectorId(2)).unwrap());
    let h = e.search(&a, &[1.0, 0.0], 10, Metric::Dot).unwrap();
    assert_eq!(ids(&h), vec![1, 3]);
    assert_eq!(e.count(&a).unwrap(), 2);
    assert_eq!(e.fetch(&a, VectorId(2)).unwrap_err().code(), "NOT_FOUND");
    assert_eq!(e.fetch(&b, VectorId(2)).unwrap().metadata, json!({"b": 2}));
    assert_eq!(e.count(&b).unwrap(), 1);
}

#[test]
fn forged_metadata_filter_cannot_widen() {
    let (e, _) = open("forged");
    let (a, b) = (full("a"), full("b"));
    put(
        &e,
        &a,
        1,
        [1.0, 0.0],
        json!({"tenant": "b", "tenant_id": "b"}),
    );
    put(&e, &a, 2, [1.0, 0.0], json!({"tenant": "b"}));
    put(&e, &b, 11, [1.0, 0.0], json!({"tenant": "a"}));
    let q = [1.0, 0.0];
    let hb = e.search(&b, &q, 10, Metric::Dot).unwrap();
    assert_eq!(
        ids(&hb),
        vec![11],
        "metadata saying tenant=b must not pull A's rows into B"
    );
    let ha = e.search(&a, &q, 10, Metric::Dot).unwrap();
    assert_eq!(ids(&ha), vec![1, 2]);
    let only_b_id = e
        .search_filtered(&a, &q, 10, Metric::Dot, &|id| id == VectorId(11))
        .unwrap();
    assert!(
        only_b_id.is_empty(),
        "a filter naming B's id must not reach B"
    );
    assert_eq!(e.fetch(&b, VectorId(1)).unwrap_err().code(), "NOT_FOUND");
}

#[test]
fn perm_denied_per_surface() {
    let e = seeded("perm-table");
    for (op, need) in OPS {
        let without = scope("a", PermSet::ALL.without(need));
        forbidden(
            run_op(&e, &without, op),
            &format!("{op:?} without {need:?}"),
        );
        let only = scope("a", need);
        if let Err(err) = run_op(&e, &only, op) {
            panic!("{op:?} with only its permission failed: {err:?}");
        }
    }
    let a = full("a");
    assert!(e.fetch(&a, VectorId(900)).is_ok() && e.fetch(&a, VectorId(901)).is_ok());
}

type Sink<'a> = &'a mut dyn FnMut(Item) -> Result<(), AtlasError>;

fn collect(f: impl FnOnce(Sink) -> Result<u64, AtlasError>) -> (Vec<Item>, u64) {
    let mut items = Vec::new();
    let n = f(&mut |i: Item| {
        items.push(i);
        Ok(())
    })
    .unwrap();
    (items, n)
}

#[test]
fn snapshot_export_contain_only_tenant() {
    let (e, _) = open("snap-export");
    let (a, b) = (full("a"), full("b"));
    for i in 1..=3 {
        put(&e, &a, i, [1.0, 0.0], json!({"secret": "a-ok"}));
    }
    for i in 1..=5 {
        put(&e, &b, i + 10, [1.0, 0.0], json!({"secret": "b-LEAK"}));
    }
    let want = e.count(&a).unwrap();
    assert_eq!(want, 3);
    let (snap, sn) = collect(|s| e.snapshot(&a, s));
    let (exp, en) = collect(|s| e.export(&a, s));
    for (name, items, n) in [("snapshot", snap, sn), ("export", exp, en)] {
        let mut got: Vec<u64> = items.iter().map(|i| i.id.0).collect();
        got.sort();
        assert_eq!(got, vec![1, 2, 3], "{name} ids");
        assert_eq!(items.len(), want, "{name} len == count(A)");
        assert_eq!(n as usize, want, "{name} reported count");
        let md: Vec<&Value> = items.iter().map(|i| &i.metadata).collect();
        let text = serde_json::to_string(&md).unwrap();
        assert!(!text.contains("LEAK"), "{name} leaked B metadata: {text}");
    }
}

fn seed_scenario(e: &Engine) {
    let (a, b) = (full("a"), full("b"));
    for i in 1..=3 {
        put(e, &a, i, [0.0, 1.0], json!({"o": "A", "i": i}));
    }
    put(e, &b, 1, [1.0, 0.0], json!({"o": "B", "i": 1}));
    for i in 11..=15 {
        put(e, &b, i, [1.0, 0.0], json!({"o": "B", "i": i}));
    }
    assert!(e.delete(&a, VectorId(2)).unwrap());
}

fn check_scenario(e: &Engine, stage: &str) {
    let (a, b) = (full("a"), full("b"));
    assert_eq!(e.count(&a).unwrap(), 2, "{stage}: count A");
    assert_eq!(e.count(&b).unwrap(), 6, "{stage}: count B");
    let h = e.search(&a, &[1.0, 0.0], 100, Metric::Dot).unwrap();
    assert_eq!(ids(&h), vec![1, 3], "{stage}: search A");
    let fa = e.fetch(&a, VectorId(1)).unwrap();
    assert_eq!(fa.metadata["o"], "A", "{stage}");
    let fb = e.fetch(&b, VectorId(1)).unwrap();
    assert_eq!(fb.metadata["o"], "B", "{stage}");
    let deleted = e.fetch(&a, VectorId(2)).unwrap_err();
    assert_eq!(deleted.code(), "NOT_FOUND", "{stage}: deleted");
    let cross = e.fetch(&a, VectorId(11)).unwrap_err();
    assert_eq!(cross.code(), "NOT_FOUND", "{stage}: cross");
    assert!(
        !e.delete(&a, VectorId(11)).unwrap(),
        "{stage}: cross delete"
    );
}

#[test]
fn recovery_preserves_isolation() {
    let (e, dir) = open("recovery");
    seed_scenario(&e);
    check_scenario(&e, "before reopen");
    drop(e);
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    check_scenario(&e, "after reopen");
}

fn wal_with(dir: &Path, payload: &[u8]) {
    std::fs::create_dir_all(dir).unwrap();
    write_meta(dir, 2).unwrap();
    let (wal, _) = Wal::open(&dir.join("wal.log"), &Config::new(2), Box::new(|_, _| {})).unwrap();
    wal.append(payload.to_vec()).unwrap();
}

#[test]
fn recovery_rejects_tenantless_record() {
    let rec = |t: &str| format!(r#"{{{t}"id":1,"vector":[1.0,0.0],"metadata":null}}"#);
    let bad = [
        rec(""),
        rec(r#""tenant":null,"#),
        rec(r#""tenant":"","#),
        rec(r#""tenant":5,"#),
        rec(r#""tenant":"bad tenant!","#),
        rec(r#""tenant":"a/b","#),
        rec(&format!(r#""tenant":"{}","#, "x".repeat(65))),
        r#"{"id":1,"deleted":true}"#.to_string(),
        r#"{"tenant":"a.b","id":1,"deleted":true}"#.to_string(),
    ];
    for (n, payload) in bad.iter().enumerate() {
        for mode in [RecoverMode::Normal, RecoverMode::Repair] {
            let dir = tmp(&format!("tenantless-{n}-{mode:?}"));
            wal_with(&dir, payload.as_bytes());
            let mut cfg = Config::reopen();
            cfg.recover_mode = mode;
            let err = match Engine::open(&dir, cfg) {
                Err(e) => e,
                Ok(_) => panic!("{mode:?} open accepted bad record: {payload}"),
            };
            assert_eq!(err.code(), "WAL_CORRUPT", "{mode:?} {payload}");
        }
    }
    let dir = tmp("tenantful-control");
    wal_with(&dir, rec(r#""tenant":"t1","#).as_bytes());
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(e.count(&full("t1")).unwrap(), 1);
    assert_eq!(e.count(&full("other")).unwrap(), 0);
}

#[test]
fn op_id_scoped_per_tenant() {
    let (e, dir) = open("op-id");
    let (a, b) = (full("a"), full("b"));
    let (xa, yb) = (json!({"x": 1}), json!({"y": 1}));
    let sa = e.insert_op(&a, "op1", VectorId(1), &[1.0, 0.0], xa.clone());
    let sa = sa.unwrap();
    let again = e.insert_op(&a, "op1", VectorId(1), &[1.0, 0.0], xa.clone());
    assert_eq!(sa, again.unwrap(), "same-tenant retry returns original seq");
    let sb = e
        .insert_op(&b, "op1", VectorId(1), &[0.0, 1.0], yb.clone())
        .expect("op ids are per tenant");
    assert_ne!(sa, sb);
    let sb2 = e.insert_op(&b, "op1", VectorId(1), &[0.0, 1.0], yb.clone());
    assert_eq!(sb, sb2.unwrap());
    assert_eq!(e.fetch(&a, VectorId(1)).unwrap().metadata, xa);
    assert_eq!(e.fetch(&b, VectorId(1)).unwrap().metadata, yb);
    drop(e);
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    let ra = e.insert_op(&a, "op1", VectorId(1), &[1.0, 0.0], xa.clone());
    assert_eq!(ra.unwrap(), sa, "after reopen, A");
    let rb = e.insert_op(&b, "op1", VectorId(1), &[0.0, 1.0], yb.clone());
    assert_eq!(rb.unwrap(), sb, "after reopen, B");
}

#[test]
fn concurrent_keys_isolated() {
    const THREADS: u64 = 8;
    const PER: u64 = 25;
    let (e, _) = open("concurrent");
    std::thread::scope(|sc| {
        for t in 0..THREADS {
            let e = &e;
            sc.spawn(move || {
                let name = if t % 2 == 0 { "a" } else { "b" };
                let s = full(name);
                for i in 0..PER {
                    let id = VectorId(1000 * t + i);
                    e.insert(&s, id, &[1.0, t as f32], json!({"t": t})).unwrap();
                    let hits = e.search(&s, &[1.0, 0.0], 10_000, Metric::Dot).unwrap();
                    for h in &hits {
                        assert_eq!((h.id.0 / 1000) % 2, t % 2, "{name} saw id {}", h.id.0);
                    }
                    let c = e.count(&s).unwrap() as u64;
                    assert!(c > i && c <= (THREADS / 2) * PER, "tenant {name} count {c}");
                }
            });
        }
    });
    let want = (THREADS / 2 * PER) as usize;
    assert_eq!(e.count(&full("a")).unwrap(), want);
    assert_eq!(e.count(&full("b")).unwrap(), want);
}

#[test]
fn export_mid_stream_failure_is_not_a_complete_export() {
    let (e, _) = open("mid-stream");
    let a = full("a");
    for i in 1..=5 {
        put(&e, &a, i, [1.0, 0.0], json!(null));
    }
    for name in ["snapshot", "export"] {
        let mut seen = 0;
        let mut sink = |_: Item| {
            seen += 1;
            if seen == 3 {
                Err(AtlasError::Io(std::io::Error::other("disk full")))
            } else {
                Ok(())
            }
        };
        let r = if name == "snapshot" {
            e.snapshot(&a, &mut sink)
        } else {
            e.export(&a, &mut sink)
        };
        match r {
            Ok(n) => panic!("{name}: failed stream reported complete Ok({n})"),
            Err(AtlasError::Incomplete { emitted, .. }) => {
                assert_eq!(
                    emitted, 2,
                    "{name}: typed count of records emitted before failure"
                );
            }
            Err(err) => panic!("{name}: expected Incomplete, got {err:?}"),
        }
        assert_eq!(seen, 3, "{name}: stream must stop at the failing item");
    }
    let (items, n) = collect(|s| e.export(&a, s));
    assert_eq!((items.len(), n), (5, 5));
}

#[test]
fn export_unauthorized_emits_nothing() {
    let e = seeded("export-unauth");
    let no_export = scope("a", PermSet::ALL.without(PermSet::EXPORT));
    let no_snap = scope("a", PermSet::ALL.without(PermSet::SNAPSHOT));
    for (name, s) in [
        ("no-export", no_export),
        ("default", EffectiveScope::default()),
    ] {
        let mut emitted = 0;
        let r = e.export(&s, &mut |_| {
            emitted += 1;
            Ok(())
        });
        assert_eq!(r.unwrap_err().code(), "FORBIDDEN", "{name}");
        assert_eq!(emitted, 0, "{name}: records emitted before authorization");
    }
    let mut emitted = 0;
    let r = e.snapshot(&no_snap, &mut |_| {
        emitted += 1;
        Ok(())
    });
    assert_eq!(r.unwrap_err().code(), "FORBIDDEN");
    assert_eq!(emitted, 0);
}
