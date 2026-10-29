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
    b[HEADER + REC + REC_HDR + 2] ^= 0xFF;
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
    b.extend(rec_full(10, 12345, 2, 0, &[2; 10]));
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
    for cut in [3usize, 15, 27, 28, 30, 37] {
        let p = tmp(&format!("cut{cut}"));
        let mut b = file_with(&p, &[1, 2]);
        b.extend(&rec(3, 3)[..cut]);
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
    b.extend([0u8; REC_HDR]);
    b.extend(rec(3, 3));
    std::fs::write(&p, &b).unwrap();
    let (_w, recs) = Wal::open(&p, &Config::new(1), noop()).unwrap();
    assert_eq!(recs.len(), 2);
    assert_eq!(
        std::fs::metadata(&p).unwrap().len() as usize,
        HEADER + 2 * REC
    );
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn mutate(base: &[u8], r: &mut Lcg) -> Vec<u8> {
    let mut b = base.to_vec();
    for _ in 0..=(r.next() % 3) {
        let at = (r.next() as usize) % b.len().max(1);
        match r.next() % 5 {
            0 => b[at] ^= 1 << (r.next() % 8),
            1 => b[at] = r.next() as u8,
            2 => b.truncate(at),
            3 => b.extend((0..=(r.next() % 40)).map(|_| r.next() as u8)),
            _ => {
                let end = (at + (r.next() as usize % 64)).min(b.len());
                b.drain(at..end);
            }
        }
        if b.is_empty() {
            break;
        }
    }
    b
}

#[test]
fn seeded_random_corruption_never_panics_and_never_yields_a_bad_log() {
    let base = {
        let p = tmp("fuzz-base");
        file_with(&p, &[1, 2, 3, 4, 5, 6])
    };
    let mut r = Lcg(0xA71A5);
    let (mut ok, mut err) = (0u32, 0u32);
    for i in 0..1500u32 {
        let p = tmp(&format!("fuzz{}", i % 8));
        let bytes = if i % 10 == 0 {
            let mut v = header(2);
            v.extend((0..(r.next() % 300)).map(|_| r.next() as u8));
            v
        } else {
            mutate(&base, &mut r)
        };
        std::fs::write(&p, &bytes).unwrap();
        match Wal::open(&p, &Config::new(1), noop()) {
            Ok((_w, recs)) => {
                ok += 1;
                for (n, rec) in recs.iter().enumerate() {
                    assert_eq!(rec.0, n as u64 + 1, "iteration {i}: non-contiguous seq");
                }
            }
            Err(e) => {
                err += 1;
                assert!(
                    matches!(
                        e.code(),
                        "WAL_CORRUPT" | "UNSUPPORTED_WAL_VERSION" | "RECOVERY_REQUIRED"
                    ),
                    "iteration {i}: unexpected code {}",
                    e.code()
                );
            }
        }
    }
    assert!(
        ok > 50 && err > 50,
        "mutation mix too narrow: ok={ok} err={err}"
    );
}
