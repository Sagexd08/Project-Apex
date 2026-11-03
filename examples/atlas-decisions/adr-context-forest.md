---
id: adr-context-forest
kind: decision
title: Model knowledge as a Living Context Forest on top of the engine public API
status: confirmed
date: 2026-10-30
parent: proj-atlas
depends_on: adr-wal-durability
relates_to: adr-outcome-unknown
---
Objects and typed relationships are canonical engine records, so durability, recovery and tenant isolation are inherited. A change is a new object plus a SUPERSEDES relationship; nothing is overwritten. Reads come from a derived index and can be pinned to a version. See docs/QUICKSTART_DEMO.md.
