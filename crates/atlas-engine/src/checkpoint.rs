use super::{vecfile, OpEntry, OpMap, Parts, Record};
use atlas_core::{AtlasError, TenantId};
use atlas_index::MappedFile;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

const TAG_ITEM: u8 = 1;
const TAG_OP: u8 = 2;
const TAG_ITEM_BIN: u8 = 3;
const TAG_MAP: u8 = 4;

pub(super) fn map_record(w: &vecfile::Written) -> Vec<u8> {
    let mut body = Vec::with_capacity(12);
    body.extend_from_slice(&w.crc.to_le_bytes());
    body.extend_from_slice(&w.count.to_le_bytes());
    tagged(TAG_MAP, body)
}

fn load_map(
    parts: &mut Parts,
    dir: &Path,
    seq: u64,
    body: &[u8],
) -> Result<MappedFile, AtlasError> {
    if body.len() != 12 {
        return Err(bad("map reference has the wrong length"));
    }
    let crc = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    let count = u64::from_le_bytes([
        body[4], body[5], body[6], body[7], body[8], body[9], body[10], body[11],
    ]);
    let opened = vecfile::open(dir, seq, parts.dim)?;
    if opened.crc != crc || opened.count != count {
        return Err(bad("vector file does not match its segment reference"));
    }
    for t in opened.tenants {
        let tenant = TenantId::parse(&t.name).map_err(bad)?;
        if parts.map.contains_key(&tenant) {
            return Err(bad("tenant appears twice"));
        }
        let mut ix = (parts.make)(parts.dim);
        let (mut ids, mut meta) = (t.ids, t.meta);
        if ix.adopt(&mut ids, t.vectors.clone(), &mut meta)? {
            parts.map.insert(tenant, ix);
            continue;
        }
        ix.reserve(ids.len());
        let dim = parts.dim;
        for ((id, m), row) in ids
            .into_iter()
            .zip(meta)
            .zip(t.vectors.as_slice().chunks_exact(dim))
        {
            ix.insert(id, row, m)?;
        }
        parts.map.insert(tenant, ix);
    }
    Ok(opened.file)
}

#[derive(Serialize, Deserialize)]
struct OpRecord {
    tenant: String,
    op_id: String,
    seq: u64,
    fp: u64,
    payload: String,
}

fn bad(why: impl std::fmt::Display) -> AtlasError {
    AtlasError::WalCorrupt(format!("checkpoint: {why}"))
}

fn tagged(tag: u8, body: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 1);
    out.push(tag);
    out.extend_from_slice(&body);
    out
}

pub(super) fn snapshot(parts: &Parts) -> Vec<Record> {
    let mut tenants: Vec<_> = parts.map.keys().collect();
    tenants.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    let mut out = Vec::new();
    for t in tenants {
        let Some(ix) = parts.map.get(t) else { continue };
        for (id, vector, metadata) in ix.entries() {
            out.push(Record {
                tenant: t.as_str().to_string(),
                id,
                vector,
                metadata,
                deleted: false,
                op_id: None,
            });
        }
    }
    out
}

#[cfg(test)]
fn encode_binary(rec: &Record) -> Result<Vec<u8>, AtlasError> {
    let tenant = rec.tenant.as_bytes();
    let tenant_len = u16::try_from(tenant.len()).map_err(|_| bad("tenant too long"))?;
    let dim = u32::try_from(rec.vector.len()).map_err(|_| bad("vector too long"))?;
    let meta = if rec.metadata.is_null() {
        Vec::new()
    } else {
        serde_json::to_vec(&rec.metadata).map_err(|e| AtlasError::Codec(e.to_string()))?
    };
    let meta_len = u32::try_from(meta.len()).map_err(|_| bad("metadata too long"))?;
    let mut out = Vec::with_capacity(19 + tenant.len() + rec.vector.len() * 4 + meta.len());
    out.push(TAG_ITEM_BIN);
    out.extend_from_slice(&tenant_len.to_le_bytes());
    out.extend_from_slice(tenant);
    out.extend_from_slice(&rec.id.0.to_le_bytes());
    out.extend_from_slice(&dim.to_le_bytes());
    for x in &rec.vector {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.extend_from_slice(&meta_len.to_le_bytes());
    out.extend_from_slice(&meta);
    Ok(out)
}

fn take<'a>(body: &mut &'a [u8], n: usize) -> Result<&'a [u8], AtlasError> {
    if body.len() < n {
        return Err(bad("truncated binary item"));
    }
    let (head, rest) = body.split_at(n);
    *body = rest;
    Ok(head)
}

struct BinItem<'a> {
    tenant: &'a str,
    id: u64,
    raw: &'a [u8],
    meta: &'a [u8],
}

fn parse_binary(mut body: &[u8]) -> Result<BinItem<'_>, AtlasError> {
    let tenant_len = u16::from_le_bytes(take(&mut body, 2)?.try_into().map_err(bad)?) as usize;
    let tenant = std::str::from_utf8(take(&mut body, tenant_len)?).map_err(bad)?;
    let id = u64::from_le_bytes(take(&mut body, 8)?.try_into().map_err(bad)?);
    let dim = u32::from_le_bytes(take(&mut body, 4)?.try_into().map_err(bad)?) as usize;
    let raw = take(
        &mut body,
        dim.checked_mul(4)
            .ok_or_else(|| bad("dimension overflow"))?,
    )?;
    let meta_len = u32::from_le_bytes(take(&mut body, 4)?.try_into().map_err(bad)?) as usize;
    let meta = take(&mut body, meta_len)?;
    if !body.is_empty() {
        return Err(bad("trailing bytes in binary item"));
    }
    Ok(BinItem {
        tenant,
        id,
        raw,
        meta,
    })
}

fn fill_vector(raw: &[u8], out: &mut Vec<f32>) {
    out.clear();
    out.extend(
        raw.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])),
    );
}

fn parse_metadata(meta: &[u8]) -> Result<serde_json::Value, AtlasError> {
    if meta.is_empty() {
        Ok(serde_json::Value::Null)
    } else {
        serde_json::from_slice(meta).map_err(bad)
    }
}

#[cfg(test)]
fn decode_binary(body: &[u8]) -> Result<Record, AtlasError> {
    let item = parse_binary(body)?;
    let mut vector = Vec::new();
    fill_vector(item.raw, &mut vector);
    Ok(Record {
        tenant: item.tenant.to_string(),
        id: atlas_core::VectorId(item.id),
        vector,
        metadata: parse_metadata(item.meta)?,
        deleted: false,
        op_id: None,
    })
}

pub(super) fn ops(map: &OpMap, upto_seq: u64) -> Result<Vec<Vec<u8>>, AtlasError> {
    let mut entries: Vec<_> = map.iter().filter(|(_, e)| e.seq <= upto_seq).collect();
    entries.sort_by_key(|(_, e)| e.seq);
    let mut out = Vec::with_capacity(entries.len());
    for ((t, op_id), e) in entries {
        let payload = String::from_utf8(e.payload.to_vec()).map_err(|_| bad("op payload"))?;
        let rec = OpRecord {
            tenant: t.as_str().to_string(),
            op_id: op_id.clone(),
            seq: e.seq,
            fp: e.fp,
            payload,
        };
        let body = serde_json::to_vec(&rec).map_err(|e| AtlasError::Codec(e.to_string()))?;
        out.push(tagged(TAG_OP, body));
    }
    Ok(out)
}

fn peek_tenant(payload: &[u8]) -> Option<&str> {
    let (tag, body) = payload.split_first()?;
    if *tag != TAG_ITEM_BIN || body.len() < 2 {
        return None;
    }
    let len = u16::from_le_bytes([body[0], body[1]]) as usize;
    std::str::from_utf8(body.get(2..2 + len)?).ok()
}

fn count_items<'a>(payloads: impl Iterator<Item = &'a [u8]>) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for p in payloads {
        if let Some(t) = peek_tenant(p) {
            match counts.get_mut(t) {
                Some(n) => *n += 1,
                None => {
                    counts.insert(t.to_string(), 1);
                }
            }
        }
    }
    counts
}

fn load_binary_item(
    parts: &mut Parts,
    scratch: &mut Vec<f32>,
    cached: &mut Option<(String, TenantId)>,
    counts: &HashMap<String, usize>,
    body: &[u8],
) -> Result<(), AtlasError> {
    let item = parse_binary(body)?;
    if cached.as_ref().map(|(name, _)| name.as_str()) != Some(item.tenant) {
        let t = TenantId::parse(item.tenant).map_err(bad)?;
        *cached = Some((item.tenant.to_string(), t));
    }
    let Some((_, tenant)) = cached.as_ref() else {
        return Err(bad("tenant cache"));
    };
    fill_vector(item.raw, scratch);
    let metadata = parse_metadata(item.meta)?;
    let id = atlas_core::VectorId(item.id);
    if let Some(ix) = parts.map.get_mut(tenant) {
        return ix.insert(id, scratch, metadata);
    }
    let mut ix = (parts.make)(parts.dim);
    ix.reserve(counts.get(item.tenant).copied().unwrap_or(0));
    ix.insert(id, scratch, metadata)?;
    parts.map.insert(tenant.clone(), ix);
    Ok(())
}

pub(super) fn load<'a>(
    parts: &RwLock<Parts>,
    ops: &Mutex<OpMap>,
    payloads: impl Iterator<Item = &'a [u8]> + Clone,
    dir: &Path,
    seq: u64,
) -> Result<Option<MappedFile>, AtlasError> {
    let mut mapped: Option<MappedFile> = None;
    let counts = count_items(payloads.clone());
    let mut staged: HashMap<(TenantId, String), OpEntry> = HashMap::new();
    let mut guard = parts.write().map_err(|_| bad("index lock poisoned"))?;
    let mut scratch: Vec<f32> = Vec::new();
    let mut cached: Option<(String, TenantId)> = None;
    for (i, p) in payloads.enumerate() {
        let Some((tag, body)) = p.split_first() else {
            return Err(bad(format!("record {i} is empty")));
        };
        match *tag {
            TAG_MAP => {
                if mapped.is_some() {
                    return Err(bad("segment has two vector file references"));
                }
                mapped = Some(load_map(&mut guard, dir, seq, body)?);
            }
            TAG_ITEM_BIN => load_binary_item(&mut guard, &mut scratch, &mut cached, &counts, body)
                .map_err(|e| bad(format!("record {i}: {e}")))?,
            TAG_ITEM => {
                let r: Record = serde_json::from_slice(body).map_err(bad)?;
                let t = TenantId::parse(&r.tenant).map_err(bad)?;
                if r.deleted || r.op_id.is_some() {
                    return Err(bad(format!("record {i} is not a plain item")));
                }
                guard
                    .apply(t, r)
                    .map_err(|e| bad(format!("record {i}: {e}")))?;
            }
            TAG_OP => {
                let r: OpRecord = serde_json::from_slice(body).map_err(bad)?;
                let t = TenantId::parse(&r.tenant).map_err(bad)?;
                let entry = OpEntry {
                    seq: r.seq,
                    fp: r.fp,
                    payload: Arc::from(r.payload.as_bytes()),
                };
                if staged.insert((t, r.op_id), entry).is_some() {
                    return Err(bad(format!("record {i} repeats an operation id")));
                }
            }
            other => return Err(bad(format!("record {i} has unknown tag {other}"))),
        }
    }
    drop(guard);
    ops.lock()
        .map_err(|_| bad("op map lock poisoned"))?
        .extend(staged);
    Ok(mapped)
}

#[cfg(test)]
mod binary_tests {
    use super::*;
    use atlas_core::VectorId;
    use serde_json::json;

    fn rec(metadata: serde_json::Value, vector: Vec<f32>) -> Record {
        Record {
            tenant: "acme".into(),
            id: VectorId(42),
            vector,
            metadata,
            deleted: false,
            op_id: None,
        }
    }

    #[test]
    fn binary_item_round_trips_with_and_without_metadata() {
        for r in [
            rec(json!({"k": "v", "n": 3}), vec![0.5, -1.25, 3.0]),
            rec(serde_json::Value::Null, vec![1.0, 2.0]),
            rec(serde_json::Value::Null, vec![]),
        ] {
            let bytes = encode_binary(&r).unwrap();
            assert_eq!(bytes[0], TAG_ITEM_BIN);
            assert!(decode_binary(&bytes[1..]).unwrap() == r);
        }
    }

    #[test]
    fn truncated_or_padded_binary_item_is_rejected() {
        let bytes = encode_binary(&rec(json!({"a": 1}), vec![1.0, 2.0])).unwrap();
        for cut in 1..bytes.len() - 1 {
            assert!(decode_binary(&bytes[1..cut]).is_err(), "cut {cut}");
        }
        let mut padded = bytes[1..].to_vec();
        padded.push(0);
        assert!(decode_binary(&padded).is_err());
    }
}
