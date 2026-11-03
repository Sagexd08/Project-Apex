use crate::http::Request;
use crate::Server;
use atlas_core::{AtlasError, EffectiveScope, Metric, VectorId};
use serde_json::{json, Value};
use std::collections::HashSet;

pub const MAX_K: usize = 1000;
pub const MAX_ALLOW_IDS: usize = 100_000;
pub const MAX_BATCH: usize = 1000;
const BATCH_THREADS: usize = 16;

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
        ("POST", "/v1/insert_batch") => insert_batch(server, &scope, &body),
        ("POST", "/v1/search") => search(server, &scope, &body),
        ("POST", "/v1/fetch") => fetch(server, &scope, &body),
        ("POST", "/v1/delete") => delete(server, &scope, &body),
        ("GET", "/v1/count") => server
            .engine
            .count(&scope)
            .map(|n| json!({ "count": n }))
            .map_err(fail),
        (m, p) if p.starts_with("/v1/lcf/") => lcf(server, &scope, m, p, &body),
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

fn insert_batch(s: &Server, scope: &EffectiveScope, b: &Value) -> Result<Value, Reply> {
    let items = b
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("items"))?;
    if items.is_empty() || items.len() > MAX_BATCH {
        return Err(bad("items must be 1..=1000"));
    }
    let mut parsed = Vec::with_capacity(items.len());
    for it in items {
        let meta = it.get("metadata").cloned().unwrap_or(Value::Null);
        parsed.push((id_of(it)?, vector_of(it, "vector")?, meta));
    }
    let results: Vec<Result<u64, AtlasError>> = std::thread::scope(|sc| {
        let chunk = parsed.len().div_ceil(BATCH_THREADS);
        let handles: Vec<_> = parsed
            .chunks(chunk)
            .map(|part| {
                sc.spawn(move || {
                    part.iter()
                        .map(|(id, v, m)| s.engine.insert(scope, *id, v, m.clone()))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    let mut seqs = Vec::with_capacity(results.len());
    let mut first_err = None;
    for r in results {
        match r {
            Ok(seq) => seqs.push(seq),
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    match first_err {
        None => Ok(json!({ "inserted": seqs.len(), "seqs": seqs })),
        Some(e) => {
            let (status, mut body) = fail(e);
            body["inserted"] = json!(seqs.len());
            Err((status, body))
        }
    }
}

fn search(s: &Server, scope: &EffectiveScope, b: &Value) -> Result<Value, Reply> {
    let q = vector_of(b, "vector")?;
    let k = b.get("k").and_then(Value::as_u64).unwrap_or(10) as usize;
    if k == 0 || k > MAX_K {
        return Err(bad("k must be 1..=1000"));
    }
    let metric = metric_of(b)?;
    if b.get("filter").is_some() && b.get("allow_ids").is_some() {
        return Err(bad("use filter or allow_ids, not both"));
    }
    let hits = match b.get("allow_ids").and_then(Value::as_array) {
        Some(ids) => {
            if ids.len() > MAX_ALLOW_IDS {
                return Err(bad("allow_ids too long"));
            }
            let allow: HashSet<u64> = ids.iter().filter_map(Value::as_u64).collect();
            s.engine
                .search_filtered(scope, &q, k, metric, &|id| allow.contains(&id.0))
        }
        None => match b.get("filter") {
            Some(f) => {
                let filter = atlas_engine::Filter::from_json(f).map_err(fail)?;
                s.engine.search_where(scope, &q, k, metric, &filter)
            }
            None => s.engine.search(scope, &q, k, metric),
        },
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

fn lfail(e: atlas_lcf::LcfError) -> Reply {
    match e {
        atlas_lcf::LcfError::Engine(e) => fail(e),
        other => {
            let code = other.code();
            let status = match code.as_str() {
                "INVALID" => 400,
                "NOT_FOUND" => 404,
                "CONFLICT" => 409,
                "UNAVAILABLE" => 503,
                _ => 500,
            };
            (status, json!({ "error": code, "detail": other.detail() }))
        }
    }
}

fn text(b: &Value, k: &str) -> String {
    b.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn node_of(b: &Value) -> atlas_lcf::Node {
    atlas_lcf::Node {
        key: text(b, "key"),
        kind: text(b, "kind"),
        title: text(b, "title"),
        body: text(b, "body"),
        source: text(b, "source"),
        status: if b.get("status").is_some() {
            text(b, "status")
        } else {
            "asserted".into()
        },
        ts: b.get("ts").and_then(Value::as_i64).unwrap_or(0),
        ver: 0,
    }
}

fn edge_of(b: &Value) -> atlas_lcf::Edge {
    atlas_lcf::Edge {
        rel: text(b, "rel"),
        from: text(b, "from"),
        to: text(b, "to"),
        source: text(b, "source"),
        ts: b.get("ts").and_then(Value::as_i64).unwrap_or(0),
        status: if b.get("status").is_some() {
            text(b, "status")
        } else {
            "asserted".into()
        },
        ver: 0,
    }
}

fn lcf(
    s: &Server,
    scope: &EffectiveScope,
    method: &str,
    path: &str,
    b: &Value,
) -> Result<Value, Reply> {
    use atlas_lcf::{cnr::Opts, Lcf};
    let l = Lcf::new(&s.engine, &s.lcf);
    let as_of = b.get("as_of").and_then(Value::as_u64);
    let num = |k: &str, d: usize| {
        b.get(k)
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(d)
    };
    let r = match (method, path) {
        ("POST", "/v1/lcf/node") => l
            .put_node(scope, &node_of(b))
            .map(|seq| json!({ "seq": seq })),
        ("POST", "/v1/lcf/edge") => l
            .put_edge(scope, &edge_of(b))
            .map(|seq| json!({ "seq": seq })),
        ("POST", "/v1/lcf/supersede") => {
            let new = node_of(b.get("node").unwrap_or(&Value::Null));
            l.supersede(scope, &text(b, "old"), &new)
                .map(|seq| json!({ "seq": seq }))
        }
        ("POST", "/v1/lcf/query") => {
            let d = Opts::default();
            let o = Opts {
                max_hops: num("max_hops", d.max_hops),
                max_nodes: num("max_nodes", d.max_nodes),
                seeds: num("seeds", d.seeds),
                as_of,
            };
            l.query(scope, &text(b, "q"), o)
        }
        ("POST", "/v1/lcf/node/get") => l.node_view(scope, &text(b, "key"), as_of),
        ("POST", "/v1/lcf/expand") => {
            let rels: Vec<String> = b
                .get("rels")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            l.expand(scope, &text(b, "key"), &rels, as_of)
        }
        ("POST", "/v1/lcf/ingest") => {
            let docs: Vec<atlas_lcf::ingest::Doc> = b
                .get("documents")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|d| atlas_lcf::ingest::Doc {
                            path: text(d, "path"),
                            text: text(d, "text"),
                        })
                        .collect()
                })
                .unwrap_or_default();
            l.ingest(scope, &docs)
        }
        ("GET", "/v1/lcf/roots") => l.roots(scope),
        ("GET", "/v1/lcf/stats") => l.stats(scope),
        _ => return Err((404, json!({ "error": "NOT_FOUND" }))),
    };
    r.map_err(lfail)
}
