//! WAL v2 integrity: protected record header, tail vs mid-log policy, fail-closed cases.
use atlas_core::Config;
use atlas_storage::Wal;
use std::path::{Path, PathBuf};

const HEADER: usize = 12;
const REC_HDR: usize = 28;
const REC: usize = REC_HDR + 10;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-int-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.join("wal.log")
}

fn noop() -> atlas_storage::Applier {
    Box::new(|_, _| {})
}

fn header(version: u32) -> Vec<u8> {
    let mut h = b"ATLASWAL".to_vec();
    h.extend_from_slice(&version.to_le_bytes());
    h
}

/// v2 record: len | !len | seq | flags | payload_crc | header_crc(first 24 bytes) | payload
fn rec_full(len: u32, comp: u32, seq: u64, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut r = len.to_le_bytes().to_vec();
    r.extend_from_slice(&comp.to_le_bytes());
    r.extend_from_slice(&seq.to_le_bytes());
    r.extend_from_slice(&flags.to_le_bytes());
    r.extend_from_slice(&crc32fast::hash(payload).to_le_bytes());
    r.extend_from_slice(&crc32fast::hash(&r).to_le_bytes());
    r.extend_from_slice(payload);
    r
}

fn rec(seq: u64, fill: u8) -> Vec<u8> {
    rec_full(10, !10u32, seq, 0, &[fill; 10])
}

fn file_with(p: &Path, seqs: &[u64]) -> Vec<u8> {
    let mut b = header(2);
    for s in seqs {
        b.extend(rec(*s, *s as u8));
    }
    std::fs::write(p, &b).unwrap();
    b
}

fn open_err(p: &Path) -> atlas_core::AtlasError {
    open_err_cfg(p, &Config::new(1))
}

fn open_err_cfg(p: &Path, cfg: &Config) -> atlas_core::AtlasError {
    Wal::open(p, cfg, noop()).err().expect("open must fail")
}

#[test]
fn unsupported_versions_have_explicit_error_including_the_old_v1_format() {
    for v in [0u32, 1, 3, 99] {
        let p = tmp(&format!("ver{v}"));
        std::fs::write(&p, header(v)).unwrap();
        let e = open_err(&p);
        assert_eq!(e.code(), "UNSUPPORTED_WAL_VERSION", "version {v}");
        assert!(e.to_string().contains(&v.to_string()));
    }
    let p = tmp("magic");
    std::fs::write(&p, b"NOTAWAL!\x02\0\0\0").unwrap();
    assert_eq!(open_err(&p).code(), "WAL_CORRUPT");
}

#[test]
fn mid_log_checksum_failure_fails_closed_and_leaves_file_untouched() {
    let p = tmp("mid-crc");
    let mut b = file_with(&p, &[1, 2, 3, 4, 5]);
    b[HEADER + REC + REC_HDR + 2] ^= 0xFF; // record 2 payload; records 3-5 still valid
    std::fs::write(&p, &b).unwrap();
    assert_eq!(open_err(&p).code(), "WAL_CORRUPT");
    assert_eq!(std::fs::read(&p).unwrap(), b);
}

#[test]
fn mid_log_flipped_length_fails_closed() {
    let p = tmp("mid-len");
    let mut b = file_with(&p, &[1, 2, 3, 4, 5]);
    b[HEADER + REC..HEADER + REC + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    std::fs::write(&p, &b).unwrap();
    assert_eq!(open_err(&p).code(), "WAL_CORRUPT");
    assert_eq!(std::fs::read(&p).unwrap(), b);
}

#[test]
fn header_checksum_covers_len_complement_seq_flags_and_payload_crc() {
    // (offset inside the final record header, label); the checksum itself is not updated.
    for (off, name) in [
        (0usize, "len"),
        (4, "complement"),
        (8, "seq"),
        (16, "flags"),
        (20, "payload_crc"),
    ] {
        let p = tmp(&format!("hdrcrc-{name}"));
        let mut b = file_with(&p, &[1, 2, 3]);
        b[HEADER + 2 * REC + off] ^= 0x01;
        std::fs::write(&p, &b).unwrap();
        assert_eq!(open_err(&p).code(), "RECOVERY_REQUIRED", "{name}");
        assert_eq!(std::fs::read(&p).unwrap(), b, "{name}: bytes unchanged");
    }
}

#[test]
fn inconsistent_length_complement_with_valid_checksum_fails_closed() {
    let p = tmp("complement");
    let mut b = header(2);
    b.extend(rec(1, 1));
    b.extend(rec_full(10, 12345, 2, 0, &[2; 10])); // checksum is valid, pair is not
    std::fs::write(&p, &b).unwrap();
    assert!(matches!(
        open_err(&p).code(),
        "WAL_CORRUPT" | "RECOVERY_REQUIRED"
    ));
    assert_eq!(std::fs::read(&p).unwrap(), b);
}

#[test]
fn length_beyond_configured_max_fails_closed_without_allocating() {
    for len in [11u32, u32::MAX] {
        let p = tmp(&format!("max{len}"));
        let mut b = header(2);
        b.extend(rec(1, 1));
        // valid header checksum, trustworthy length, but above max_record_len
        b.extend(rec_full(len, !len, 2, 0, &[2; 10]));
        std::fs::write(&p, &b).unwrap();
        let mut cfg = Config::new(1);
        cfg.max_record_len = 10;
        assert_eq!(open_err_cfg(&p, &cfg).code(), "WAL_CORRUPT", "len {len}");
        assert_eq!(std::fs::read(&p).unwrap(), b);
    }
}

#[test]
fn duplicate_or_non_monotonic_seq_fails_closed() {
    for (name, seqs) in [
        ("dup", vec![1, 2, 2]),
        ("gap", vec![1, 3]),
        ("back", vec![1, 2, 1]),
        ("start", vec![2, 3]),
    ] {
        let p = tmp(name);
        let b = file_with(&p, &seqs);
        assert_eq!(open_err(&p).code(), "WAL_CORRUPT", "{name}");
        assert_eq!(std::fs::read(&p).unwrap(), b, "{name}: file untouched");
    }
}

#[test]
fn incomplete_final_header_or_payload_is_truncated() {
    // cut < 28: header cut short; cut >= 28: valid header, payload cut short
    for cut in [3usize, 15, 27, 28, 30, 37] {
        let p = tmp(&format!("cut{cut}"));
        let mut b = file_with(&p, &[1, 2]);
        b.extend(&rec(3, 3)[..cut]); // torn third record
        std::fs::write(&p, &b).unwrap();
        let (_w, recs) = Wal::open(&p, &Config::new(1), noop()).unwrap();
        assert_eq!(recs.len(), 2, "cut {cut}");
        assert_eq!(
            std::fs::metadata(&p).unwrap().len() as usize,
            HEADER + 2 * REC
        );
    }
}

#[test]
fn zeroed_header_marker_ends_the_log_even_if_old_records_follow() {
    let p = tmp("zero");
    let mut b = file_with(&p, &[1, 2]);
    b.extend([0u8; REC_HDR]); // invalidation marker
    b.extend(rec(3, 3)); // stale unacked bytes after it
    std::fs::write(&p, &b).unwrap();
    let (_w, recs) = Wal::open(&p, &Config::new(1), noop()).unwrap();
    assert_eq!(recs.len(), 2);
    assert_eq!(
        std::fs::metadata(&p).unwrap().len() as usize,
        HEADER + 2 * REC
    );
}
