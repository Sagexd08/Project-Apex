# Pre-registered experiments (acceptance written before runs)
Details and datasets: Stage B report Deliverable 4; run order in `docs/BENCHMARK_NUMBER_ONE_PLAN.md`. Numeric criteria fixed now:
| EXP | Adopt if | Drop if |
|---|---|---|
| 01 F16 default | Recall@10 loss < 0.003 vs F32 and index RAM down >= 40% | loss >= 0.003 |
| 01 RaBitQ/INT8 over OPQ | matches or beats PQ/OPQ Pareto on OOD without training | otherwise keep as baseline only |
| 02 planner | find sigma_crit where exact scan has lower p99 at recall@10 >= 0.98 | no crossover found |
| 03 two-tier | INT8+F32 rescore (<=4x) recall@10 >= 0.99, p99 warm < 1.5 ms, cold < 4 ms, <= 30% of F32 RAM | any miss |
| 04 segmented churn | within 0.5% recall and 15% p99 of fresh rebuild over 10 cycles | any miss |
| 05 cost model | picks best plan (or within 10% latency) on > 95% of grid | otherwise table lookup |
| 07 delta buffer | 0 ms commit-to-visible, 0 isolation violations in 1e6 ops, reader p99 < 5 ms | any violation |
| 06 amplification | A_space <= 2.2x, A_write <= 4x | any miss |
| 08 reproducibility | <3% QPS variance, identical recall | otherwise not publishable |
Gates: 01-05 after S14/E0; 06-07 after E5; 08 at E7. Hardware numbers in the report are hypotheses until measured at a digest.

## S-DB and S-MEM experiments (registered 2026-11-03, before any run)
Same claim rule as the charter: equal hardware, idle-host manifest, engines run interleaved, median of at least 5 repetitions, raw output saved unedited, Atlas side tied to `scripts/digest.sh`. A result that fails its criterion is published as a loss.
| EXP | Question | Engines | Metric | Atlas leads if | Drop if |
|---|---|---|---|---|---|
| 09 recovery | Time from SIGKILL to first correct query, N=100k and 1M vectors, no clean shutdown | Atlas, Qdrant (docker, same data, same cores) | restart-to-ready ms, median of 5; acked-write loss count | median restart-to-ready is lower than Qdrant at both N and loss is 0 | Qdrant is lower at either N (report as a loss) |
| 10 acked-write durability | Writer acks N inserts, SIGKILL mid-stream, reopen, count survivors | Atlas, Qdrant (wait_for=true) | acked-but-missing count over 20 kills | 0 missing across all kills and Qdrant is not better | any missing acked write in Atlas (a defect, not a loss) |
| 11 commit-to-visible | Latency from insert ack to the point being returned by search | Atlas, Qdrant | p50/p99 ms over 10k inserts | p99 lower than Qdrant | otherwise report |
| 12 tenant isolation | Cross-tenant leaks under concurrent filtered search, 1e6 queries | Atlas, Qdrant (payload filter) | leaked results | 0 leaks for Atlas (a correctness result, not a speed ranking) | any leak |
| 13 memory task | Paraphrase retrieval of the right decision record from the 13-doc ADR set plus a larger synthetic set, 50 questions with no shared keywords | Atlas LCF + MiniLM, vector-only top-k with the same embedder, hash embedder control | hit@1, hit@3, tokens sent to the model | LCF hit@3 >= vector-only hit@3 and tokens sent lower | LCF hit@3 below vector-only |
| 14 sustained acked-write throughput (registered 2026-11-03 after EXP-10 showed a possible gap; not yet run) | Acked single-point inserts per second with 16 concurrent writers, 30 s windows, N=20k base, 4 CPUs and 6 GB each | Atlas, Qdrant (wait=true) | median acked writes/s of 5 windows per stage, ABBA, 0 acked loss after a SIGKILL at the end of each window | Atlas median at least 10% higher at equal or better durability evidence | Atlas not at least 10% higher (report as a tie or loss); durability semantics of both ack paths must be stated in the result |
Honest notes: EXP-09 on Atlas includes the WAL replay and index rebuild (23 to 33 s at 200k records with HNSW; exact scan is faster to open); Qdrant restarts from persisted segments, so Atlas may lose. EXP-13 questions and answer keys must be written and frozen before the first run.
