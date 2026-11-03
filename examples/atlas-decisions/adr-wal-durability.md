---
id: adr-wal-durability
kind: decision
title: Acknowledge writes only after the WAL record and its directory entry are synced
status: confirmed
date: 2026-10-26
parent: proj-atlas
depends_on: proj-atlas
---
A write is acknowledged only after the WAL record is fsynced, and file creation is made durable by syncing the directory. Process-kill tests cover acknowledged-write survival (docs/DURABILITY.md). Process kill is not power-loss evidence, so power loss stays an open item.
