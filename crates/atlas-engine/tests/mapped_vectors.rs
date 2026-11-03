use atlas_core::{
    AuthContext, Config, EffectiveScope, KeyScope, Metric, PermSet, Principal, TenantId, VectorId,
};
use atlas_engine::Engine;
use serde_json::json;
use std::path::PathBuf;

const DIM: usize = 8;

fn scope(tenant: &str) -> EffectiveScope {
    let t = TenantId::parse(tenant).unwrap();
    AuthContext::for_test(
        Principal {
            tenant: t.clone(),
            perms: PermSet::ALL,
        },
        KeyScope {
            tenant: t,
            allowed: PermSet::ALL,
        },
    )
    .effective()
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-mapped-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn vec_of(seed: u64) -> Vec<f32> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..DIM)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 40) as f32 / 8_388_608.0 - 1.0
        })
        .collect()
}

fn fill(e: &Engine) {
    for (tenant, base) in [("alice", 0u64), ("bob", 10_000)] {
        let s = scope(tenant);
        for i in 0..200u64 {
            let meta = if i % 7 == 0 {
                json!({ "k": i })
            } else {
                serde_json::Value::Null
            };
            e.insert(&s, VectorId(base + i), &vec_of(base + i), meta)
                .unwrap();
        }
    }
}

fn sidecars(dir: &PathBuf) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "map"))
        .collect()
}

#[test]
fn mapped_reopen_matches_original_and_keeps_tenants_apart() {
    let dir = tmp("reopen");
    let q = vec_of(5);
    let before = {
        let e = Engine::open(&dir, Config::new(DIM)).unwrap();
        fill(&e);
        let hits = e.search(&scope("alice"), &q, 10, Metric::Cosine).unwrap();
        e.checkpoint().unwrap();
        hits
    };
    assert_eq!(sidecars(&dir).len(), 1);
    std::env::set_var("ATLAS_EAGER_VERIFY", "1");
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    let after = e.search(&scope("alice"), &q, 10, Metric::Cosine).unwrap();
    assert_eq!(before, after);
    assert_eq!(e.count(&scope("alice")).unwrap(), 200);
    assert_eq!(e.count(&scope("bob")).unwrap(), 200);
    let bob_hits = e.search(&scope("bob"), &q, 400, Metric::Cosine).unwrap();
    assert!(bob_hits.iter().all(|h| h.id.0 >= 10_000));
    let fetched = e.fetch(&scope("alice"), VectorId(7)).unwrap();
    assert_eq!(fetched.metadata, json!({ "k": 7 }));
    assert_eq!(fetched.vector, vec_of(7));
    drop(e);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mutations_after_mapped_open_are_correct_and_durable() {
    let dir = tmp("mutate");
    {
        let e = Engine::open(&dir, Config::new(DIM)).unwrap();
        fill(&e);
        e.checkpoint().unwrap();
    }
    std::env::set_var("ATLAS_EAGER_VERIFY", "1");
    {
        let e = Engine::open(&dir, Config::reopen()).unwrap();
        let s = scope("alice");
        e.insert(&s, VectorId(900), &vec_of(900), json!({ "new": true }))
            .unwrap();
        e.insert(&s, VectorId(3), &vec_of(777), serde_json::Value::Null)
            .unwrap();
        assert!(e.delete(&s, VectorId(4)).unwrap());
        assert_eq!(e.count(&s).unwrap(), 200);
    }
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    let s = scope("alice");
    assert_eq!(e.count(&s).unwrap(), 200);
    assert_eq!(e.fetch(&s, VectorId(900)).unwrap().vector, vec_of(900));
    assert_eq!(e.fetch(&s, VectorId(3)).unwrap().vector, vec_of(777));
    assert!(e.fetch(&s, VectorId(4)).is_err());
    assert_eq!(e.count(&scope("bob")).unwrap(), 200);
    e.checkpoint().unwrap();
    drop(e);
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(e.count(&scope("alice")).unwrap(), 200);
    assert_eq!(
        e.fetch(&scope("alice"), VectorId(3)).unwrap().vector,
        vec_of(777)
    );
    drop(e);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn corrupt_vector_file_is_rejected_when_verified() {
    let dir = tmp("corrupt");
    {
        let e = Engine::open(&dir, Config::new(DIM)).unwrap();
        fill(&e);
        e.checkpoint().unwrap();
    }
    let path = sidecars(&dir).pop().unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0x55;
    std::fs::write(&path, &bytes).unwrap();
    std::env::set_var("ATLAS_EAGER_VERIFY", "1");
    assert!(Engine::open(&dir, Config::reopen()).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_vector_file_fails_closed() {
    let dir = tmp("missing");
    {
        let e = Engine::open(&dir, Config::new(DIM)).unwrap();
        fill(&e);
        e.checkpoint().unwrap();
    }
    for p in sidecars(&dir) {
        std::fs::remove_file(p).unwrap();
    }
    assert!(Engine::open(&dir, Config::reopen()).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn wal_tail_after_checkpoint_replays_on_top_of_mapped_vectors() {
    let dir = tmp("tail");
    {
        let e = Engine::open(&dir, Config::new(DIM)).unwrap();
        fill(&e);
        e.checkpoint().unwrap();
        e.insert(
            &scope("bob"),
            VectorId(20_000),
            &vec_of(20_000),
            json!({ "tail": 1 }),
        )
        .unwrap();
    }
    std::env::set_var("ATLAS_EAGER_VERIFY", "1");
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(e.count(&scope("bob")).unwrap(), 201);
    assert_eq!(e.count(&scope("alice")).unwrap(), 200);
    assert_eq!(
        e.fetch(&scope("bob"), VectorId(20_000)).unwrap().metadata,
        json!({ "tail": 1 })
    );
    drop(e);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn legacy_segment_is_converted_by_the_next_checkpoint() {
    let dir = tmp("legacy");
    let seq = {
        let e = Engine::open(&dir, Config::new(DIM)).unwrap();
        fill(&e);
        e.checkpoint().unwrap()
    };
    for p in sidecars(&dir) {
        std::fs::remove_file(p).unwrap();
    }
    let mut payloads: Vec<Vec<u8>> = Vec::new();
    for (tenant, base) in [("alice", 0u64), ("bob", 10_000)] {
        for i in 0..200u64 {
            let meta = if i % 7 == 0 {
                json!({ "k": i })
            } else {
                serde_json::Value::Null
            };
            let rec = json!({
                "tenant": tenant, "id": base + i, "vector": vec_of(base + i),
                "metadata": meta, "deleted": false, "op_id": null
            });
            let mut p = vec![1u8];
            p.extend(serde_json::to_vec(&rec).unwrap());
            payloads.push(p);
        }
    }
    atlas_storage::segment::write_segment(&dir, seq, &payloads).unwrap();
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(e.count(&scope("alice")).unwrap(), 200);
    assert_eq!(e.checkpointed_seq(), 0);
    assert!(sidecars(&dir).is_empty());
    e.checkpoint().unwrap();
    assert_eq!(sidecars(&dir).len(), 1);
    assert_eq!(e.checkpointed_seq(), seq);
    drop(e);
    std::env::set_var("ATLAS_EAGER_VERIFY", "1");
    let e = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(e.count(&scope("bob")).unwrap(), 200);
    assert_eq!(
        e.fetch(&scope("alice"), VectorId(7)).unwrap().vector,
        vec_of(7)
    );
    drop(e);
    let _ = std::fs::remove_dir_all(dir);
}
