---
id: adr-segments-checkpoint
kind: decision
title: Checkpoints write a segment and cut the WAL prefix; WAL format version 3 carries base_seq
status: confirmed
date: 2026-10-28
parent: proj-atlas
depends_on: adr-wal-durability
relates_to: adr-wal-corruption-contract
---
Engine checkpoint writes a segment and truncates the WAL prefix up to it so recovery replays only the tail. WAL v3 adds base_seq and stays read-compatible with v2. See docs/SEGMENTS.md and docs/WAL_FORMAT.md.
