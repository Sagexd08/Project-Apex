//! Write path and reads. Durability, authorization and tenant isolation come from
//! `atlas_engine::Engine`; reads are served from the derived index in `index.rs`.
use crate::index::*;
use crate::model::*;
use atlas_core::{EffectiveScope, Metric};
use atlas_engine::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub const ADJ_LIMIT: usize = 1000;

pub struct Lcf<'a> {
    pub engine: &'a Engine,
    pub st: &'a LcfState,
}

fn check_text(what: &str, v: &str, max: usize) -> Res<()> {
    if v.is_empty() || v.chars().count() > max {
        return invalid(&format!("{what} must be 1..={max} chars"));
    }
    Ok(())
}

fn unpoison<T>(r: std::sync::LockResult<T>) -> T {
    r.unwrap_or_else(|e| e.into_inner())
}

fn visible(ver: u64, as_of: Option<u64>) -> bool {
    as_of.map(|v| ver <= v).unwrap_or(true)
}

impl<'a> Lcf<'a> {
    pub fn new(engine: &'a Engine, st: &'a LcfState) -> Self {
        Lcf { engine, st }
    }

    /// Per-tenant state, loading the index from durable records on first use.
    pub fn tenant(&self, sc: &EffectiveScope) -> Res<Arc<TenantState>> {
        self.engine.count(sc)?; // authorization / tenant check first
        let name = sc
            .tenant()
            .map(|t| t.as_str().to_string())
            .ok_or_else(|| LcfError::Invalid("no tenant".into()))?;
        let ts = {
            let mut m = unpoison(self.st.tenants.lock());
            Arc::clone(m.entry(name).or_default())
        };
        if !unpoison(ts.idx.read()).loaded {
            let mut g = unpoison(ts.idx.write());
            if !g.loaded {
                self.load(sc, &mut g)?;
                let mut q = unpoison(ts.seq.lock());
                q.published = g.ver;
                q.next = g.ver + 1;
            }
        }
        Ok(ts)
    }

    fn load(&self, sc: &EffectiveScope, g: &mut TIndex) -> Res<()> {
        let me = self.st.embedder.name();
        let mut sink = |it: atlas_engine::Item| {
            if let Some(n) = Node::from_meta(&it.metadata) {
                let other = it
                    .metadata
                    .get("emb")
                    .and_then(Value::as_str)
                    .map(|e| e != me);
                if other.unwrap_or(false) {
                    g.emb_mismatch = true;
                }
                g.add_node(n);
            } else if let Some(e) = Edge::from_meta(&it.metadata) {
                g.add_edge(e);
            }
            Ok(())
        };
        self.engine.export(sc, &mut sink)?;
        g.loaded = true;
        Ok(())
    }

    fn embed(&self, text: &str) -> Res<Vec<f32>> {
        let v = self
            .st
            .embedder
            .embed(text)
            .map_err(LcfError::Unavailable)?;
        if v.len() != self.engine.dim() {
            return Err(LcfError::Unavailable(format!(
                "embedding dim {} != engine dim {}",
                v.len(),
                self.engine.dim()
            )));
        }
        Ok(v)
    }

    /// Take the next ticket for `tag`, or return the existing version for an identical record.
    /// `check` runs under the sequencing lock with read access to the published index and returns
    /// Ok(Some(ver)) for an identical existing record, Ok(None) to proceed, or an error.
    fn ticket(
        &self,
        ts: &TenantState,
        tag: &str,
        check: impl FnOnce(&TIndex) -> Res<Option<u64>>,
    ) -> Res<Result<u64, u64>> {
        let mut q = unpoison(ts.seq.lock());
        if q.inflight.contains(tag) {
            return Err(LcfError::Conflict(format!(
                "a write of '{tag}' is already in flight"
            )));
        }
        let g = unpoison(ts.idx.read());
        if let Some(v) = check(&g)? {
            return Ok(Err(v));
        }
        let t = q.next;
        q.next += 1;
        q.inflight.insert(tag.to_string());
        Ok(Ok(t))
    }

    /// Publish ticket `t` in order. `apply` is None when the write failed (the ticket is skipped).
    fn publish(
        &self,
        ts: &TenantState,
        t: u64,
        tag: &str,
        apply: Option<impl FnOnce(&mut TIndex)>,
    ) {
        let mut q = unpoison(ts.seq.lock());
        while q.published + 1 != t {
            q = unpoison(ts.turn.wait(q));
        }
        if let Some(f) = apply {
            let mut g = unpoison(ts.idx.write());
            f(&mut g);
        }
        q.published = t;
        q.inflight.remove(tag);
        drop(q);
        ts.turn.notify_all();
    }

    /// Idempotent by key: identical content is a no-op returning the existing version; different
    /// content is CONFLICT (canonical records are never overwritten; use `supersede`).
    pub fn put_node(&self, sc: &EffectiveScope, n: &Node) -> Res<u64> {
        check_text("key", &n.key, MAX_KEY)?;
        check_text("title", &n.title, MAX_TITLE)?;
        if n.body.chars().count() > MAX_BODY {
            return invalid("body too long");
        }
        if !KINDS.contains(&n.kind.as_str()) {
            return invalid("unknown kind");
        }
        if !STATUSES.contains(&n.status.as_str()) {
            return invalid("unknown status");
        }
        if n.source.is_empty() {
            return invalid("source is required (provenance)");
        }
        let ts = self.tenant(sc)?;
        let tag = format!("n|{}", n.key);
        let id = id_of(&tag);
        let t = match self.ticket(&ts, &tag, |g| {
            if let Some(old) = g.nodes.get(&n.key) {
                return if old.same_content(n) {
                    Ok(Some(old.ver))
                } else {
                    Err(LcfError::Conflict(format!(
                        "node '{}' exists with different content; use supersede",
                        n.key
                    )))
                };
            }
            if g.id_key.get(&id.0).map(|k| k != &n.key).unwrap_or(false) {
                return Err(LcfError::Conflict("id collision".into()));
            }
            Ok(None)
        })? {
            Ok(t) => t,
            Err(existing) => return Ok(existing),
        };
        let mut node = n.clone();
        node.ver = t;
        let result = self
            .embed(&format!("{} {} {}", n.title, n.body, n.kind))
            .and_then(|vec| {
                let meta = node.to_meta(&self.st.embedder.name());
                self.engine
                    .insert(sc, id, &vec, meta)
                    .map_err(LcfError::from)
            });
        match result {
            Ok(_) => {
                self.publish(&ts, t, &tag, Some(|g: &mut TIndex| g.add_node(node)));
                Ok(t)
            }
            Err(e) => {
                self.publish(&ts, t, &tag, None::<fn(&mut TIndex)>);
                Err(e)
            }
        }
    }

    pub fn put_edge(&self, sc: &EffectiveScope, e: &Edge) -> Res<u64> {
        if !RELS.contains(&e.rel.as_str()) {
            return invalid("unknown relationship type");
        }
        if !STATUSES.contains(&e.status.as_str()) {
            return invalid("unknown status");
        }
        if e.source.is_empty() {
            return invalid("source is required (provenance)");
        }
        if e.from == e.to {
            return invalid("self edges are not allowed");
        }
        let ts = self.tenant(sc)?;
        let tag = edge_tag(e);
        let t = match self.ticket(&ts, &tag, |g| {
            for k in [&e.from, &e.to] {
                if !g.nodes.contains_key(k) {
                    return Err(LcfError::NotFound(format!("endpoint '{k}'")));
                }
            }
            if g.tags.contains(&tag) {
                let old = g.adj[&e.from]
                    .iter()
                    .find(|o| o.rel == e.rel && o.to == e.to);
                return match old {
                    Some(o) if (&o.source, o.ts, &o.status) == (&e.source, e.ts, &e.status) => {
                        Ok(Some(o.ver))
                    }
                    _ => Err(LcfError::Conflict(
                        "edge exists with different content".into(),
                    )),
                };
            }
            Ok(None)
        })? {
            Ok(t) => t,
            Err(existing) => return Ok(existing),
        };
        let mut edge = e.clone();
        edge.ver = t;
        let mut vec = vec![0f32; self.engine.dim()];
        vec[0] = 1.0; // edges are not searched by similarity; node searches exclude them
        match self.engine.insert(sc, id_of(&tag), &vec, edge.to_meta()) {
            Ok(_) => {
                self.publish(&ts, t, &tag, Some(|g: &mut TIndex| g.add_edge(edge)));
                Ok(t)
            }
            Err(err) => {
                self.publish(&ts, t, &tag, None::<fn(&mut TIndex)>);
                Err(err.into())
            }
        }
    }

    /// New version of `old_key`: writes `new`, then `new SUPERSEDES old`. The old record stays.
    pub fn supersede(&self, sc: &EffectiveScope, old_key: &str, new: &Node) -> Res<u64> {
        if self.get_node(sc, old_key)?.is_none() {
            return Err(LcfError::NotFound(format!("'{old_key}'")));
        }
        self.put_node(sc, new)?;
        let e = Edge {
            rel: "SUPERSEDES".into(),
            from: new.key.clone(),
            to: old_key.into(),
            source: new.source.clone(),
            ts: new.ts,
            status: new.status.clone(),
            ver: 0,
        };
        self.put_edge(sc, &e)
    }

    /// Latest committed version for this tenant (what an unpinned read sees).
    pub fn current_version(&self, sc: &EffectiveScope) -> Res<u64> {
        let ts = self.tenant(sc)?;
        let v = unpoison(ts.idx.read()).ver;
        Ok(v)
    }

    pub fn get_node(&self, sc: &EffectiveScope, key: &str) -> Res<Option<Node>> {
        self.get_node_at(sc, key, None)
    }

    pub fn get_node_at(
        &self,
        sc: &EffectiveScope,
        key: &str,
        as_of: Option<u64>,
    ) -> Res<Option<Node>> {
        let ts = self.tenant(sc)?;
        let g = unpoison(ts.idx.read());
        Ok(g.nodes.get(key).filter(|n| visible(n.ver, as_of)).cloned())
    }

    /// Every edge touching `key` visible at `as_of`, deterministic order. bool = truncated.
    pub fn adjacency_at(
        &self,
        sc: &EffectiveScope,
        key: &str,
        as_of: Option<u64>,
    ) -> Res<(Vec<Edge>, bool)> {
        let ts = self.tenant(sc)?;
        let g = unpoison(ts.idx.read());
        let mut v: Vec<Edge> = g
            .adj
            .get(key)
            .map(|a| {
                a.iter()
                    .filter(|e| visible(e.ver, as_of))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        let truncated = v.len() > ADJ_LIMIT;
        v.truncate(ADJ_LIMIT);
        Ok((v, truncated))
    }

    pub fn adjacency(&self, sc: &EffectiveScope, key: &str) -> Res<(Vec<Edge>, bool)> {
        self.adjacency_at(sc, key, None)
    }

    pub fn nodes_of_kind(&self, sc: &EffectiveScope, kind: &str, limit: usize) -> Res<Vec<Node>> {
        let ts = self.tenant(sc)?;
        let g = unpoison(ts.idx.read());
        let mut v: Vec<Node> = g
            .by_kind
            .get(kind)
            .map(|ks| ks.iter().filter_map(|k| g.nodes.get(k).cloned()).collect())
            .unwrap_or_default();
        v.sort_by(|a, b| a.key.cmp(&b.key));
        v.truncate(limit);
        Ok(v)
    }

    /// Seed candidates: vector (engine exact search restricted to node ids) plus lexical postings.
    /// Returns (node, vector similarity or 0.0) pairs visible at `as_of`, and whether the vector
    /// signal was off because stored vectors came from a different embedder.
    pub fn candidates(
        &self,
        sc: &EffectiveScope,
        text: &str,
        k: usize,
        as_of: Option<u64>,
    ) -> Res<(Vec<(Node, f32)>, bool)> {
        let ts = self.tenant(sc)?;
        let mismatch = unpoison(ts.idx.read()).emb_mismatch;
        let mut out: HashMap<String, (Node, f32)> = HashMap::new();
        if !mismatch {
            let qv = self.embed(text)?;
            let g = unpoison(ts.idx.read());
            let hits = self
                .engine
                .search_filtered(sc, &qv, k, Metric::Cosine, &|id| {
                    g.id_key.contains_key(&id.0)
                })?;
            for h in hits {
                if let Some(n) = Node::from_meta(&h.metadata) {
                    out.insert(n.key.clone(), (n, h.score.max(0.0)));
                }
            }
        }
        let g = unpoison(ts.idx.read());
        let mut score: HashMap<&String, usize> = HashMap::new();
        for t in tokens(text) {
            for key in g.posting.get(&t).map(|v| v.as_slice()).unwrap_or(&[]) {
                *score.entry(key).or_default() += 1;
            }
        }
        let mut lex: Vec<(&String, usize)> = score.into_iter().collect();
        lex.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        for (key, _) in lex.into_iter().take(k) {
            if let Some(n) = g.nodes.get(key) {
                out.entry(key.clone()).or_insert((n.clone(), 0.0));
            }
        }
        let v: Vec<(Node, f32)> = out
            .into_values()
            .filter(|(n, _)| visible(n.ver, as_of))
            .collect();
        Ok((v, mismatch))
    }

    /// "current" unless a SUPERSEDES edge points at it.
    pub fn lifecycle(edges: &[Edge], key: &str) -> (&'static str, Option<String>) {
        match edges.iter().find(|e| e.rel == "SUPERSEDES" && e.to == key) {
            Some(e) => ("superseded", Some(e.from.clone())),
            None => ("current", None),
        }
    }

    pub fn stats(&self, sc: &EffectiveScope) -> Res<Value> {
        let ts = self.tenant(sc)?;
        let g = unpoison(ts.idx.read());
        let kinds: serde_json::Map<String, Value> = KINDS
            .iter()
            .map(|k| {
                (
                    k.to_string(),
                    json!(g.by_kind.get(*k).map(Vec::len).unwrap_or(0)),
                )
            })
            .collect();
        Ok(
            json!({"records": self.engine.count(sc)?, "nodes": g.nodes.len(), "edges": g.edges,
                  "version": g.ver, "nodes_by_kind": kinds, "embedder": self.st.embedder.name(),
                  "embedder_mismatch": g.emb_mismatch}),
        )
    }
}
