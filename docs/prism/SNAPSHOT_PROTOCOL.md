# Prism: snapshot protocol (Stage C, 2026-10-27)

Status: **Proposed**. No model, prototype or crash test exists for this protocol. Research Note v1.0, not peer reviewed.

## S.1 Why a log position is not a snapshot

A log sequence number (LSN) says how far the log has grown. A snapshot says which immutable index state a reader may use. They coincide only if the system defines and checks the relationship between durable records, applied records, published segments, deletions and reclamation. The reference systems keep that relationship explicit: RocksDB pins a snapshot to a sequence number and makes compaction preserve what live snapshots see; SQLite readers fix a WAL end mark and a checkpoint stops at it; RCU defers reclamation until pre-existing readers finish (official documentation, read in Stage B).

## S.2 Objects

| Object | Meaning |
|---|---|
| Record `r` | An insert or delete with `lsn(r)`, tenant, id, vector, metadata |
| Durable LSN `D` | Highest LSN that reached the durability boundary; acknowledgement may use it |
| Applied LSN `A <= D` | Highest LSN applied to the writer's in-memory state |
| Segment `g` | Immutable once sealed; holds records with `lsn in [lo_g, hi_g]` |
| Snapshot `S` | `(gen, lsn_S, segments)`, immutable after publication |
| Pin `p = (epoch, S)` | A reader's reference to a published snapshot; `epoch = (seed, d, schedule)` |
| Retired segment | Replaced by compaction; reclaimable once no pin references it |

## S.3 Legal transitions

```
Appended -> Durable -> Applied -> Sealed -> Published -> Retired -> Reclaimed
```

1. **Appended to Durable:** the record reached the configured durability boundary. The write may be acknowledged. Not visible to readers.
2. **Durable to Applied:** applied to the writer's state in LSN order. Visibility to the writer's own reads is a choice (read-your-writes); any lag must be stated, not implied.
3. **Applied to Sealed:** the segment is complete and immutable; its codes and full-precision vectors are both present.
4. **Sealed to Published:** one atomic pointer swap installs a new snapshot listing the segment. Only a **fully sealed** segment may be listed, so no reader sees a partial one.
5. **Published to Retired:** compaction replaces segments with one holding the same visible records.
6. **Retired to Reclaimed:** no live pin references the segment.

Illegal: publishing a segment that is not durable; reclaiming a segment a pin still references; a pin to an unpublished snapshot; a snapshot whose `lsn_S` exceeds `D`.

## S.4 Visible set and invariants

```
V(S) = { r : lsn(r) <= lsn_S  and  not deleted at or before lsn_S }
```

| Id | Invariant |
|---|---|
| I1 | Repeatability: for one pin and one query the result is identical every time |
| I2 | No torn read: a commit is visible whole or not at all (codes and vector together) |
| I3 | Monotonic pins: successive pins by one reader never decrease `lsn_S` |
| I4 | Compaction transparency: compaction never changes `V(S)` |
| I5 | Durability bound: `lsn_S <= D` for every published snapshot |
| I6 | Tenant closure: the visible set of one tenant never contains another tenant's record |
| I7 | Eligible-set threshold: the pruning threshold `tau` is computed over the filter-passing subset of `V(S)` only |

## S.5 Recovery obligation

After a crash at any step, recovery must rebuild exactly the snapshots the durable log can justify, and discard any partially sealed segment. A snapshot whose segment list cannot be re-derived from the log must not be published. The index is derived data: it is never trusted over the log.

## S.6 Reclamation and backpressure (undefined, open)

- A pin that is never released blocks reclamation. SQLite documents the same failure: overlapping readers can stop checkpoints and let the WAL grow without bound. Prism needs a pin lease and an expiry rule, and a defined error when a reader outlives its lease.
- Retained-version bytes need a cap with writer backpressure; the cap, the error and the test are not designed.
- Delete semantics for a pinned reader (when a tombstone is reclaimable) are not specified beyond the formula in S.4.

## S.7 Tests the protocol requires (none exist yet)

1. A deterministic state-machine model with a crash injected at every transition, checking I1 to I7 after recovery.
2. A history check (linearizability-style) over concurrent readers, writers and compaction.
3. A long-lived pin test that fills the retention cap and observes backpressure.
4. A differential test: results from a pinned snapshot equal an exact scan of `V(S)` for the same eligible set.

## S.8 Relation to the Atlas engine

The production Atlas engine uses a simpler design: the write-ahead log is the single source of truth, the index is rebuilt from it on open, and a corrupt or poisoned state fails closed. It has no pinned readers or segments. This protocol is research, and no part of it changes the Atlas WAL contract or Gate 2 evidence.
