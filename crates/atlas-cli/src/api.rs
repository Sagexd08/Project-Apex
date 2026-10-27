//! JSON routes. Every route authenticates; the tenant comes only from the key, never the body.
use crate::http::Request;
use crate::Server;
use atlas_core::{AtlasError, EffectiveScope, Metric, VectorId};
use serde_json::{json, Value};
use std::collections::HashSet;

pub const MAX_K: usize = 1000;
pub const MAX_ALLOW_IDS: usize = 100_000;

type Reply = (u16, Value);

fn status_of(e: &AtlasError) -> u16 {
    match e.code() {
        "FORBIDDEN" => 403,
        "NOT_FOUND" => 404,
        "OP_ID_CONFLICT" => 409,
        "RECORD_TOO_LARGE" => 413,
        c if c.starts_with("INVALID") || c == "DIMENSION_MISMATCH" => 400,
        _ => 500,
    }
}

/// Only the stable error code leaves the process; messages stay in the engine.
fn fail(e: AtlasError) -> Reply {
    (status_of(&e), json!({ "error": e.code() }))
}

fn bad(msg: &str) -> Reply {
    (400, json!({ "error": "BAD_REQUEST", "detail": msg }))
}

fn unauth() -> Reply {
    (401, json!({ "error": "UNAUTHENTICATED" }))
}

fn vector_of(v: &Value, key: &str) -> Result<Vec<f32>, Reply> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| bad(key))?;
    arr.iter()
        .map(|x| x.as_f64().map(|f| f as f32).ok_or_else(|| bad(key)))
        .collect()
}

fn id_of(v: &Value) -> Result<VectorId, Reply> {
    v.get("id")
        .and_then(Value::as_u64)
        .map(VectorId)
        .ok_or_else(|| bad("id"))
}

fn metric_of(v: &Value) -> Result<Metric, Reply> {
    match v.get("metric").and_then(Value::as_str).unwrap_or("cosine") {
        "cosine" => Ok(Metric::Cosine),
        "dot" => Ok(Metric::Dot),
        "euclidean" => Ok(Metric::Euclidean),
        _ => Err(bad("metric")),
    }
}

fn scope_of(server: &Server, req: &Request) -> Result<EffectiveScope, Reply> {
    let secret = req.bearer.as_deref().ok_or_else(unauth)?;
    server
        .auth
        .authenticate(secret)
        .map(|c| c.effective())
        .map_err(|_| unauth())
}

pub fn dispatch(server: &Server, req: &Request) -> Reply {
    let scope = match scope_of(server, req) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let body: Value = if req.body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&req.body) {
            Ok(v) => v,
            Err(_) => return bad("json"),
        }
    };
    let out = match (req.method.as_str(), req.path.as_str()) {
        ("POST", "/v1/insert") => insert(server, &scope, &body),
        ("POST", "/v1/search") => search(server, &scope, &body),
        ("POST", "/v1/fetch") => fetch(server, &scope, &body),
        ("POST", "/v1/delete") => delete(server, &scope, &body),
        ("GET", "/v1/count") => server
            .engine
            .count(&scope)
            .map(|n| json!({ "count": n }))
            .map_err(fail),
        _ => Err((404, json!({ "error": "NOT_FOUND" }))),
    };
    match out {
        Ok(v) => (200, v),
        Err(e) => e,
    }
}

fn insert(s: &Server, scope: &EffectiveScope, b: &Value) -> Result<Value, Reply> {
    let (id, v) = (id_of(b)?, vector_of(b, "vector")?);
    let meta = b.get("metadata").cloned().unwrap_or(Value::Null);
    let seq = match b.get("op_id").and_then(Value::as_str) {
        Some(op) => s.engine.insert_op(scope, op, id, &v, meta),
        None => s.engine.insert(scope, id, &v, meta),
    };
    seq.map(|seq| json!({ "seq": seq })).map_err(fail)
}

fn search(s: &Server, scope: &EffectiveScope, b: &Value) -> Result<Value, Reply> {
    let q = vector_of(b, "vector")?;
    let k = b.get("k").and_then(Value::as_u64).unwrap_or(10) as usize;
    if k == 0 || k > MAX_K {
        return Err(bad("k must be 1..=1000"));
    }
    let metric = metric_of(b)?;
    let hits = match b.get("allow_ids").and_then(Value::as_array) {
        Some(ids) => {
            if ids.len() > MAX_ALLOW_IDS {
                return Err(bad("allow_ids too long"));
            }
            let allow: HashSet<u64> = ids.iter().filter_map(Value::as_u64).collect();
            s.engine
                .search_filtered(scope, &q, k, metric, &|id| allow.contains(&id.0))
        }
        None => s.engine.search(scope, &q, k, metric),
    }
    .map_err(fail)?;
    let hits: Vec<Value> = hits
        .into_iter()
        .map(|h| json!({ "id": h.id.0, "score": h.score, "metadata": h.metadata }))
        .collect();
    Ok(json!({ "hits": hits }))
}

fn fetch(s: &Server, scope: &EffectiveScope, b: &Value) -> Result<Value, Reply> {
    let it = s.engine.fetch(scope, id_of(b)?).map_err(fail)?;
    Ok(json!({ "id": it.id.0, "vector": it.vector, "metadata": it.metadata }))
}

fn delete(s: &Server, scope: &EffectiveScope, b: &Value) -> Result<Value, Reply> {
    s.engine
        .delete(scope, id_of(b)?)
        .map(|existed| json!({ "deleted": existed }))
        .map_err(fail)
}
