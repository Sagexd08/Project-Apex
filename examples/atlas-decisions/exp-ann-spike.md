---
id: exp-ann-spike
kind: experiment
title: USearch spike at 1M vectors x 768 dimensions
status: observed
date: 2026-10-27
parent: proj-atlas
---
Synthetic clustered data, cosine. Recall@10 0.964 at expansion_search 128. Single-thread p95 1.73 ms against a 15 ms target. Filtered search p95 was 49.7 ms at 1 percent selectivity and 417.7 ms at 0.1 percent selectivity. Details in docs/ANN_SPIKE.md; the run was on a non-idle machine.
