//! JSON views used by the HTTP API and the Explorer. Progressive: one hop at a time.
use crate::model::*;
use crate::store::Lcf;
use atlas_core::EffectiveScope;
use serde_json::{json, Value};

pub fn node_json(n: &Node, life: &str, by: Option<String>) -> Value {
    json!({"key": n.key, "kind": n.kind, "title": n.title, "body": n.body, "source": n.source,
           "epistemic_status": n.status, "ts": n.ts, "lifecycle": life, "superseded_by": by})
}

pub fn edge_json(e: &Edge) -> Value {
    json!({"from": e.from, "rel": e.rel, "to": e.to, "source": e.source, "ts": e.ts,
           "epistemic_status": e.status})
}

pub const EXPAND_LIMIT: usize = 60;

impl<'a> Lcf<'a> {
    pub fn node_view(&self, sc: &EffectiveScope, key: &str, as_of: Option<u64>) -> Res<Value> {
        let n = self
            .get_node_at(sc, key, as_of)?
            .ok_or_else(|| LcfError::NotFound(key.to_string()))?;
        let (edges, trunc) = self.adjacency_at(sc, key, as_of)?;
        let (life, by) = Lcf::lifecycle(&edges, key);
        let mut v = node_json(&n, life, by);
        v["degree"] = json!(edges.len());
        v["truncated"] = json!(trunc);
        Ok(v)
    }

    /// One hop around `key`, optionally restricted to relation types. Never more than EXPAND_LIMIT
    /// neighbours; `has_more` says so. The browser asks again per node it wants to open.
    pub fn expand(
        &self,
        sc: &EffectiveScope,
        key: &str,
        rels: &[String],
        as_of: Option<u64>,
    ) -> Res<Value> {
        let center = self.node_view(sc, key, as_of)?;
        let (edges, trunc) = self.adjacency_at(sc, key, as_of)?;
        let picked: Vec<&Edge> = edges
            .iter()
            .filter(|e| rels.is_empty() || rels.contains(&e.rel))
            .collect();
        let (mut nodes, mut out_edges, mut has_more) = (vec![], vec![], trunc);
        for e in picked {
            if nodes.len() >= EXPAND_LIMIT {
                has_more = true;
                break;
            }
            let nb = e.other(key);
            if let Some(n) = self.get_node_at(sc, nb, as_of)? {
                let (nadj, _) = self.adjacency_at(sc, nb, as_of)?;
                let (life, by) = Lcf::lifecycle(&nadj, nb);
                let mut j = node_json(&n, life, by);
                j["degree"] = json!(nadj.len());
                nodes.push(j);
                out_edges.push(edge_json(e));
            }
        }
        Ok(json!({"center": center, "nodes": nodes, "edges": out_edges, "has_more": has_more}))
    }

    /// Entry points for the Explorer: workspaces, else projects, else nodes nothing contains.
    pub fn roots(&self, sc: &EffectiveScope) -> Res<Value> {
        let mut picked = self.nodes_of_kind(sc, "workspace", 50)?;
        if picked.is_empty() {
            picked = self.nodes_of_kind(sc, "project", 50)?;
        }
        if picked.is_empty() {
            let ts = self.tenant(sc)?;
            let g = ts.idx.read().unwrap_or_else(|e| e.into_inner());
            let mut keys: Vec<&String> = g
                .nodes
                .keys()
                .filter(|k| {
                    !g.adj
                        .get(*k)
                        .map(|a| a.iter().any(|e| e.rel == "CONTAINS" && &e.to == *k))
                        .unwrap_or(false)
                })
                .collect();
            keys.sort();
            picked = keys
                .into_iter()
                .take(20)
                .filter_map(|k| g.nodes.get(k).cloned())
                .collect();
        }
        let mut v = vec![];
        for n in picked {
            let (edges, _) = self.adjacency_at(sc, &n.key, None)?;
            let mut j = node_json(&n, "current", None);
            j["degree"] = json!(edges.len());
            v.push(j);
        }
        Ok(json!({"roots": v, "version": self.current_version(sc)?}))
    }
}
