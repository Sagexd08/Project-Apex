use atlas_core::{
    AuthContext, Config, EffectiveScope, KeyScope, Metric, PermSet, Principal, TenantId, VectorId,
};
use atlas_engine::Engine;
use atlas_usearch::UsearchIndex;
use serde_json::json;

const DIM: usize = 16;
const N: u64 = 5000;

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

fn vec_for(i: u64) -> Vec<f32> {
    let mut z = i.wrapping_mul(0x9E3779B97F4A7C15) ^ 0x5A5A;
    (0..DIM)
        .map(|_| {
            z = z.wrapping_add(0x9E3779B97F4A7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
            ((x ^ (x >> 31)) % 2001) as f32 / 1000.0 - 1.0
        })
        .collect()
}

#[test]
fn engine_on_usearch_survives_reopen_and_isolates_tenants() {
    let dir = std::env::temp_dir().join(format!("atlas-usearch-eng-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (a, b) = (scope("tenant-a"), scope("tenant-b"));
    let q = vec_for(777);

    let eng = Engine::open_with_index(&dir, Config::new(DIM), UsearchIndex::factory()).unwrap();
    for i in 0..N {
        eng.insert_op(
            &a,
            &format!("op-{i}"),
            VectorId(i),
            &vec_for(i),
            json!({ "i": i }),
        )
        .unwrap();
    }
    eng.insert(&b, VectorId(1), &vec_for(777), json!({ "b": true }))
        .unwrap();
    let before = eng.search(&a, &q, 5, Metric::Cosine).unwrap();
    assert_eq!(
        before[0].id,
        VectorId(777),
        "the exact vector is its own nearest neighbour"
    );
    let tb = eng.search(&b, &q, 5, Metric::Cosine).unwrap();
    assert_eq!(tb.len(), 1, "tenant b sees only its own vector");
    assert_eq!(tb[0].metadata, json!({ "b": true }));
    drop(eng);

    let eng = Engine::open_with_index(&dir, Config::reopen(), UsearchIndex::factory()).unwrap();
    let after = eng.search(&a, &q, 5, Metric::Cosine).unwrap();
    assert_eq!(after[0].id, VectorId(777));
    assert_eq!(eng.count(&a).unwrap(), N as usize);

    for keep in [2u64, 50] {
        let hits = eng
            .search_filtered(&a, &q, 10, Metric::Cosine, &|id| id.0 % keep == 0)
            .unwrap();
        assert!(!hits.is_empty() && hits.iter().all(|h| h.id.0 % keep == 0));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
