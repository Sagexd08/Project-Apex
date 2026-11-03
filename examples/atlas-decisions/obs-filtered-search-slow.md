---
id: obs-filtered-search-slow
kind: observation
title: Graph-walk filtered search is too slow at low selectivity
status: observed
date: 2026-10-27
parent: proj-atlas
derived_from: exp-ann-spike
---
At 1 percent selectivity filtered p95 was 49.7 ms and at 0.1 percent it was 417.7 ms, far above the unfiltered 1.73 ms. The graph walk visits many nodes that fail the filter.
