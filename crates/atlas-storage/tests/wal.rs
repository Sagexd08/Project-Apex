use atlas_core::{Config, Durability};
use atlas_storage::Wal;
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-wal-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.join("wal.log")
}

fn noop() -> atlas_storage::Applier {
    Box::new(|_, _| {})
}

const HEADER: usize = 12;
const REC_HDR: usize = 28;
const REC: usize = REC_HDR + 10;

fn write_five(p: &Path, cfg: &Config) {
    let (wal, recs) = Wal::open(p, cfg, noop()).unwrap();
    assert!(recs.is_empty());
    for i in 1..=5u8 {
        assert_eq!(wal.append(vec![i; 10]).unwrap(), i as u64);
    }
}

#[test]
fn torn_tail_recovers_to_last_good_record() {
    let p = tmp("torn");
    let cfg = Config::new(1);
    write_five(&p, &cfg);
    let len = std::fs::metadata(&p).unwrap().len();
    assert_eq!(len as usize, HEADER + 5 * REC);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&p)
        .unwrap()
        .set_len(len - 3)
        .unwrap();
    let (wal, recs) = Wal::open(&p, &cfg, noop()).unwrap();
    assert_eq!(recs.len(), 4);
    assert_eq!(
        std::fs::metadata(&p).unwrap().len() as usize,
        HEADER + 4 * REC
    );
    assert_eq!(wal.append(vec![9; 10]).unwrap(), 5);
    drop(wal);
    let (_w, recs) = Wal::open(&p, &cfg, noop()).unwrap();
    assert_eq!(recs.len(), 5);
    assert_eq!(recs[4].1, vec![9; 10]);
}

#[test]
fn complete_final_record_with_bad_crc_fails_closed_and_is_not_discarded() {
    let p = tmp("crc");
    let cfg = Config::new(1);
    write_five(&p, &cfg);
    let mut b = std::fs::read(&p).unwrap();
    b[HEADER + 4 * REC + REC_HDR + 2] ^= 0xFF;
    std::fs::write(&p, &b).unwrap();
    let e = Wal::open(&p, &cfg, noop()).err().expect("must fail closed");
    assert_eq!(e.code(), "RECOVERY_REQUIRED");
    assert_eq!(
        std::fs::read(&p).unwrap(),
        b,
        "WAL must be byte-for-byte unchanged"
    );
}

#[test]
fn repair_of_bad_final_record_keeps_removed_bytes_in_a_bak_sidecar() {
    let p = tmp("repair-bak");
    let mut cfg = Config::new(1);
    write_five(&p, &cfg);
    let mut b = std::fs::read(&p).unwrap();
    b[HEADER + 4 * REC + REC_HDR + 2] ^= 0xFF;
    std::fs::write(&p, &b).unwrap();
    cfg.recover_mode = atlas_core::RecoverMode::Repair;
    let (_w, recs) = Wal::open(&p, &cfg, noop()).unwrap();
    assert_eq!(recs.len(), 4);
    let bak = p.with_file_name("wal.corrupt-5.bak");
    assert_eq!(
        std::fs::read(&bak).unwrap(),
        b[HEADER + 4 * REC..],
        "evidence preserved"
    );
    assert_eq!(
        std::fs::metadata(&p).unwrap().len() as usize,
        HEADER + 4 * REC
    );
}

#[test]
fn flipped_length_in_final_header_fails_closed_with_bytes_identical() {
    let p = tmp("flip-len");
    let cfg = Config::new(1);
    write_five(&p, &cfg);
    let mut b = std::fs::read(&p).unwrap();
    b[HEADER + 4 * REC..HEADER + 4 * REC + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    std::fs::write(&p, &b).unwrap();
    let e = Wal::open(&p, &cfg, noop()).err().expect("must fail closed");
    assert_eq!(e.code(), "RECOVERY_REQUIRED");
    assert_eq!(std::fs::read(&p).unwrap(), b);
}

#[test]
fn header_handling_and_record_limit() {
    let p = tmp("hdr");
    let mut cfg = Config::new(1);
    cfg.max_record_len = 8;
    std::fs::write(&p, b"ATLAS").unwrap();
    let (wal, recs) = Wal::open(&p, &cfg, noop()).unwrap();
    assert!(recs.is_empty());
    assert_eq!(
        wal.append(vec![0; 9]).unwrap_err().code(),
        "RECORD_TOO_LARGE"
    );
    drop(wal);
    std::fs::write(&p, b"NOTAWAL-NOTAWAL-").unwrap();
    assert_eq!(
        Wal::open(&p, &cfg, noop()).err().unwrap().code(),
        "WAL_CORRUPT"
    );
}

#[test]
fn sync_per_commit_and_failed_flush_rollback() {
    let p = tmp("fail");
    let mut cfg = Config::new(1);
    cfg.durability = Durability::SyncPerCommit;
    let (wal, _) = Wal::open(&p, &cfg, noop()).unwrap();
    wal.append(vec![1; 10]).unwrap();
    wal.fail_next_flush_handle()
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        wal.append(vec![2; 10]).unwrap_err().code(),
        "WAL_FLUSH_FAILED"
    );
    assert_eq!(wal.append(vec![3; 10]).unwrap(), 2);
    drop(wal);
    let (_w, recs) = Wal::open(&p, &cfg, noop()).unwrap();
    let got: Vec<_> = recs.iter().map(|r| r.1[0]).collect();
    assert_eq!(got, vec![1, 3]);
}
