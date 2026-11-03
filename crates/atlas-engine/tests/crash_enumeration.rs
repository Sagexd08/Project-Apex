//! SIMULATED crash-consistency enumeration. This is NOT power-loss evidence.
//!
//! Every acknowledged write is followed by a record of the WAL length. For every byte-length
//! prefix of the final WAL (i.e. every possible torn tail a crash could leave), a copy of the
//! data dir is truncated to that length and reopened. The recovered set must equal exactly the
//! writes whose acknowledgement point lies inside that prefix: nothing acked is lost, nothing
//! partial is exposed. Test code only; it changes no engine code.
mod common;
use atlas_core::{Config, Durability, Metric, RecoverMode, VectorId};
use atlas_engine::Engine;
use common::single_tenant_full;
use serde_json::json;
use std::path::{Path, PathBuf};

const WRITES: u64 = 10;

fn tmp(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-enum-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

fn ids(eng: &Engine) -> Vec<u64> {
    let mut v: Vec<u64> = eng
        .search(&single_tenant_full(), &[1.0; 4], 100, Metric::Dot)
        .unwrap()
        .iter()
        .map(|h| h.id.0)
        .collect();
    v.sort();
    v
}

fn wal_path(dir: &Path) -> PathBuf {
    dir.join("wal.log")
}

#[test]
fn every_torn_wal_prefix_recovers_exactly_the_acked_prefix() {
    let src = tmp("src");
    let mut cfg = Config::new(4);
    cfg.durability = Durability::SyncPerCommit;
    let eng = Engine::open(&src, cfg).unwrap();
    let base = std::fs::metadata(wal_path(&src))
        .map(|m| m.len())
        .unwrap_or(0);
    let mut acked_at = vec![]; // (wal length after ack, id)
    for id in 1..=WRITES {
        eng.insert(
            &single_tenant_full(),
            VectorId(id),
            &[1.0; 4],
            json!({"i": id}),
        )
        .unwrap();
        acked_at.push((std::fs::metadata(wal_path(&src)).unwrap().len(), id));
    }
    drop(eng);
    let full = std::fs::metadata(wal_path(&src)).unwrap().len();
    assert!(full > base, "writes must have grown the WAL");

    let (mut checked, mut refused) = (0u64, 0u64);
    for len in base..=full {
        let work = tmp("work");
        copy_dir(&src, &work);
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(wal_path(&work))
            .unwrap();
        f.set_len(len).unwrap();
        drop(f);
        let expect: Vec<u64> = acked_at
            .iter()
            .filter(|(l, _)| *l <= len)
            .map(|(_, i)| *i)
            .collect();
        let mut repair = Config::reopen();
        repair.recover_mode = RecoverMode::Repair;
        let eng = match Engine::open(&work, Config::reopen()) {
            Ok(e) => e,
            Err(e) if e.code() == "RECOVERY_REQUIRED" || e.code() == "WAL_CORRUPT" => {
                refused += 1;
                Engine::open(&work, repair)
                    .unwrap_or_else(|e| panic!("len {len}: repair failed {e:?}"))
            }
            Err(e) => panic!("len {len}: unexpected open error {e:?}"),
        };
        assert_eq!(
            ids(&eng),
            expect,
            "torn at byte {len}: recovered set != acked prefix"
        );
        // the recovered engine must accept new writes and keep them across a reopen
        eng.insert(&single_tenant_full(), VectorId(900), &[1.0; 4], json!(null))
            .unwrap();
        drop(eng);
        let again = Engine::open(&work, Config::reopen()).unwrap();
        let mut want = expect.clone();
        want.push(900);
        assert_eq!(
            ids(&again),
            want,
            "torn at byte {len}: post-recovery write lost"
        );
        drop(again);
        let _ = std::fs::remove_dir_all(&work);
        checked += 1;
    }
    println!("CRASH_ENUMERATION checked={checked} offsets, needed_explicit_repair={refused}, acked_writes={WRITES}");
    let _ = std::fs::remove_dir_all(&src);
}
