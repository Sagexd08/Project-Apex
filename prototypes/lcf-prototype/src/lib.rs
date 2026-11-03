//! Deterministic, in-memory prototype of the proposed knowledge semantics. NOT production storage.
//! Purpose: pin down the logical contract (atomic publication, idempotency, bi-temporal reads,
//! supersession, provenance, tenant isolation, bounded authorized traversal) before any engine work.
//! Status: written while the S14 benchmark was running; NOT YET COMPILED OR TESTED.
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evidence { Explicit, Deterministic, ModelExtracted, ModelInferred, HumanConfirmed }

/// `NotFound` is returned both for missing ids and for ids owned by another tenant (no existence leak).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error { NotFound, Invalid(&'static str) }

#[derive(Clone, Debug)]
pub struct Episode { pub id: u64, pub tenant: String, pub event_time: i64, pub payload: String, pub seq: u64 }

#[derive(Clone, Debug)]
pub struct Fact {
    pub id: u64, pub tenant: String, pub subject: u64, pub predicate: String, pub object: String,
    pub valid_from: i64, pub valid_to: Option<i64>, pub created_seq: u64,
    /// (transaction seq at which the validity interval was closed, closing valid time)
    pub closed: Option<(u64, i64)>,
    pub superseded_by: Option<u64>, pub evidence: Evidence, pub sources: Vec<u64>,
}

#[derive(Clone, Debug)]
pub struct Rel { pub id: u64, pub tenant: String, pub from: u64, pub to: u64, pub kind: String, pub sources: Vec<u64>, pub created_seq: u64 }

#[derive(Clone, Debug)]
pub struct FactSpec { pub subject: String, pub predicate: String, pub object: String, pub valid_from: i64, pub evidence: Evidence, pub sources: Vec<u64>, pub supersedes: Option<u64> }

#[derive(Clone, Debug)]
pub struct RelSpec { pub from: String, pub to: String, pub kind: String, pub sources: Vec<u64> }

#[derive(Clone, Debug)]
pub struct Batch { pub id: String, pub facts: Vec<FactSpec>, pub rels: Vec<RelSpec> }

#[derive(Default)]
pub struct Store {
    seq: u64, next_id: u64,
    episodes: BTreeMap<u64, Episode>,
    idem: BTreeMap<(String, String), u64>,
    entities: BTreeMap<(String, String), u64>,
    entity_name: BTreeMap<u64, String>,
    facts: BTreeMap<u64, Fact>,
    rels: Vec<Rel>,
    batches: BTreeMap<(String, String), u64>,
    branches: BTreeMap<(String, String), BTreeSet<u64>>,
}

impl Store {
    pub fn new() -> Self { Self::default() }
    pub fn current_seq(&self) -> u64 { self.seq }
    pub fn fact_count(&self) -> usize { self.facts.len() }
    pub fn entity_count(&self) -> usize { self.entities.len() }

    fn next(&mut self) -> u64 { self.next_id += 1; self.next_id }

    fn ensure_entity(&mut self, tenant: &str, name: &str) -> u64 {
        let key = (tenant.to_string(), name.to_string());
        if let Some(id) = self.entities.get(&key) { return *id; }
        let id = self.next();
        self.entities.insert(key, id);
        self.entity_name.insert(id, name.to_string());
        id
    }

    /// Durable capture. Same (tenant, idempotency key) returns the original episode id and changes nothing.
    pub fn append_episode(&mut self, tenant: &str, idem: &str, event_time: i64, payload: &str) -> u64 {
        let key = (tenant.to_string(), idem.to_string());
        if let Some(id) = self.idem.get(&key) { return *id; }
        self.seq += 1;
        let id = self.next();
        self.episodes.insert(id, Episode { id, tenant: tenant.to_string(), event_time, payload: payload.to_string(), seq: self.seq });
        self.idem.insert(key, id);
        id
    }

    fn source_ok(&self, tenant: &str, sources: &[u64]) -> Result<(), Error> {
        if sources.is_empty() { return Err(Error::Invalid("derived item needs at least one source")); }
        for s in sources {
            match self.episodes.get(s) { Some(e) if e.tenant == tenant => {} _ => return Err(Error::NotFound) }
        }
        Ok(())
    }

    /// Atomic derived publication: everything is validated first, then applied. A replayed batch id is a no-op.
    pub fn publish(&mut self, tenant: &str, batch: &Batch) -> Result<u64, Error> {
        let bkey = (tenant.to_string(), batch.id.clone());
        if let Some(seq) = self.batches.get(&bkey) { return Ok(*seq); }
        let mut targets = BTreeSet::new();
        for f in &batch.facts {
            if f.predicate.is_empty() || f.subject.is_empty() { return Err(Error::Invalid("empty subject or predicate")); }
            self.source_ok(tenant, &f.sources)?;
            if let Some(old) = f.supersedes {
                match self.facts.get(&old) {
                    Some(o) if o.tenant == tenant => { if o.closed.is_some() || !targets.insert(old) { return Err(Error::Invalid("fact already superseded")); } }
                    _ => return Err(Error::NotFound),
                }
            }
        }
        for r in &batch.rels {
            if r.from.is_empty() || r.to.is_empty() || r.kind.is_empty() { return Err(Error::Invalid("empty relationship field")); }
            self.source_ok(tenant, &r.sources)?;
        }
        // Validation passed: apply. Nothing below can fail.
        self.seq += 1;
        let seq = self.seq;
        for f in &batch.facts {
            let subject = self.ensure_entity(tenant, &f.subject);
            let id = self.next();
            if let Some(old) = f.supersedes {
                if let Some(o) = self.facts.get_mut(&old) { o.closed = Some((seq, f.valid_from)); o.superseded_by = Some(id); }
            }
            self.facts.insert(id, Fact { id, tenant: tenant.to_string(), subject, predicate: f.predicate.clone(), object: f.object.clone(),
                valid_from: f.valid_from, valid_to: None, created_seq: seq, closed: None, superseded_by: None, evidence: f.evidence, sources: f.sources.clone() });
        }
        for r in &batch.rels {
            let from = self.ensure_entity(tenant, &r.from);
            let to = self.ensure_entity(tenant, &r.to);
            let id = self.next();
            self.rels.push(Rel { id, tenant: tenant.to_string(), from, to, kind: r.kind.clone(), sources: r.sources.clone(), created_seq: seq });
        }
        self.batches.insert(bkey, seq);
        Ok(seq)
    }

    fn valid_to_at(f: &Fact, known_at: u64) -> Option<i64> {
        match f.closed { Some((s, t)) if s <= known_at => Some(t), _ => f.valid_to }
    }

    /// Bi-temporal read: facts valid at `valid_at` according to what Atlas knew at transaction seq `known_at`.
    pub fn facts_as_of(&self, tenant: &str, valid_at: i64, known_at: u64) -> Vec<u64> {
        self.facts.values().filter(|f| f.tenant == tenant && f.created_seq <= known_at && f.valid_from <= valid_at
            && Self::valid_to_at(f, known_at).map_or(true, |t| valid_at < t)).map(|f| f.id).collect()
    }

    /// Pairs of visible facts with the same subject and predicate, different object, overlapping validity.
    pub fn contradictions(&self, tenant: &str, known_at: u64) -> Vec<(u64, u64)> {
        let vis: Vec<&Fact> = self.facts.values().filter(|f| f.tenant == tenant && f.created_seq <= known_at).collect();
        let mut out = vec![];
        for (i, a) in vis.iter().enumerate() {
            for b in &vis[i + 1..] {
                if a.subject != b.subject || a.predicate != b.predicate || a.object == b.object { continue; }
                let (at, bt) = (Self::valid_to_at(a, known_at).unwrap_or(i64::MAX), Self::valid_to_at(b, known_at).unwrap_or(i64::MAX));
                if a.valid_from < bt && b.valid_from < at { out.push((a.id, b.id)); }
            }
        }
        out
    }

    /// Evidence chain for a fact.
    pub fn explain(&self, tenant: &str, fact_id: u64) -> Result<Vec<Episode>, Error> {
        let f = self.facts.get(&fact_id).filter(|f| f.tenant == tenant).ok_or(Error::NotFound)?;
        Ok(f.sources.iter().filter_map(|s| self.episodes.get(s).cloned()).collect())
    }

    pub fn add_to_branch(&mut self, tenant: &str, branch: &str, fact_id: u64) -> Result<(), Error> {
        self.facts.get(&fact_id).filter(|f| f.tenant == tenant).ok_or(Error::NotFound)?;
        self.branches.entry((tenant.to_string(), branch.to_string())).or_default().insert(fact_id);
        Ok(())
    }

    pub fn branch_facts(&self, tenant: &str, branch: &str) -> Vec<u64> {
        self.branches.get(&(tenant.to_string(), branch.to_string())).map(|s| s.iter().copied().collect()).unwrap_or_default()
    }

    /// Bounded neighbourhood. Only the caller tenant's relationships are ever examined (authorization before expansion).
    /// Returns (entity names in BFS order, truncated_by_node_budget). Hop-limit cut-offs are not flagged.
    pub fn neighborhood(&self, tenant: &str, entity: &str, max_hops: usize, max_nodes: usize) -> Result<(Vec<String>, bool), Error> {
        let start = *self.entities.get(&(tenant.to_string(), entity.to_string())).ok_or(Error::NotFound)?;
        let mut seen = BTreeSet::from([start]);
        let mut order = vec![start];
        let mut frontier = vec![start];
        let mut truncated = false;
        for _ in 0..max_hops {
            let mut cand = BTreeSet::new();
            for n in &frontier {
                for r in self.rels.iter().filter(|r| r.tenant == tenant) {
                    let other = if r.from == *n { r.to } else if r.to == *n { r.from } else { continue };
                    if !seen.contains(&other) { cand.insert(other); }
                }
            }
            let mut added = vec![];
            for o in cand {
                if order.len() >= max_nodes { truncated = true; break; }
                seen.insert(o); order.push(o); added.push(o);
            }
            if truncated || added.is_empty() { break; }
            frontier = added;
        }
        Ok((order.iter().map(|i| self.entity_name[i].clone()).collect(), truncated))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs(subject: &str, pred: &str, obj: &str, from: i64, src: Vec<u64>, sup: Option<u64>) -> FactSpec {
        FactSpec { subject: subject.into(), predicate: pred.into(), object: obj.into(), valid_from: from, evidence: Evidence::Explicit, sources: src, supersedes: sup }
    }
    fn batch(id: &str, facts: Vec<FactSpec>) -> Batch { Batch { id: id.into(), facts, rels: vec![] } }

    #[test]
    fn episode_capture_is_idempotent() {
        let mut s = Store::new();
        let a = s.append_episode("t", "k1", 1, "x");
        let seq = s.current_seq();
        assert_eq!(s.append_episode("t", "k1", 1, "x"), a);
        assert_eq!(s.current_seq(), seq);
        assert_ne!(s.append_episode("u", "k1", 1, "x"), a); // same key, other tenant: distinct
    }

    #[test]
    fn invalid_batch_publishes_nothing() {
        let mut s = Store::new();
        let e = s.append_episode("t", "k", 1, "x");
        let seq = s.current_seq();
        let bad = batch("b", vec![fs("atlas", "uses", "f32", 1, vec![e], None), fs("atlas", "has", "wal", 1, vec![999], None)]);
        assert_eq!(s.publish("t", &bad), Err(Error::NotFound));
        assert_eq!((s.fact_count(), s.entity_count(), s.current_seq()), (0, 0, seq));
    }

    #[test]
    fn replayed_batch_is_a_noop() {
        let mut s = Store::new();
        let e = s.append_episode("t", "k", 1, "x");
        let b = batch("b1", vec![fs("atlas", "uses", "f32", 1, vec![e], None)]);
        let s1 = s.publish("t", &b).unwrap();
        assert_eq!(s.publish("t", &b).unwrap(), s1);
        assert_eq!(s.fact_count(), 1);
    }

    #[test]
    fn supersession_keeps_history_and_bitemporal_reads() {
        let mut s = Store::new();
        let e = s.append_episode("t", "k", 1, "x");
        let s1 = s.publish("t", &batch("b1", vec![fs("atlas", "uses", "f32", 10, vec![e], None)])).unwrap();
        let a = s.facts_as_of("t", 15, s1)[0];
        let s2 = s.publish("t", &batch("b2", vec![fs("atlas", "uses", "f16", 20, vec![e], Some(a))])).unwrap();
        assert_eq!(s.facts_as_of("t", 15, s2), vec![a]);          // valid then, known now
        assert_eq!(s.facts_as_of("t", 25, s1), vec![a]);          // before we learned of the change
        let b = s.facts_as_of("t", 25, s2);
        assert_eq!(b.len(), 1);
        assert_ne!(b[0], a);                                       // after: the new fact
        assert!(s.contradictions("t", s2).is_empty());
        assert_eq!(s.publish("t", &batch("b3", vec![fs("atlas", "uses", "i8", 30, vec![e], Some(a))])), Err(Error::Invalid("fact already superseded")));
    }

    #[test]
    fn contradictions_coexist_and_are_reported() {
        let mut s = Store::new();
        let e = s.append_episode("t", "k", 1, "x");
        let sq = s.publish("t", &batch("b", vec![fs("atlas", "uses", "f32", 10, vec![e], None), fs("atlas", "uses", "f16", 12, vec![e], None)])).unwrap();
        assert_eq!(s.contradictions("t", sq).len(), 1);
        assert_eq!(s.facts_as_of("t", 13, sq).len(), 2); // both retained, neither silently dropped
    }

    #[test]
    fn provenance_chain_and_multi_branch_membership() {
        let mut s = Store::new();
        let e1 = s.append_episode("t", "k1", 1, "a");
        let e2 = s.append_episode("t", "k2", 2, "b");
        let sq = s.publish("t", &batch("b", vec![fs("atlas", "uses", "f32", 1, vec![e1, e2], None)])).unwrap();
        let f = s.facts_as_of("t", 5, sq)[0];
        assert_eq!(s.explain("t", f).unwrap().len(), 2);
        s.add_to_branch("t", "search", f).unwrap();
        s.add_to_branch("t", "decisions", f).unwrap();
        assert_eq!(s.branch_facts("t", "search"), vec![f]);
        assert_eq!(s.branch_facts("t", "decisions"), vec![f]);
    }

    #[test]
    fn tenant_isolation_has_no_existence_leak() {
        let mut s = Store::new();
        let ea = s.append_episode("a", "k", 1, "secret");
        let sq = s.publish("a", &Batch { id: "b".into(), facts: vec![fs("atlas", "uses", "f32", 1, vec![ea], None)],
            rels: vec![RelSpec { from: "atlas".into(), to: "wal".into(), kind: "has".into(), sources: vec![ea] }] }).unwrap();
        let f = s.facts_as_of("a", 5, sq)[0];
        assert_eq!(s.explain("b", f), s.explain("b", 424242));          // foreign id == missing id
        assert_eq!(s.add_to_branch("b", "x", f), Err(Error::NotFound));
        assert_eq!(s.neighborhood("b", "atlas", 2, 10), Err(Error::NotFound));
        assert!(s.facts_as_of("b", 5, sq).is_empty());
        let eb = s.append_episode("b", "k", 1, "mine");
        assert_eq!(s.publish("b", &batch("bb", vec![fs("x", "y", "z", 1, vec![ea], None)])), Err(Error::NotFound)); // cannot cite a's episode
        assert!(s.publish("b", &batch("bb", vec![fs("x", "y", "z", 1, vec![eb], None)])).is_ok());
    }

    #[test]
    fn neighborhood_is_bounded() {
        let mut s = Store::new();
        let e = s.append_episode("t", "k", 1, "x");
        let rels = ["a", "b", "c", "d", "e"].windows(2).map(|w| RelSpec { from: w[0].into(), to: w[1].into(), kind: "next".into(), sources: vec![e] }).collect();
        s.publish("t", &Batch { id: "b".into(), facts: vec![], rels }).unwrap();
        assert_eq!(s.neighborhood("t", "a", 2, 10).unwrap(), (vec!["a".to_string(), "b".into(), "c".into()], false));
        let (names, truncated) = s.neighborhood("t", "a", 4, 3).unwrap();
        assert_eq!((names.len(), truncated), (3, true));
    }

    #[test]
    fn same_operations_give_same_state() {
        let run = || {
            let mut s = Store::new();
            let e = s.append_episode("t", "k", 1, "x");
            let sq = s.publish("t", &batch("b", vec![fs("atlas", "uses", "f32", 1, vec![e], None)])).unwrap();
            (sq, s.facts_as_of("t", 5, sq))
        };
        assert_eq!(run(), run());
    }
}
