---
id: adr-outcome-unknown
kind: decision
title: A failed recovery-marker write poisons the engine with OUTCOME_UNKNOWN and callers retry by operation id
status: confirmed
date: 2026-10-26
parent: proj-atlas
derived_from: adr-wal-corruption-contract
---
When the outcome of a write cannot be known the engine stops accepting writes until reopen. The operation may or may not exist after restart. Callers use insert_op with an operation id so a retry never double-applies; the same id with a different payload is a conflict.
