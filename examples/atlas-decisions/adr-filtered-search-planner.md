---
id: adr-filtered-search-planner
kind: decision
title: Pre-filter the allowed set and run an exact scan below about 10 percent selectivity
status: confirmed
date: 2026-10-28
parent: proj-atlas
caused_by: obs-filtered-search-slow
derived_from: exp-ann-spike
---
Acceptance criteria: recall@10 at least 0.95 at 1 percent and 0.1 percent selectivity, measured against the exact FlatIndex oracle. A cost rule in the USearch adapter chooses exact scan or graph walk. The acceptance run on the spike dataset is still open (docs/GATES.md).
