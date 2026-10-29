use super::{OpEntry, OpMap, Parts, Record};
use atlas_core::{AtlasError, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

const TAG_ITEM: u8 = 1;
const TAG_OP: u8 = 2;

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

pub(super) fn items(parts: &Parts) -> Result<Vec<Vec<u8>>, AtlasError> {
    let mut tenants: Vec<_> = parts.map.keys().collect();
    tenants.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    let mut out = Vec::new();
    for t in tenants {
        let Some(ix) = parts.map.get(t) else { continue };
        for (id, vector, metadata) in ix.entries() {
            let rec = Record {
                tenant: t.as_str().to_string(),
                id,
                vector,
                metadata,
                deleted: false,
                op_id: None,
            };
            let body = serde_json::to_vec(&rec).map_err(|e| AtlasError::Codec(e.to_string()))?;
            out.push(tagged(TAG_ITEM, body));
        }
    }
    Ok(out)
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

pub(super) fn load(
    parts: &RwLock<Parts>,
    ops: &Mutex<OpMap>,
    payloads: &[Vec<u8>],
) -> Result<(), AtlasError> {
    let mut staged: HashMap<(TenantId, String), OpEntry> = HashMap::new();
    for (i, p) in payloads.iter().enumerate() {
        let Some((tag, body)) = p.split_first() else {
            return Err(bad(format!("record {i} is empty")));
        };
        match *tag {
            TAG_ITEM => {
                let r: Record = serde_json::from_slice(body).map_err(bad)?;
                let t = TenantId::parse(&r.tenant).map_err(bad)?;
                if r.deleted || r.op_id.is_some() {
                    return Err(bad(format!("record {i} is not a plain item")));
                }
                parts
                    .write()
                    .map_err(|_| bad("index lock poisoned"))?
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
    ops.lock()
        .map_err(|_| bad("op map lock poisoned"))?
        .extend(staged);
    Ok(())
}
