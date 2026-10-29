use atlas_core::Config;
use atlas_storage::{truncate_prefix, Wal};
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-trunc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.join("wal.log")
}

fn noop() -> atlas_storage::Applier {
    Box::new(|_, _| {})
}

fn write_n(p: &Path, n: u8) {
    let (w, _) = Wal::open(p, &Config::new(1), noop()).unwrap();
    for i in 0..n {
        w.append(vec![i; 3]).unwrap();
    }
}

fn replay(p: &Path) -> Vec<(u64, Vec<u8>)> {
    Wal::open(p, &Config::new(1), noop()).unwrap().1
}

#[test]
fn prefix_is_dropped_suffix_kept_and_numbering_continues() {
    let p = tmp("basic");
    write_n(&p, 10);
    let r = truncate_prefix(&p, 6, 1 << 20).unwrap();
    assert_eq!((r.base_seq, r.dropped, r.kept), (6, 6, 4));
    let seqs: Vec<u64> = replay(&p).iter().map(|x| x.0).collect();
    assert_eq!(seqs, vec![7, 8, 9, 10]);
    {
        let (w, _) = Wal::open(&p, &Config::new(1), noop()).unwrap();
        assert_eq!(w.append(vec![99]).unwrap(), 11);
    }
    let all = replay(&p);
    assert_eq!(
        all.iter().map(|x| x.0).collect::<Vec<_>>(),
        vec![7, 8, 9, 10, 11]
    );
    assert_eq!(all[0].1, vec![6u8; 3]);
    assert_eq!(all[4].1, vec![99]);
}

#[test]
fn truncating_everything_keeps_the_base_so_numbers_never_restart() {
    let p = tmp("all");
    write_n(&p, 4);
    let r = truncate_prefix(&p, 4, 1 << 20).unwrap();
    assert_eq!((r.dropped, r.kept), (4, 0));
    assert!(replay(&p).is_empty());
    let (w, _) = Wal::open(&p, &Config::new(1), noop()).unwrap();
    assert_eq!(w.append(vec![1]).unwrap(), 5);
}

#[test]
fn segment_ahead_of_the_log_still_sets_the_base() {
    let p = tmp("ahead");
    write_n(&p, 3);
    let r = truncate_prefix(&p, 20, 1 << 20).unwrap();
    assert_eq!((r.dropped, r.kept), (3, 0));
    let (w, _) = Wal::open(&p, &Config::new(1), noop()).unwrap();
    assert_eq!(w.append(vec![1]).unwrap(), 21);
}

#[test]
fn truncation_is_idempotent_and_never_moves_the_base_backwards() {
    let p = tmp("idem");
    write_n(&p, 8);
    truncate_prefix(&p, 5, 1 << 20).unwrap();
    let before = std::fs::read(&p).unwrap();
    let r = truncate_prefix(&p, 5, 1 << 20).unwrap();
    assert_eq!((r.base_seq, r.dropped, r.kept), (5, 0, 3));
    let r = truncate_prefix(&p, 2, 1 << 20).unwrap();
    assert_eq!((r.base_seq, r.dropped, r.kept), (5, 0, 3));
    assert_eq!(std::fs::read(&p).unwrap(), before);
}

#[test]
fn leftover_temp_file_from_a_crash_changes_nothing() {
    let p = tmp("crash");
    write_n(&p, 5);
    std::fs::write(p.with_extension("log.truncating"), b"half written").unwrap();
    assert_eq!(replay(&p).len(), 5);
    let r = truncate_prefix(&p, 2, 1 << 20).unwrap();
    assert_eq!(r.kept, 3);
    assert_eq!(replay(&p).len(), 3);
}

#[test]
fn damaged_logs_are_not_rewritten() {
    let p = tmp("damaged");
    write_n(&p, 6);
    let mut b = std::fs::read(&p).unwrap();
    let last = b.len() - 2;
    b[last] ^= 0xFF;
    std::fs::write(&p, &b).unwrap();
    assert!(truncate_prefix(&p, 3, 1 << 20).is_err());
    assert_eq!(std::fs::read(&p).unwrap(), b);

    let q = tmp("tail");
    write_n(&q, 3);
    let mut t = std::fs::read(&q).unwrap();
    t.extend_from_slice(&[1, 2, 3]);
    std::fs::write(&q, &t).unwrap();
    assert!(truncate_prefix(&q, 1, 1 << 20).is_err());
    assert_eq!(std::fs::read(&q).unwrap(), t);
}

#[test]
fn records_missing_after_the_base_fail_closed() {
    let p = tmp("gap");
    write_n(&p, 8);
    truncate_prefix(&p, 3, 1 << 20).unwrap();
    let mut b = std::fs::read(&p).unwrap();
    b[12..20].copy_from_slice(&5u64.to_le_bytes());
    std::fs::write(&p, &b).unwrap();
    let e = Wal::open(&p, &Config::new(1), noop())
        .err()
        .expect("must fail");
    assert_eq!(e.code(), "WAL_CORRUPT");
}

#[test]
fn short_v3_header_and_unknown_versions_are_rejected() {
    let p = tmp("hdr");
    let mut h = b"ATLASWAL".to_vec();
    h.extend_from_slice(&3u32.to_le_bytes());
    h.extend_from_slice(&[0, 0, 0]);
    std::fs::write(&p, &h).unwrap();
    let e = Wal::open(&p, &Config::new(1), noop())
        .err()
        .expect("must fail");
    assert_eq!(e.code(), "WAL_CORRUPT");
    let mut v = b"ATLASWAL".to_vec();
    v.extend_from_slice(&4u32.to_le_bytes());
    std::fs::write(&p, &v).unwrap();
    let e = Wal::open(&p, &Config::new(1), noop())
        .err()
        .expect("must fail");
    assert_eq!(e.code(), "UNSUPPORTED_WAL_VERSION");
}
