//! In-memory read index, rebuilt from the engine's durable records on first use per tenant and
//! kept current by the write path (a record is added to the index only after the engine made it
//! durable). The engine stays the single source of truth; this index is derived and disposable.
use crate::embed::{Embedder, HashEmbedder};
use crate::model::*;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex, RwLock};

#[derive(Default)]
pub struct TIndex {
    pub loaded: bool,
    pub ver: u64,
    pub nodes: HashMap<String, Node>,
    pub id_key: HashMap<u64, String>,
    pub adj: HashMap<String, Vec<Edge>>,
    pub tags: HashSet<String>,
    pub posting: HashMap<String, Vec<String>>,
    pub by_kind: HashMap<String, Vec<String>>,
    pub emb_mismatch: bool,
    pub edges: usize,
}

pub fn edge_tag(e: &Edge) -> String {
    format!("e|{}|{}|{}", e.from, e.rel, e.to)
}

impl TIndex {
    pub fn add_node(&mut self, n: Node) {
        let mut toks: Vec<String> = tokens(&n.title)
            .into_iter()
            .chain(tokens(&n.body))
            .chain(tokens(&n.key))
            .collect();
        toks.sort();
        toks.dedup();
        for t in toks {
            self.posting.entry(t).or_default().push(n.key.clone());
        }
        self.by_kind
            .entry(n.kind.clone())
            .or_default()
            .push(n.key.clone());
        self.id_key
            .insert(id_of(&format!("n|{}", n.key)).0, n.key.clone());
        self.ver = self.ver.max(n.ver);
        self.nodes.insert(n.key.clone(), n);
    }

    pub fn add_edge(&mut self, e: Edge) {
        if !self.tags.insert(edge_tag(&e)) {
            return;
        }
        self.ver = self.ver.max(e.ver);
        self.edges += 1;
        self.adj.entry(e.from.clone()).or_default().push(e.clone());
        self.adj.entry(e.to.clone()).or_default().push(e);
    }
}

/// Write sequencing: tickets are handed out in order, durable inserts run concurrently (so the
/// engine can group-commit them), and results are published to the index strictly in ticket
/// order. A reader pinned at version v therefore never misses a record with version <= v.
#[derive(Default)]
pub struct Seq {
    pub next: u64,
    pub published: u64,
    pub inflight: HashSet<String>,
}

#[derive(Default)]
pub struct TenantState {
    pub idx: RwLock<TIndex>,
    pub seq: Mutex<Seq>,
    pub turn: Condvar,
}

pub struct LcfState {
    pub tenants: Mutex<HashMap<String, Arc<TenantState>>>,
    pub embedder: Arc<dyn Embedder>,
}

impl LcfState {
    pub fn new(embedder: Arc<dyn Embedder>) -> Self {
        LcfState {
            tenants: Mutex::new(HashMap::new()),
            embedder,
        }
    }
}

impl Default for LcfState {
    fn default() -> Self {
        Self::new(Arc::new(HashEmbedder))
    }
}
