//! Deterministic demo scenario, written through the real write path (`put_node`/`put_edge`).
//! Idempotent: re-running against an existing database is a no-op (same payloads, same op ids).
use crate::model::*;
use crate::store::Lcf;
use atlas_core::EffectiveScope;

const T0: i64 = 1_788_000_000; // fixed so seeding is reproducible

// (key, kind, title, body, source, status, day offset)
type N = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    i64,
);

const NODES: [N; 28] = [
    ("ws-acme", "workspace", "Acme platform", "Workspace for the Acme engineering agents.", "seed", "asserted", 0),
    ("proj-orders", "project", "Orders service", "Order intake and fulfilment service.", "seed", "asserted", 0),
    ("proj-search", "project", "Search service", "Catalogue search and indexing service.", "seed", "asserted", 0),
    ("agent-ada", "agent", "Agent Ada", "Performance engineering agent working on Orders.", "seed", "asserted", 0),
    ("agent-bo", "agent", "Agent Bo", "Storage prototyping agent working on Search.", "seed", "asserted", 0),
    ("dom-engineering", "entity", "Engineering domain", "Shared engineering concepts.", "seed", "asserted", 0),
    ("concept-event-sourcing", "entity", "Event sourcing", "Store state changes as an append-only event log and derive state by projection.", "wiki:event-sourcing", "asserted", 1),
    ("concept-checkpointing", "entity", "Checkpointing", "Periodically persist derived state so replay starts from the checkpoint, not from the first event.", "wiki:checkpointing", "asserted", 1),
    ("ep-orders-perf", "episode", "Orders write-latency investigation", "Sprint 14 investigation into order write latency.", "tracker:ORD-212", "observed", 3),
    ("req-orders-latency", "fact", "Orders write p99 must stay under 20 ms at 2000 writes/s", "Product requirement from the checkout team.", "doc:checkout-slo-v3", "confirmed", 3),
    ("dec-inplace", "decision", "Use in-place row updates in Postgres", "Original design: update order rows in place inside one transaction.", "adr:ADR-007", "asserted", 3),
    ("exp-pg-bench", "experiment", "Postgres single-node benchmark at 2000 writes/s", "Ran the order write path on a single Postgres node. Measured p99 41 ms; lock contention on hot order_items rows.", "bench:run-2026-09-12", "observed", 14),
    ("obs-lock-contention", "observation", "Lock contention on order_items hot rows", "p99 write latency of 41 ms exceeds the 20 ms budget; waits concentrated on a few hot rows.", "log:orders-api-2026-09-12", "observed", 14),
    ("evd-bench-log", "evidence", "Benchmark log bench-2026-09-12", "Raw latency histogram: p50 8 ms, p99 41 ms, max 190 ms.", "file:bench-2026-09-12.txt", "confirmed", 14),
    ("dec-eventlog", "decision", "Adopt an append-only event log with projections", "Replace in-place updates with an event log and projections to remove row lock contention.", "adr:ADR-011", "confirmed", 16),
    ("ep-eventlog", "episode", "Event-log write-path prototype", "Search team prototype of an append-only write path.", "tracker:SRCH-88", "observed", 18),
    ("exp-eventlog", "experiment", "Event-log prototype at 2000 writes/s", "Prototype append-only write path. Measured p99 9 ms at 2000 writes/s. Replay time grows with log length.", "bench:run-2026-09-20", "observed", 22),
    ("obs-slow-replay", "observation", "Replay takes 14 minutes after 30 million events", "Recovery by full replay is too slow for the 5 minute recovery objective.", "log:search-proto-2026-09-21", "observed", 23),
    ("dec-checkpoints", "decision", "Add periodic checkpoints to bound replay time", "Persist a projection checkpoint every 1 million events so recovery replays only the tail.", "adr:ADR-014", "confirmed", 25),
    ("fact-ckpt-1m", "fact", "Checkpoint interval is 1 million events", "Initial checkpoint interval chosen from the replay measurement.", "adr:ADR-014", "asserted", 25),
    ("exp-ckpt", "experiment", "Checkpoint interval sweep", "Compared 250k, 1M and 4M event intervals. 250k kept recovery under 3 minutes with 2 percent write overhead.", "bench:run-2026-10-02", "observed", 33),
    ("fact-ckpt-250k", "fact", "Checkpoint interval is 250 thousand events", "Interval reduced to 250k events after the sweep; recovery 2.6 minutes.", "adr:ADR-016", "confirmed", 34),
    ("art-orders-v2", "artifact", "orders-service v2 event store", "Implementation of the event log, projections and checkpoints (commit a1b2c3d).", "git:orders-service@a1b2c3d", "confirmed", 36),
    ("obs-open-compaction", "observation", "Open problem: log growth without compaction", "Disk use grows without bound; compaction of old events is not designed yet.", "tracker:ORD-301", "observed", 40),
    ("evd-sweep-log", "evidence", "Checkpoint sweep results", "Table of recovery time and write overhead for the three intervals.", "file:ckpt-sweep.csv", "confirmed", 33),
    ("obs-hotspot-search", "observation", "Search index shows similar hot-key contention", "Index update path in Search shows the same hot-key pattern seen in Orders.", "log:search-2026-10-05", "observed", 36),
    ("dec-search-adopt", "decision", "Evaluate event log for the search index update path", "Reuse the Orders event-log design for Search index updates, pending a prototype.", "adr:ADR-018", "inferred", 38),
    ("ep-sync", "episode", "Cross-team sync on write-path designs", "Weekly sync where Ada and Bo compared the Orders and Search write paths.", "meeting:2026-10-06", "observed", 37),
];

// (from, rel, to, source, status)
type E = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

const EDGES: [E; 53] = [
    ("ws-acme", "CONTAINS", "proj-orders", "seed", "asserted"),
    ("ws-acme", "CONTAINS", "proj-search", "seed", "asserted"),
    ("ws-acme", "CONTAINS", "agent-ada", "seed", "asserted"),
    ("ws-acme", "CONTAINS", "agent-bo", "seed", "asserted"),
    ("ws-acme", "CONTAINS", "dom-engineering", "seed", "asserted"),
    (
        "dom-engineering",
        "CONTAINS",
        "concept-event-sourcing",
        "seed",
        "asserted",
    ),
    (
        "dom-engineering",
        "CONTAINS",
        "concept-checkpointing",
        "seed",
        "asserted",
    ),
    (
        "proj-orders",
        "CONTAINS",
        "ep-orders-perf",
        "seed",
        "asserted",
    ),
    ("proj-search", "CONTAINS", "ep-eventlog", "seed", "asserted"),
    ("proj-search", "CONTAINS", "ep-sync", "seed", "asserted"),
    (
        "ep-orders-perf",
        "CONTAINS",
        "req-orders-latency",
        "tracker:ORD-212",
        "asserted",
    ),
    (
        "ep-orders-perf",
        "CONTAINS",
        "dec-inplace",
        "tracker:ORD-212",
        "asserted",
    ),
    (
        "ep-orders-perf",
        "CONTAINS",
        "exp-pg-bench",
        "tracker:ORD-212",
        "asserted",
    ),
    (
        "ep-orders-perf",
        "CONTAINS",
        "obs-lock-contention",
        "tracker:ORD-212",
        "asserted",
    ),
    (
        "ep-orders-perf",
        "CONTAINS",
        "evd-bench-log",
        "tracker:ORD-212",
        "asserted",
    ),
    (
        "ep-orders-perf",
        "CONTAINS",
        "dec-eventlog",
        "tracker:ORD-212",
        "asserted",
    ),
    (
        "ep-orders-perf",
        "CONTAINS",
        "art-orders-v2",
        "tracker:ORD-212",
        "asserted",
    ),
    (
        "ep-eventlog",
        "CONTAINS",
        "exp-eventlog",
        "tracker:SRCH-88",
        "asserted",
    ),
    (
        "ep-eventlog",
        "CONTAINS",
        "obs-slow-replay",
        "tracker:SRCH-88",
        "asserted",
    ),
    (
        "proj-orders",
        "CONTAINS",
        "dec-checkpoints",
        "seed",
        "asserted",
    ),
    (
        "proj-orders",
        "CONTAINS",
        "fact-ckpt-1m",
        "seed",
        "asserted",
    ),
    ("proj-orders", "CONTAINS", "exp-ckpt", "seed", "asserted"),
    (
        "proj-orders",
        "CONTAINS",
        "fact-ckpt-250k",
        "seed",
        "asserted",
    ),
    (
        "proj-orders",
        "CONTAINS",
        "evd-sweep-log",
        "seed",
        "asserted",
    ),
    (
        "proj-orders",
        "CONTAINS",
        "obs-open-compaction",
        "seed",
        "asserted",
    ),
    (
        "proj-search",
        "CONTAINS",
        "obs-hotspot-search",
        "seed",
        "asserted",
    ),
    (
        "proj-search",
        "CONTAINS",
        "dec-search-adopt",
        "seed",
        "asserted",
    ),
    (
        "agent-ada",
        "RELATES_TO",
        "exp-pg-bench",
        "bench:run-2026-09-12",
        "observed",
    ),
    (
        "agent-ada",
        "RELATES_TO",
        "exp-ckpt",
        "bench:run-2026-10-02",
        "observed",
    ),
    (
        "agent-bo",
        "RELATES_TO",
        "exp-eventlog",
        "bench:run-2026-09-20",
        "observed",
    ),
    (
        "exp-pg-bench",
        "CAUSED_BY",
        "dec-inplace",
        "bench:run-2026-09-12",
        "observed",
    ),
    (
        "obs-lock-contention",
        "DERIVED_FROM",
        "exp-pg-bench",
        "log:orders-api-2026-09-12",
        "observed",
    ),
    (
        "evd-bench-log",
        "SUPPORTS",
        "obs-lock-contention",
        "file:bench-2026-09-12.txt",
        "confirmed",
    ),
    (
        "evd-bench-log",
        "SUPPORTS",
        "exp-pg-bench",
        "file:bench-2026-09-12.txt",
        "confirmed",
    ),
    (
        "dec-eventlog",
        "CAUSED_BY",
        "obs-lock-contention",
        "adr:ADR-011",
        "confirmed",
    ),
    (
        "dec-eventlog",
        "DERIVED_FROM",
        "exp-pg-bench",
        "adr:ADR-011",
        "confirmed",
    ),
    (
        "dec-eventlog",
        "DEPENDS_ON",
        "req-orders-latency",
        "adr:ADR-011",
        "confirmed",
    ),
    (
        "dec-eventlog",
        "SUPERSEDES",
        "dec-inplace",
        "adr:ADR-011",
        "confirmed",
    ),
    (
        "dec-eventlog",
        "RELATES_TO",
        "concept-event-sourcing",
        "adr:ADR-011",
        "asserted",
    ),
    (
        "exp-eventlog",
        "SUPPORTS",
        "dec-eventlog",
        "bench:run-2026-09-20",
        "observed",
    ),
    (
        "exp-eventlog",
        "RELATES_TO",
        "exp-pg-bench",
        "bench:run-2026-09-20",
        "observed",
    ),
    (
        "obs-slow-replay",
        "CAUSED_BY",
        "exp-eventlog",
        "log:search-proto-2026-09-21",
        "observed",
    ),
    (
        "obs-slow-replay",
        "DISPUTES",
        "dec-eventlog",
        "log:search-proto-2026-09-21",
        "observed",
    ),
    (
        "dec-checkpoints",
        "DERIVED_FROM",
        "obs-slow-replay",
        "adr:ADR-014",
        "confirmed",
    ),
    (
        "dec-checkpoints",
        "DEPENDS_ON",
        "dec-eventlog",
        "adr:ADR-014",
        "confirmed",
    ),
    (
        "dec-checkpoints",
        "RELATES_TO",
        "concept-checkpointing",
        "adr:ADR-014",
        "asserted",
    ),
    (
        "fact-ckpt-250k",
        "SUPERSEDES",
        "fact-ckpt-1m",
        "adr:ADR-016",
        "confirmed",
    ),
    (
        "fact-ckpt-250k",
        "DERIVED_FROM",
        "exp-ckpt",
        "adr:ADR-016",
        "confirmed",
    ),
    (
        "evd-sweep-log",
        "SUPPORTS",
        "fact-ckpt-250k",
        "file:ckpt-sweep.csv",
        "confirmed",
    ),
    (
        "art-orders-v2",
        "DERIVED_FROM",
        "dec-checkpoints",
        "git:orders-service@a1b2c3d",
        "confirmed",
    ),
    (
        "obs-open-compaction",
        "DISPUTES",
        "art-orders-v2",
        "tracker:ORD-301",
        "observed",
    ),
    (
        "dec-search-adopt",
        "DERIVED_FROM",
        "dec-eventlog",
        "adr:ADR-018",
        "inferred",
    ),
    (
        "dec-search-adopt",
        "CAUSED_BY",
        "obs-hotspot-search",
        "adr:ADR-018",
        "inferred",
    ),
];

// Extra cross-branch links (Search <-> Orders).
const CROSS: [E; 2] = [
    (
        "obs-hotspot-search",
        "RELATES_TO",
        "obs-lock-contention",
        "log:search-2026-10-05",
        "observed",
    ),
    (
        "ep-sync",
        "RELATES_TO",
        "dec-search-adopt",
        "meeting:2026-10-06",
        "observed",
    ),
];

/// Writes the scenario; returns (nodes, edges) written or already present.
pub fn seed_demo(l: &Lcf, sc: &EffectiveScope) -> Res<(usize, usize)> {
    for (key, kind, title, body, source, status, day) in NODES {
        let n = Node {
            key: key.into(),
            kind: kind.into(),
            title: title.into(),
            body: body.into(),
            source: source.into(),
            status: status.into(),
            ts: T0 + day * 86_400,
            ver: 0,
        };
        l.put_node(sc, &n)?;
    }
    let mut count = 0;
    for (from, rel, to, source, status) in EDGES.iter().chain(CROSS.iter()) {
        let ts = l.get_node(sc, from)?.map(|n| n.ts).unwrap_or(T0);
        let e = Edge {
            rel: (*rel).into(),
            from: (*from).into(),
            to: (*to).into(),
            source: (*source).into(),
            ts,
            status: (*status).into(),
            ver: 0,
        };
        l.put_edge(sc, &e)?;
        count += 1;
    }
    Ok((NODES.len(), count))
}
