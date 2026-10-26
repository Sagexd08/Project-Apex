use atlas_core::{Config, Metric, VectorId};
use atlas_engine::Engine;
use serde_json::{json, Value};
use std::sync::Arc;

fn meta_of(eng: &Engine, id: u64) -> Value {
    eng.search(&[0.0; 4], 100, Metric::Dot)
        .unwrap()
        .into_iter()
        .find(|h| h.id == VectorId(id))
        .unwrap()
        .metadata
}

#[test]
fn repeated_id_updates_apply_in_wal_order() {
    for round in 0..60 {
        let dir = std::env::temp_dir().join(format!("atlas-ord-{}-{round}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let eng = Arc::new(Engine::open(&dir, Config::new(4)).unwrap());
        let hs: Vec<_> = (0..8u64)
            .map(|t| {
                let eng = eng.clone();
                std::thread::spawn(move || {
                    (0..5u64)
                        .map(|i| {
                            let tag = json!({"t": t, "i": i});
                            (
                                eng.insert(VectorId(7), &[t as f32; 4], tag.clone())
                                    .unwrap(),
                                tag,
                            )
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let all: Vec<_> = hs.into_iter().flat_map(|h| h.join().unwrap()).collect();
        let expected = all.iter().max_by_key(|(s, _)| *s).unwrap().1.clone();
        assert_eq!(meta_of(&eng, 7), expected, "live index, round {round}");
        drop(Arc::into_inner(eng).unwrap());
        let eng = Engine::open(&dir, Config::new(4)).unwrap();
        assert_eq!(meta_of(&eng, 7), expected, "after reopen, round {round}");
    }
}
