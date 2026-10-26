use atlas_core::{Config, Metric, VectorId};
use atlas_engine::Engine;
use serde_json::json;

fn tmp(n: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-dim-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn reopen_with_other_dimension_is_dimension_mismatch_not_wal_corrupt() {
    let dir = tmp("mismatch");
    let eng = Engine::open(&dir, Config::new(768)).unwrap();
    eng.insert(VectorId(1), &[0.5; 768], json!(null)).unwrap();
    drop(eng);
    assert_eq!(Engine::open(&dir, Config::new(768)).unwrap().len(), 1);
    let e = Engine::open(&dir, Config::new(384)).err().unwrap();
    assert_eq!(e.code(), "DIMENSION_MISMATCH");
}

#[test]
fn empty_db_dimension_is_persisted_too() {
    let dir = tmp("empty");
    drop(Engine::open(&dir, Config::new(16)).unwrap());
    let e = Engine::open(&dir, Config::new(8)).err().unwrap();
    assert_eq!(e.code(), "DIMENSION_MISMATCH");
}

#[test]
fn reopen_recovers_dimension_without_caller_supplying_it() {
    let dir = tmp("recover");
    let eng = Engine::open(&dir, Config::new(5)).unwrap();
    eng.insert(VectorId(1), &[1.0; 5], json!(null)).unwrap();
    drop(eng);
    let eng = Engine::open(&dir, Config::reopen()).unwrap();
    assert_eq!(eng.dim(), 5);
    assert_eq!(eng.search(&[1.0; 5], 1, Metric::Dot).unwrap().len(), 1);
}

#[test]
fn create_requires_dimension_and_bad_meta_is_detected() {
    let dir = tmp("create");
    assert_eq!(
        Engine::open(&dir, Config::reopen()).err().unwrap().code(),
        "INVALID_CONFIG"
    );
    assert_eq!(
        Engine::open(&dir, Config::new(0)).err().unwrap().code(),
        "INVALID_CONFIG"
    );
    drop(Engine::open(&dir, Config::new(4)).unwrap());
    let meta = dir.join("meta.bin");
    let good = std::fs::read(&meta).unwrap();
    let open_code = || Engine::open(&dir, Config::reopen()).err().unwrap().code();
    let mut b = good.clone();
    b[14] ^= 1; // dimension byte: checksum mismatch
    std::fs::write(&meta, &b).unwrap();
    assert_eq!(open_code(), "META_CORRUPT");
    std::fs::write(&meta, &good[..10]).unwrap();
    assert_eq!(open_code(), "META_CORRUPT");
    let mut b = good.clone();
    b[8] = 9; // version
    std::fs::write(&meta, &b).unwrap();
    assert_eq!(open_code(), "META_CORRUPT");
    // wal without meta (legacy / damaged) is refused rather than guessed
    std::fs::remove_file(&meta).unwrap();
    assert_eq!(open_code(), "INVALID_CONFIG");
}
