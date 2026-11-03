//! Context Neighbourhood Retrieval, first slice. Deterministic and bounded:
//! plan (intent -> relation set) -> seed (vector + lexical + exact) -> bounded BFS -> trace.
//! Not implemented (stated in every result): temporal ranking, learned ranking, manifest read versions.
use crate::model::*;
use crate::store::Lcf;
use atlas_core::EffectiveScope;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};

pub const HARD_HOPS: usize = 4;
pub const HARD_NODES: usize = 60;
pub const HARD_EDGES: usize = 300;
pub const DEADLINE: Duration = Duration::from_millis(2000);
const CANDIDATES: usize = 200;
/// Hashed vectors collide, so a seed with no lexical or exact support needs a strong vector match.

#[derive(Clone, Copy)]
pub struct Opts {
    pub max_hops: usize,
    pub max_nodes: usize,
    pub seeds: usize,
    /// Pin the read to a version (see current_version); None means latest committed.
    pub as_of: Option<u64>,
}
impl Default for Opts {
    fn default() -> Self {
        Opts {
            max_hops: 3,
            max_nodes: 40,
            seeds: 3,
            as_of: None,
        }
    }
}

struct Plan {
    rels: Vec<&'static str>,
    reasons: Vec<String>,
    prefer: Option<&'static str>,
}

type Intent = (
    &'static [&'static str],
    &'static [&'static str],
    Option<&'static str>,
    &'static str,
);

const INTENTS: [Intent; 6] = [
    (
        &["why", "cause", "caused", "led", "reason", "because"],
        &["CAUSED_BY", "DERIVED_FROM", "DEPENDS_ON", "SUPPORTS"],
        None,
        "causal question",
    ),
    (
        &[
            "evidence",
            "support",
            "supporting",
            "prove",
            "proof",
            "source",
        ],
        &["SUPPORTS", "DERIVED_FROM", "DISPUTES"],
        None,
        "evidence question",
    ),
    (
        &[
            "superseded",
            "replaced",
            "history",
            "changed",
            "change",
            "previous",
            "latest",
            "current",
            "version",
        ],
        &["SUPERSEDES"],
        None,
        "history question",
    ),
    (
        &["experiment", "experiments", "test", "tested"],
        &["RELATES_TO", "DERIVED_FROM", "CAUSED_BY"],
        Some("experiment"),
        "experiment question",
    ),
    (
        &[
            "problem",
            "problems",
            "remain",
            "remaining",
            "risk",
            "fail",
            "failure",
            "dispute",
            "contradict",
        ],
        &["DISPUTES", "CAUSED_BY"],
        None,
        "open-problem question",
    ),
    (
        &["contains", "inside", "under", "belongs"],
        &["CONTAINS"],
        None,
        "structure question",
    ),
];

fn plan(words: &[String]) -> Plan {
    let mut p = Plan {
        rels: vec![],
        reasons: vec![],
        prefer: None,
    };
    for (keys, rels, prefer, why) in INTENTS {
        if let Some(k) = keys.iter().find(|k| words.iter().any(|w| w == **k)) {
            for r in rels {
                if !p.rels.contains(r) {
                    p.rels.push(r);
                }
            }
            p.prefer = p.prefer.or(prefer);
            p.reasons.push(format!("'{k}' -> {why}"));
        }
    }
    if p.rels.is_empty() {
        p.rels = RELS.iter().copied().filter(|r| *r != "CONTAINS").collect();
        p.reasons
            .push("no intent keyword: all relations except CONTAINS".into());
    }
    p
}

struct Seed {
    node: Node,
    vec: f32,
    lex: f32,
    exact: f32,
    score: f32,
}

fn lexical(q: &[String], n: &Node) -> f32 {
    if q.is_empty() {
        return 0.0;
    }
    let title: HashSet<String> = tokens(&n.title).into_iter().collect();
    let body: HashSet<String> = tokens(&n.body).into_iter().collect();
    let got: f32 = q
        .iter()
        .map(|t| {
            if title.contains(t) {
                1.0
            } else if body.contains(t) {
                0.5
            } else {
                0.0
            }
        })
        .sum();
    (got / q.len() as f32).min(1.0)
}

struct Visit {
    node: Node,
    depth: usize,
    parent: Option<(String, Edge)>,
}

struct Walk {
    seen: HashMap<String, Visit>,
    order: Vec<String>,
    adj: HashMap<String, Vec<Edge>>,
    warn: Vec<String>,
    trace: Vec<String>,
}

impl<'a> Lcf<'a> {
    fn seeds(
        &self,
        sc: &EffectiveScope,
        text: &str,
        p: &Plan,
        o: Opts,
        warn: &mut Vec<String>,
    ) -> Res<Vec<Seed>> {
        let (cands, mismatch) = self.candidates(sc, text, CANDIDATES, o.as_of)?;
        if mismatch {
            warn.push("EMBEDDER_MISMATCH: stored vectors came from another embedder; vector signal off, lexical only".into());
        }
        let strong = self.st.embedder.strong_similarity();
        let (q, low) = (tokens(text), text.to_lowercase());
        let mut v: Vec<Seed> = cands
            .into_iter()
            .map(|(node, vec)| (vec, node))
            .map(|(vec, node)| {
                let lex = lexical(&q, &node);
                let exact = (low.contains(&node.key.to_lowercase())
                    || (node.title.len() >= 6 && low.contains(&node.title.to_lowercase())))
                    as u8 as f32;
                let bonus = if p.prefer == Some(node.kind.as_str()) {
                    0.1
                } else {
                    0.0
                };
                let score = 0.4 * vec + 0.4 * lex + 0.5 * exact + bonus;
                Seed {
                    node,
                    vec,
                    lex,
                    exact,
                    score,
                }
            })
            .filter(|s| s.score >= 0.1 && (s.lex > 0.0 || s.exact > 0.0 || s.vec >= strong))
            .collect();
        v.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(a.node.key.cmp(&b.node.key))
        });
        v.truncate(o.seeds);
        Ok(v)
    }

    #[allow(clippy::too_many_arguments)] // ponytail: one bounded-BFS entry point; a params struct if it grows
    fn walk(
        &self,
        sc: &EffectiveScope,
        seeds: &[Seed],
        p: &Plan,
        o: Opts,
        t0: Instant,
        trace: Vec<String>,
        warn: Vec<String>,
    ) -> Res<Walk> {
        let mut w = Walk {
            seen: HashMap::new(),
            order: vec![],
            adj: HashMap::new(),
            warn,
            trace,
        };
        for s in seeds {
            w.seen.insert(
                s.node.key.clone(),
                Visit {
                    node: s.node.clone(),
                    depth: 0,
                    parent: None,
                },
            );
            w.order.push(s.node.key.clone());
        }
        let mut frontier = w.order.clone();
        let mut examined = 0usize;
        'hops: for depth in 0..o.max_hops {
            let mut next = vec![];
            for k in &frontier {
                if t0.elapsed() > DEADLINE {
                    w.warn.push("TRUNCATED: deadline".into());
                    break 'hops;
                }
                let (edges, trunc) = self.adjacency_at(sc, k, o.as_of)?;
                if trunc {
                    w.warn.push(format!(
                        "TRUNCATED: '{k}' has more than {} edges",
                        crate::store::ADJ_LIMIT
                    ));
                }
                for e in edges.iter().filter(|e| p.rels.contains(&e.rel.as_str())) {
                    examined += 1;
                    if examined > HARD_EDGES {
                        w.warn.push("TRUNCATED: edge budget".into());
                        break 'hops;
                    }
                    let nb = e.other(k).to_string();
                    if w.seen.contains_key(&nb) {
                        continue;
                    }
                    if w.seen.len() >= o.max_nodes {
                        w.warn.push("TRUNCATED: node budget".into());
                        break 'hops;
                    }
                    if let Some(node) = self.get_node_at(sc, &nb, o.as_of)? {
                        w.seen.insert(
                            nb.clone(),
                            Visit {
                                node,
                                depth: depth + 1,
                                parent: Some((k.clone(), e.clone())),
                            },
                        );
                        w.order.push(nb.clone());
                        next.push(nb);
                    }
                }
                w.adj.insert(k.clone(), edges);
            }
            w.trace.push(format!(
                "hop {}: expanded {} node(s), now {} node(s)",
                depth + 1,
                frontier.len(),
                w.seen.len()
            ));
            if next.is_empty() {
                w.trace.push("frontier empty: stopped".into());
                break;
            }
            frontier = next;
        }
        for k in w.order.clone() {
            if let std::collections::hash_map::Entry::Vacant(slot) = w.adj.entry(k.clone()) {
                slot.insert(self.adjacency_at(sc, &k, o.as_of)?.0);
            }
        }
        w.warn.sort();
        w.warn.dedup();
        Ok(w)
    }

    pub fn query(&self, sc: &EffectiveScope, text: &str, o: Opts) -> Res<Value> {
        let t0 = Instant::now();
        if text.trim().is_empty() || text.len() > 1000 {
            return invalid("query must be 1..=1000 bytes");
        }
        let latest = self.current_version(sc)?;
        let pinned = o.as_of.map(|v| v.min(latest)).unwrap_or(latest);
        let o = Opts {
            max_hops: o.max_hops.min(HARD_HOPS),
            max_nodes: o.max_nodes.clamp(1, HARD_NODES),
            seeds: o.seeds.clamp(1, 5),
            as_of: Some(pinned),
        };
        let p = plan(&raw_words(text));
        let mut trace = vec![format!(
            "plan: relations {:?}; {}",
            p.rels,
            p.reasons.join("; ")
        )];
        let mut warn = vec![];
        let seeds = self.seeds(sc, text, &p, o, &mut warn)?;
        let names: Vec<String> = seeds
            .iter()
            .map(|s| format!("{} ({:.2})", s.node.key, s.score))
            .collect();
        trace.push(format!(
            "seeds: {}",
            if names.is_empty() {
                "none".into()
            } else {
                names.join(", ")
            }
        ));
        let w = self.walk(sc, &seeds, &p, o, t0, trace, warn)?;
        Ok(self.render(&seeds, w, &p, t0, o, latest))
    }

    fn render(
        &self,
        seeds: &[Seed],
        w: Walk,
        p: &Plan,
        t0: Instant,
        o: Opts,
        latest: u64,
    ) -> Value {
        let mut edges: BTreeSet<Edge> = BTreeSet::new();
        for k in &w.order {
            for e in w.adj[k].iter().filter(|e| p.rels.contains(&e.rel.as_str())) {
                if w.seen.contains_key(&e.from) && w.seen.contains_key(&e.to) {
                    edges.insert(e.clone());
                }
            }
        }
        let nodes: Vec<Value> = w
            .order
            .iter()
            .map(|k| {
                let v = &w.seen[k];
                let (life, by) = Lcf::lifecycle(&w.adj[k], k);
                let mut path = vec![];
                let mut cur = v;
                while let Some((pk, e)) = &cur.parent {
                    path.push(json!({"from": e.from, "rel": e.rel, "to": e.to}));
                    cur = &w.seen[pk];
                }
                path.reverse();
                json!({"key": k, "kind": v.node.kind, "title": v.node.title, "body": v.node.body,
                       "source": v.node.source, "epistemic_status": v.node.status, "ts": v.node.ts,
                       "lifecycle": life, "superseded_by": by, "depth": v.depth, "path_from_seed": path})
            })
            .collect();
        let seeds_j: Vec<Value> = seeds
            .iter()
            .map(|s| {
                json!({"key": s.node.key, "score": s.score,
                "signals": {"vector": s.vec, "lexical": s.lex, "exact": s.exact}})
            })
            .collect();
        let mut warnings = w.warn;
        let truncated = warnings.iter().any(|x| x.starts_with("TRUNCATED"));
        if seeds.is_empty() {
            warnings.push("NO_SEED: nothing matched; no traversal performed".into());
        }
        json!({
            "query_plan": {"relations": p.rels, "reasons": p.reasons, "max_hops": o.max_hops, "max_nodes": o.max_nodes},
            "seeds": seeds_j, "nodes": nodes,
            "edges": edges.iter().map(|e| json!({"from": e.from, "rel": e.rel, "to": e.to, "source": e.source, "ts": e.ts, "epistemic_status": e.status})).collect::<Vec<_>>(),
            "trace": w.trace, "warnings": warnings, "truncated": truncated,
            "stats": {"nodes": w.order.len(), "edges": edges.len(), "elapsed_us": t0.elapsed().as_micros() as u64},
            "read_version": o.as_of, "latest_version": latest,
            "read_marker": {"pinned_to": o.as_of, "latest": latest,
                            "note": "snapshot read: objects and relationships with version above read_version are invisible"},
            "not_implemented": ["temporal ranking", "learned ranking"],
        })
    }
}
