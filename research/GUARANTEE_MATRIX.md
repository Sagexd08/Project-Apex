# Guarantee matrix (re-graded against this repo)
| ID | Guarantee | Code today | Grade | Gap | Experiment | Change justified |
|---|---|---|---|---|---|---|
| GM-01 | recall at bounded RAM | atlas-usearch F32/F16 adapter | A (s9 evidence) | no quantized tier decision | EXP-01/03 | not yet |
| GM-02 | filtered ANN | planner `use_exact_scan` in adapter; sift/nytimes-planner results | A partial | acceptance (recall>=0.95 at 1%, 0.1%) not shown on spike data | EXP-02/05 | planner exists; calibrate |
| GM-03 | temporal range | none | D | no range path | EXP-02 range track | defer |
| GM-04 | churn resilience | USearch soft delete | D | no churn test | EXP-04 | not yet |
| GM-05 | streaming visibility | WAL + segments (S10-S13) | A for durability; D for delta/watermark | no delta buffer | EXP-07 | design at E3 |
| GM-06 | recovery, amplification | WAL v3, kill tests | A (process kill only) | power loss, macOS open | EXP-06 | no |
| GM-07 | benchmark reproducibility | h2h + manifest gate | A (manifest enforced) | idle host, equal reps not yet enforced in run.sh | EXP-08 | harness work (Phase 2) |
