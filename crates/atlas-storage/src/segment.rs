use crate::sync_dir;
use atlas_core::AtlasError;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"ATLASSEG";
const TRAILER: &[u8; 8] = b"ATLASEND";
const VERSION: u32 = 1;
const HEADER_LEN: usize = 8 + 4 + 8 + 4;
const REC_HDR: usize = 8;
const TRAILER_LEN: usize = 8 + 4;
const PREFIX: &str = "seg-";
const SUFFIX: &str = ".atlas";
const SEQ_DIGITS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub wal_seq: u64,
    pub payloads: Vec<Vec<u8>>,
}

fn corrupt(path: &Path, why: &str) -> AtlasError {
    AtlasError::WalCorrupt(format!("segment {}: {why}", path.display()))
}

pub fn segment_name(wal_seq: u64) -> String {
    format!("{PREFIX}{wal_seq:0width$}{SUFFIX}", width = SEQ_DIGITS)
}

fn parse_name(name: &str) -> Option<u64> {
    let digits = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    if digits.len() != SEQ_DIGITS || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

pub fn encode(wal_seq: u64, payloads: &[Vec<u8>]) -> Result<Vec<u8>, AtlasError> {
    let count = u32::try_from(payloads.len())
        .map_err(|_| AtlasError::CapacityExceeded("segment record count"))?;
    let mut out = Vec::with_capacity(
        HEADER_LEN + payloads.iter().map(|p| REC_HDR + p.len()).sum::<usize>() + TRAILER_LEN,
    );
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&wal_seq.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    for p in payloads {
        let len = u32::try_from(p.len())
            .map_err(|_| AtlasError::CapacityExceeded("segment record length"))?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&crc32fast::hash(p).to_le_bytes());
        out.extend_from_slice(p);
    }
    let crc = crc32fast::hash(&out);
    out.extend_from_slice(TRAILER);
    out.extend_from_slice(&crc.to_le_bytes());
    Ok(out)
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub fn decode(bytes: &[u8], path: &Path) -> Result<Segment, AtlasError> {
    if bytes.len() < HEADER_LEN + TRAILER_LEN {
        return Err(corrupt(path, "shorter than header and trailer"));
    }
    if &bytes[..8] != MAGIC {
        return Err(corrupt(path, "bad magic"));
    }
    let version = u32_at(bytes, 8);
    if version != VERSION {
        return Err(corrupt(path, &format!("unsupported version {version}")));
    }
    let body_end = bytes.len() - TRAILER_LEN;
    if &bytes[body_end..body_end + 8] != TRAILER {
        return Err(corrupt(path, "missing trailer (incomplete write)"));
    }
    if crc32fast::hash(&bytes[..body_end]) != u32_at(bytes, body_end + 8) {
        return Err(corrupt(path, "file checksum mismatch"));
    }
    let wal_seq = u64::from_le_bytes(bytes[12..20].try_into().expect("8 bytes"));
    let count = u32_at(bytes, 20) as usize;
    let mut payloads = Vec::with_capacity(count.min((body_end - HEADER_LEN) / REC_HDR));
    let mut off = HEADER_LEN;
    for i in 0..count {
        if off + REC_HDR > body_end {
            return Err(corrupt(path, &format!("record {i} header past end")));
        }
        let len = u32_at(bytes, off) as usize;
        let crc = u32_at(bytes, off + 4);
        off += REC_HDR;
        if len > body_end - off {
            return Err(corrupt(path, &format!("record {i} length past end")));
        }
        let p = &bytes[off..off + len];
        if crc32fast::hash(p) != crc {
            return Err(corrupt(path, &format!("record {i} checksum mismatch")));
        }
        payloads.push(p.to_vec());
        off += len;
    }
    if off != body_end {
        return Err(corrupt(path, "trailing bytes after last record"));
    }
    Ok(Segment { wal_seq, payloads })
}

pub fn write_segment(
    dir: &Path,
    wal_seq: u64,
    payloads: &[Vec<u8>],
) -> Result<PathBuf, AtlasError> {
    let bytes = encode(wal_seq, payloads)?;
    let final_path = dir.join(segment_name(wal_seq));
    let tmp = dir.join(format!("{}.tmp", segment_name(wal_seq)));
    let mut f = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)?;
    f.write_all(&bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, &final_path)?;
    sync_dir(dir)?;
    Ok(final_path)
}

pub fn read_segment(path: &Path) -> Result<Segment, AtlasError> {
    let bytes = std::fs::read(path)?;
    decode(&bytes, path)
}

pub fn list_segments(dir: &Path) -> Result<Vec<(u64, PathBuf)>, AtlasError> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if let Some(seq) = entry.file_name().to_str().and_then(parse_name) {
            found.push((seq, entry.path()));
        }
    }
    found.sort_by_key(|(seq, _)| *seq);
    Ok(found)
}

pub fn latest_segment(dir: &Path) -> Result<Option<Segment>, AtlasError> {
    let mut all = list_segments(dir)?;
    let Some((seq, path)) = all.pop() else {
        return Ok(None);
    };
    let seg = read_segment(&path)?;
    if seg.wal_seq != seq {
        return Err(corrupt(&path, "file name and header sequence disagree"));
    }
    Ok(Some(seg))
}

pub fn remove_older_than(dir: &Path, keep_seq: u64) -> Result<usize, AtlasError> {
    let mut removed = 0;
    for (seq, path) in list_segments(dir)? {
        if seq < keep_seq {
            std::fs::remove_file(path)?;
            removed += 1;
        }
    }
    if removed > 0 {
        sync_dir(dir)?;
    }
    Ok(removed)
}

pub fn remove_stale_temp(dir: &Path) -> Result<usize, AtlasError> {
    let mut removed = 0;
    let tmp_suffix = format!("{SUFFIX}.tmp");
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with(PREFIX) && name.ends_with(&tmp_suffix) {
            std::fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}
