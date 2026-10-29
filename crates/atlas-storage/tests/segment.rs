use atlas_storage::segment::{
    decode, encode, latest_segment, list_segments, read_segment, remove_older_than,
    remove_stale_temp, segment_name, write_segment,
};
use std::path::{Path, PathBuf};

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-seg-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn payloads(n: usize) -> Vec<Vec<u8>> {
    (0..n).map(|i| vec![i as u8; i % 7]).collect()
}

#[test]
fn roundtrip_including_empty_segment_and_empty_payloads() {
    let d = dir("rt");
    for (seq, n) in [(0u64, 0usize), (5, 1), (9, 40)] {
        let p = write_segment(&d, seq, &payloads(n)).unwrap();
        let s = read_segment(&p).unwrap();
        assert_eq!((s.wal_seq, s.payloads), (seq, payloads(n)));
    }
}

#[test]
fn newest_segment_wins_and_older_ones_can_be_removed() {
    let d = dir("latest");
    assert!(latest_segment(&d).unwrap().is_none());
    for seq in [3u64, 10, 7] {
        write_segment(&d, seq, &payloads(2)).unwrap();
    }
    assert_eq!(latest_segment(&d).unwrap().unwrap().wal_seq, 10);
    assert_eq!(remove_older_than(&d, 10).unwrap(), 2);
    let left: Vec<u64> = list_segments(&d)
        .unwrap()
        .into_iter()
        .map(|x| x.0)
        .collect();
    assert_eq!(left, vec![10]);
}

#[test]
fn torn_temp_file_is_ignored_and_cleaned_never_loaded() {
    let d = dir("tmp");
    write_segment(&d, 4, &payloads(3)).unwrap();
    let bytes = encode(9, &payloads(3)).unwrap();
    std::fs::write(
        d.join(format!("{}.tmp", segment_name(9))),
        &bytes[..bytes.len() / 2],
    )
    .unwrap();
    assert_eq!(latest_segment(&d).unwrap().unwrap().wal_seq, 4);
    assert_eq!(remove_stale_temp(&d).unwrap(), 1);
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
}

#[test]
fn every_truncation_and_every_single_bit_flip_is_rejected() {
    let bytes = encode(42, &payloads(6)).unwrap();
    let p = Path::new("seg");
    assert!(decode(&bytes, p).is_ok());
    for cut in 0..bytes.len() {
        assert!(decode(&bytes[..cut], p).is_err(), "truncation at {cut}");
    }
    for i in 0..bytes.len() {
        for bit in 0..8 {
            let mut b = bytes.clone();
            b[i] ^= 1 << bit;
            assert!(decode(&b, p).is_err(), "flip byte {i} bit {bit}");
        }
    }
}

#[test]
fn name_and_header_sequence_must_agree() {
    let d = dir("name");
    let bytes = encode(5, &payloads(1)).unwrap();
    std::fs::write(d.join(segment_name(8)), bytes).unwrap();
    assert_eq!(latest_segment(&d).unwrap_err().code(), "WAL_CORRUPT");
}

#[test]
fn hostile_lengths_do_not_allocate_or_panic() {
    let p = Path::new("seg");
    let mut x = 0x1234_5678_9abc_def0u64;
    for _ in 0..3000 {
        let mut b = encode(1, &payloads(4)).unwrap();
        for _ in 0..3 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let at = (x as usize) % b.len();
            b[at] = (x >> 32) as u8;
        }
        let _ = decode(&b, p);
    }
    let mut huge = encode(1, &[]).unwrap();
    huge[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode(&huge, p).is_err());
}
