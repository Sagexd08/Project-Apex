---
id: obs-wal-v3-review-defect
kind: observation
title: Independent review of the WAL v3 digest found 1 critical and 3 high defects
status: observed
date: 2026-10-28
parent: proj-atlas
disputes: adr-segments-checkpoint
---
Code inspection only, no tests run. The critical finding concerned a stale recovery-marker offset after the prefix cut. Report: docs/evidence/review-wal-v3-dd680f64.md.
