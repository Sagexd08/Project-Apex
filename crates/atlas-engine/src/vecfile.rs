use super::Record;
use atlas_core::{AtlasError, VectorId};
use atlas_index::{MappedFile, VectorMap};
use atlas_storage::sync_dir;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"ATLASVEC";
const TRAILER: &[u8; 8] = b"ATLASEND";
const VERSION: u32 = 1;
const PREFIX: &str = "vec-";
const SUFFIX: &str = ".map";
const SEQ_DIGITS: usize = 20;
const TRAILER_LEN: usize = 12;
const FIXED_HEAD: usize = 28;

pub(super) struct Written {
    pub crc: u32,
    pub count: u64,
}

pub(super) struct TenantMap {
    pub name: String,
    pub ids: Vec<VectorId>,
    pub vectors: VectorMap,
    pub meta: Vec<Value>,
}

pub(super) struct Opened {
    pub tenants: Vec<TenantMap>,
    pub file: MappedFile,
    pub crc: u32,
    pub count: u64,
}

fn bad(why: impl std::fmt::Display) -> AtlasError {
    AtlasError::WalCorrupt(format!("vector file: {why}"))
}

pub(super) fn file_name(seq: u64) -> String {
    format!("{PREFIX}{seq:0width$}{SUFFIX}", width = SEQ_DIGITS)
}

fn parse_name(name: &str) -> Option<u64> {
    let digits = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    if digits.len() != SEQ_DIGITS || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn pad8(n: usize) -> usize {
    (8 - n % 8) % 8
}

struct Group<'a> {
    name: &'a str,
    rows: Vec<&'a Record>,
}

fn group(records: &[Record]) -> Vec<Group<'_>> {
    let mut out: Vec<Group<'_>> = Vec::new();
    for r in records {
        match out.last_mut() {
            Some(g) if g.name == r.tenant => g.rows.push(r),
            _ => out.push(Group {
                name: &r.tenant,
                rows: vec![r],
            }),
        }
    }
    out
}

fn meta_bytes(rows: &[&Record]) -> Result<Vec<u8>, AtlasError> {
    let mut out = Vec::new();
    for (slot, r) in rows.iter().enumerate() {
        if r.metadata.is_null() {
            continue;
        }
        let body = serde_json::to_vec(&r.metadata).map_err(|e| AtlasError::Codec(e.to_string()))?;
        let slot = u32::try_from(slot).map_err(|_| bad("too many rows"))?;
        let len = u32::try_from(body.len()).map_err(|_| bad("metadata too long"))?;
        out.extend_from_slice(&slot.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&body);
    }
    Ok(out)
}

struct Sink<W: Write> {
    inner: W,
    crc: crc32fast::Hasher,
    written: usize,
}

impl<W: Write> Sink<W> {
    fn put(&mut self, b: &[u8]) -> std::io::Result<()> {
        self.crc.update(b);
        self.written += b.len();
        self.inner.write_all(b)
    }

    fn align(&mut self) -> std::io::Result<()> {
        let p = pad8(self.written);
        self.put(&[0u8; 8][..p])
    }
}

struct Layout {
    ids_off: usize,
    vec_off: usize,
    meta_off: usize,
}

fn layout(groups: &[Group<'_>], metas: &[Vec<u8>], dim: usize) -> Vec<Layout> {
    let mut head = FIXED_HEAD;
    for g in groups {
        head += 2 + g.name.len() + 8 * 5;
    }
    head += pad8(head);
    let mut cursor = head;
    let mut out = Vec::with_capacity(groups.len());
    for (g, m) in groups.iter().zip(metas) {
        let n = g.rows.len();
        let ids_off = cursor;
        cursor += n * 8;
        cursor += pad8(cursor);
        let vec_off = cursor;
        cursor += n * dim * 4;
        cursor += pad8(cursor);
        let meta_off = cursor;
        cursor += m.len();
        cursor += pad8(cursor);
        out.push(Layout {
            ids_off,
            vec_off,
            meta_off,
        });
    }
    out
}

pub(super) fn write(
    dir: &Path,
    seq: u64,
    dim: usize,
    records: &[Record],
) -> Result<Written, AtlasError> {
    let groups = group(records);
    let metas: Vec<Vec<u8>> = groups
        .iter()
        .map(|g| meta_bytes(&g.rows))
        .collect::<Result<_, _>>()?;
    let lay = layout(&groups, &metas, dim);
    let final_path = dir.join(file_name(seq));
    let tmp = dir.join(format!("{}.tmp", file_name(seq)));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)?;
    let mut sink = Sink {
        inner: std::io::BufWriter::with_capacity(1 << 20, file),
        crc: crc32fast::Hasher::new(),
        written: 0,
    };
    let ntenants = u32::try_from(groups.len()).map_err(|_| bad("too many tenants"))?;
    let dim32 = u32::try_from(dim).map_err(|_| bad("dimension"))?;
    sink.put(MAGIC)?;
    sink.put(&VERSION.to_le_bytes())?;
    sink.put(&seq.to_le_bytes())?;
    sink.put(&dim32.to_le_bytes())?;
    sink.put(&ntenants.to_le_bytes())?;
    for ((g, m), l) in groups.iter().zip(&metas).zip(&lay) {
        let name_len = u16::try_from(g.name.len()).map_err(|_| bad("tenant too long"))?;
        sink.put(&name_len.to_le_bytes())?;
        sink.put(g.name.as_bytes())?;
        for v in [
            g.rows.len() as u64,
            l.ids_off as u64,
            l.vec_off as u64,
            l.meta_off as u64,
            m.len() as u64,
        ] {
            sink.put(&v.to_le_bytes())?;
        }
    }
    sink.align()?;
    for (g, m) in groups.iter().zip(&metas) {
        for r in &g.rows {
            sink.put(&r.id.0.to_le_bytes())?;
        }
        sink.align()?;
        for r in &g.rows {
            if r.vector.len() != dim {
                return Err(bad("row dimension differs from engine dimension"));
            }
            for x in &r.vector {
                sink.put(&x.to_le_bytes())?;
            }
        }
        sink.align()?;
        sink.put(m)?;
        sink.align()?;
    }
    let crc = sink.crc.clone().finalize();
    sink.inner.write_all(TRAILER)?;
    sink.inner.write_all(&crc.to_le_bytes())?;
    let file = sink.inner.into_inner().map_err(|e| e.into_error())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, &final_path)?;
    sync_dir(dir)?;
    Ok(Written {
        crc,
        count: records.len() as u64,
    })
}

fn slice_at(b: &[u8], o: usize, n: usize) -> Result<&[u8], AtlasError> {
    b.get(o..o.checked_add(n).ok_or_else(|| bad("range overflow"))?)
        .ok_or_else(|| bad("header truncated"))
}

fn u16_at(b: &[u8], o: usize) -> Result<u16, AtlasError> {
    Ok(u16::from_le_bytes(
        slice_at(b, o, 2)?.try_into().map_err(bad)?,
    ))
}

fn u32_at(b: &[u8], o: usize) -> Result<u32, AtlasError> {
    Ok(u32::from_le_bytes(
        slice_at(b, o, 4)?.try_into().map_err(bad)?,
    ))
}

fn u64_at(b: &[u8], o: usize) -> Result<u64, AtlasError> {
    Ok(u64::from_le_bytes(
        slice_at(b, o, 8)?.try_into().map_err(bad)?,
    ))
}

fn usize_at(b: &[u8], o: usize) -> Result<usize, AtlasError> {
    usize::try_from(u64_at(b, o)?).map_err(|_| bad("offset too large"))
}

fn read_meta(bytes: &[u8], rows: usize) -> Result<Vec<Value>, AtlasError> {
    let mut out = vec![Value::Null; rows];
    let mut off = 0;
    while off < bytes.len() {
        let slot = u32_at(bytes, off)? as usize;
        let len = u32_at(bytes, off + 4)? as usize;
        let body = slice_at(bytes, off + 8, len)?;
        *out.get_mut(slot)
            .ok_or_else(|| bad("metadata slot out of range"))? =
            serde_json::from_slice(body).map_err(bad)?;
        off += 8 + len;
    }
    Ok(out)
}

fn read_ids(b: &[u8], off: usize, count: usize) -> Result<Vec<VectorId>, AtlasError> {
    let raw = slice_at(
        b,
        off,
        count.checked_mul(8).ok_or_else(|| bad("range overflow"))?,
    )?;
    Ok(raw
        .chunks_exact(8)
        .map(|c| {
            VectorId(u64::from_le_bytes([
                c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7],
            ]))
        })
        .collect())
}

pub(super) fn open(dir: &Path, seq: u64, dim: usize) -> Result<Opened, AtlasError> {
    let file = MappedFile::open(&dir.join(file_name(seq))).map_err(|e| match e {
        AtlasError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
            bad(format!("missing for sequence {seq}"))
        }
        other => other,
    })?;
    let b = file.bytes();
    if b.len() < FIXED_HEAD + TRAILER_LEN {
        return Err(bad("shorter than header and trailer"));
    }
    let body_end = b.len() - TRAILER_LEN;
    if &b[..8] != MAGIC {
        return Err(bad("bad magic"));
    }
    if u32_at(b, 8)? != VERSION {
        return Err(bad("unsupported version"));
    }
    if &b[body_end..body_end + 8] != TRAILER {
        return Err(bad("missing trailer (incomplete write)"));
    }
    if u64_at(b, 12)? != seq {
        return Err(bad("file name and header sequence disagree"));
    }
    if u32_at(b, 20)? as usize != dim {
        return Err(bad("dimension differs from engine dimension"));
    }
    let crc = u32_at(b, body_end + 8)?;
    let ntenants = u32_at(b, 24)? as usize;
    let mut off = FIXED_HEAD;
    let mut tenants = Vec::with_capacity(ntenants.min(1024));
    let mut total = 0u64;
    for _ in 0..ntenants {
        let name_len = u16_at(b, off)? as usize;
        let name = std::str::from_utf8(slice_at(b, off + 2, name_len)?)
            .map_err(bad)?
            .to_string();
        off += 2 + name_len;
        let count = usize_at(b, off)?;
        let ids_off = usize_at(b, off + 8)?;
        let vec_off = usize_at(b, off + 16)?;
        let meta_off = usize_at(b, off + 24)?;
        let meta_len = usize_at(b, off + 32)?;
        off += 40;
        if ids_off.saturating_add(count.saturating_mul(8)) > body_end
            || meta_off.saturating_add(meta_len) > body_end
        {
            return Err(bad("range past end"));
        }
        let floats = count
            .checked_mul(dim)
            .ok_or_else(|| bad("range overflow"))?;
        if vec_off.saturating_add(floats.saturating_mul(4)) > body_end {
            return Err(bad("range past end"));
        }
        tenants.push(TenantMap {
            name,
            ids: read_ids(b, ids_off, count)?,
            vectors: file.floats(vec_off, floats)?,
            meta: read_meta(&b[meta_off..meta_off + meta_len], count)?,
        });
        total += count as u64;
    }
    Ok(Opened {
        tenants,
        file,
        crc,
        count: total,
    })
}

pub(super) fn verify(file: &MappedFile) -> Result<(), AtlasError> {
    let b = file.bytes();
    if b.len() < TRAILER_LEN {
        return Err(bad("shorter than trailer"));
    }
    let body_end = b.len() - TRAILER_LEN;
    if atlas_storage::segment::parallel_crc(&b[..body_end]) != u32_at(b, body_end + 8)? {
        return Err(bad("file checksum mismatch"));
    }
    Ok(())
}

pub(super) fn list(dir: &Path) -> Result<Vec<(u64, PathBuf)>, AtlasError> {
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

pub(super) fn prune(dir: &Path, keep_seq: u64) {
    if let Ok(all) = list(dir) {
        for (seq, path) in all {
            if seq < keep_seq {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

pub(super) fn remove_stale_temp(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let n = e.file_name();
        let n = n.to_string_lossy();
        if n.starts_with(PREFIX) && n.ends_with(".map.tmp") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}
