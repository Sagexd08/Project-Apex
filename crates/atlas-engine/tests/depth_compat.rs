mod common;
use atlas_core::{Config, VectorId};
use atlas_engine::Engine;
use atlas_storage::{write_meta, Wal};
use common::full;
use serde_json::{json, Value};

fn nested(levels: usize) -> Value {
    (0..levels).fold(json!(0), |v, _| Value::Array(vec![v]))
}

#[test]
fn depth_100_live_and_reopen_and_s2_deep_records_replay_but_new_deep_inserts_fail() {
    let dir = std::env::temp_dir().join(format!("atlas-depthcompat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write_meta(&dir, 2).unwrap();
    {
        let (wal, _) =
            Wal::open(&dir.join("wal.log"), &Config::new(2), Box::new(|_, _| {})).unwrap();
        for levels in 100..=126usize {
            let md = "[".repeat(levels) + "0" + &"]".repeat(levels);
            let rec = format!(
                r#"{{"tenant":"a","id":{},"vector":[1.0,0.0],"metadata":{md}}}"#,
                1000 + levels
            );
            wal.append(rec.into_bytes()).unwrap();
        }
    }
    let a = full("a");
    let e = Engine::open(&dir, Config::reopen()).expect("S2-era deep records replay");
    assert_eq!(e.count(&a).unwrap(), 27);
    e.insert(&a, VectorId(1), &[1.0, 0.0], nested(99)).unwrap();
    for levels in [100usize, 101, 126] {
        let err = e
            .insert(&a, VectorId(2), &[1.0, 0.0], nested(levels))
            .unwrap_err();
        assert_eq!(err.code(), "INVALID_METADATA", "levels {levels}");
    }
    drop(e);
    let e = Engine::open(&dir, Config::reopen()).expect("reopen after depth-100 live insert");
    assert_eq!(e.count(&a).unwrap(), 28);
    assert_eq!(e.fetch(&a, VectorId(1)).unwrap().metadata, nested(99));
    assert_eq!(e.fetch(&a, VectorId(1126)).unwrap().metadata, nested(126));
}
