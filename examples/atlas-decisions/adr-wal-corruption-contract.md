---
id: adr-wal-corruption-contract
kind: decision
title: Recovery is fail-closed with distinct outcomes for torn tail, bad final record and mid-log corruption
status: confirmed
date: 2026-10-26
parent: proj-atlas
derived_from: adr-wal-durability
---
Complete final record with a bad checksum returns RECOVERY_REQUIRED and leaves the WAL unchanged. An incomplete final record is truncated and the bytes are saved to a .bak sidecar on repair. Corruption in the middle of the log returns WAL_CORRUPT and bytes are never altered. See docs/WAL_FORMAT.md.
