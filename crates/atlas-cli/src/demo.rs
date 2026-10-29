use atlas_cli::StaticKeys;
use atlas_core::{AtlasError, Authenticator, Config, EffectiveScope, Metric, VectorId};
use atlas_engine::{Engine, Hit};
use serde_json::json;

const DIM: usize = 8;
const CLUSTERS: u64 = 5;

fn next(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}

fn centre(c: u64) -> Vec<f32> {
    let mut s = 0x9E37_79B9_7F4A_7C15 ^ (c + 1);
    (0..DIM).map(|_| next(&mut s) * 4.0).collect()
}

fn point(id: u64) -> Vec<f32> {
    let mut s = 0xD1B5_4A32_D192_ED03 ^ (id + 1);
    centre(id % CLUSTERS)
        .into_iter()
        .map(|x| x + next(&mut s) * 0.3)
        .collect()
}

fn ids(hits: &[Hit]) -> Vec<u64> {
    hits.iter().map(|h| h.id.0).collect()
}

fn fill(e: &Engine, s: &EffectiveScope, base: u64, n: u64) -> Result<(), AtlasError> {
    for id in base..base + n {
        e.insert(
            s,
            VectorId(id),
            &point(id),
            json!({ "cluster": id % CLUSTERS }),
        )?;
    }
    Ok(())
}

pub fn run() -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("atlas-demo-{}", std::process::id()));
    let result = walk(&dir).map_err(|e| e.to_string());
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn walk(dir: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let keys = StaticKeys::parse("alice=alice-demo-key,bob=bob-demo-key")?;
    let auth = Authenticator::new(keys);
    let alice = auth.authenticate(b"alice-demo-key")?.effective();
    let bob = auth.authenticate(b"bob-demo-key")?.effective();
    println!("== 1. open a {DIM}-dim database (WAL in {})", dir.display());
    let engine = Engine::open(dir, Config::new(DIM))?;
    fill(&engine, &alice, 1, 500)?;
    fill(&engine, &bob, 10_001, 200)?;
    println!(
        "   alice has {} vectors, bob has {}",
        engine.count(&alice)?,
        engine.count(&bob)?
    );

    let q = centre(3);
    println!("== 2. search: alice's 3 nearest to the cluster-3 centre (cosine)");
    let hits = engine.search(&alice, &q, 3, Metric::Cosine)?;
    for h in &hits {
        println!(
            "   id {:>5} score {:.4} metadata {}",
            h.id.0, h.score, h.metadata
        );
    }
    println!("== 3. tenant isolation: the same query as bob");
    let bob_hits = engine.search(&bob, &q, 5, Metric::Cosine)?;
    let all_bobs = bob_hits.iter().all(|h| h.id.0 >= 10_001);
    println!(
        "   bob sees ids {:?} (all bob's own: {all_bobs})",
        ids(&bob_hits)
    );

    println!("== 4. filtered search: alice, only ids divisible by 50");
    let f = engine.search_filtered(&alice, &q, 3, Metric::Cosine, &|id| id.0.is_multiple_of(50))?;
    println!("   ids {:?}", ids(&f));

    println!("== 5. idempotent write: the same op id twice");
    let op = |id: u64| engine.insert_op(&alice, "order-42", VectorId(id), &point(id), json!({}));
    let (a, b) = (op(900)?, op(900)?);
    println!("   first seq {a}, retry seq {b} (same: {})", a == b);
    let conflict = op(901)
        .err()
        .map_or("accepted?!".to_string(), |e| e.code().to_string());
    println!("   same op id, different payload -> {conflict}");

    let victim = hits[0].id;
    println!("== 6. delete id {}", victim.0);
    println!("   deleted: {}", engine.delete(&alice, victim)?);
    let after = engine.count(&alice)?;
    println!("   alice now has {after} vectors (500 + the step-5 write - 1 deleted)");

    println!("== 7. durability: close, then reopen from the WAL");
    drop(engine);
    let engine = Engine::open(dir, Config::reopen())?;
    let again = engine.count(&alice)?;
    println!(
        "   after reopen alice has {again} vectors (recovered: {})",
        again == after
    );
    println!(
        "   fetch the deleted id -> {:?}",
        engine.fetch(&alice, victim).err().map(|e| e.code())
    );
    if again != after || !all_bobs {
        return Err("demo invariant violated".into());
    }
    Ok(())
}
