# S-ANN speed plan (written 2026-11-03, before any engine code is measured)

Goal: lead a named S-ANN scoreboard (recall@10 versus latency/QPS, ann-benchmarks style) on equal hardware. No claim until `research/BENCH_CHARTER.md` is met: equal hardware, fair tuning, second-run reproduction, raw data public.

## Where we stand (measured, `docs/evidence/s14-f16-same-session.md`)
USearch adapter with F16 graph and exact F32 rescore: ahead of hnswlib on Fashion-784 (p50 0.241 vs 0.327 ms at ef 64), 1.6x behind on SIFT-128 and 1.8x behind on GloVe-100. Adapter overhead is about 1% (`benchmarks/h2h/profiling/S13_ADAPTER_PROFILE.md`), so the gap is inside USearch's traversal, which we cannot change. Control over the graph layout needs our own engine.

## Techniques to try, with their sources (all user-attested until the primary text is read for the claim used)
| Technique | Source | Reported effect (their setup, not ours) |
|---|---|---|
| Graph reordering for cache locality (RCM, Gorder) on HNSW | Coleman, Segarra, Smola, Shrivastava, NeurIPS 2022, https://arxiv.org/pdf/2104.03221 | up to 40% lower query time; p99 -17% (RCM) and -30% (Gorder) on SIFT100M |
| FINGER: approximate distances with low-rank residual angles to skip exact evaluations | Chen et al., WWW 2023, https://arxiv.org/pdf/2206.11408v1.pdf | 20% to 60% over HNSW |
| Scalar-quantized (SQ8/SQ4) traversal with SIMD, exact rescore; Glass library | https://github.com/zilliztech/pyglass | listed among top performers on ann-benchmarks; numbers not read |
| Locally-adaptive vector quantization (LVQ), Intel SVS | Aguerrebere et al., https://arxiv.org/pdf/2304.04759.pdf | up to ~8x bandwidth reduction versus float32 |
| Quantization fused with graph layout and FastScan batches (SymphonyQG) | Gou et al., SIGMOD 2025, https://arxiv.org/abs/2411.12229v1 | 1.5x to 4.5x QPS at 95% recall versus their strongest baselines |
Caveat: effects are from other hardware, datasets and baselines. Each is a hypothesis for our engine until measured.

## Engine path (our own HNSW in pure Rust, `atlas-index`, no new dependencies)
Layered, each step measured alone against the previous one and against same-session hnswlib and USearch-F16:
| Step | Change | Adopt if | Drop if |
|---|---|---|---|
| E1 | Contiguous arena (vector and level-0 neighbours in one block), M=32, ef_construction=200 to match hnswlib, epoch visited array, prefetch-friendly order | p50 at recall@10 >= 0.95 within 1.15x of hnswlib on GloVe-100 and SIFT-128 | slower than 1.5x: redesign before adding more |
| E2 | Graph reordering after build (BFS and RCM variants) | >= 10% lower p50 at equal recall | < 10% |
| E3 | SQ8 traversal (u8 codes, integer dot or L2) with exact F32 rescore of the top ef | >= 20% lower p50 and recall@10 within 0.002 at equal ef | recall loss > 0.002 |
| E4 | FINGER-style skip test on level 0 | >= 15% lower p50 at equal recall | < 15% |
| E5 | Parallel build (batched insert with locks) | build time <= hnswlib at 8 threads, recall unchanged | recall loss |
Correctness gates for every step: `atlas_index::conformance::run` for exact-result invariants, recall against `FlatIndex`, zero filter leaks, determinism of a single-threaded build.

## Measurement protocol
Same host, idle-host check recorded in the manifest, interleaved repetitions equal for all engines, build parameters equal (M=32, ef_construction=200), datasets GloVe-100, SIFT-128, NYTimes-256, Fashion-784. Raw output saved unedited under `benchmarks/h2h/profiling/raw/`. A step that misses its adopt criterion is reported as a miss.

## Public entry
`integrations/ann-benchmarks/atlas/` (module.py, Dockerfile, config.yml) over the `adapters/atlas-py` module, targeting https://github.com/erikbern/ann-benchmarks (add `ann_benchmarks/algorithms/atlas/`, plus the CI line). The entry is submitted only with the best measured engine; it measures the embedded index (no WAL, no API), and the submission text must say so.

## Added 2026-11-03 (second research pass and our own hypotheses)
| Technique | Source / origin | Note |
|---|---|---|
| Two-pass neighbour evaluation with software prefetch: first pass filters visited neighbours and issues a prefetch per unvisited one, second pass computes distances | Manticore write-up, https://dev.to/sanikolaev/faster-knn-search-in-manticore-2-pass-hnsw-batched-distances-and-avx-512-4gd2 ; k-means-on-HNSW paper, https://arxiv.org/pdf/2502.06163 | A vendor claims 10 to 20% from prefetch (docs.rs velesdb-core); no controlled single-thread x86 isolation was found, so we measure it |
| Flat array visited marks and flat vector storage are what let Lucene reach hnswlib parity | Lucene dev list, https://mail-archive.com/dev@lucene.apache.org/msg317876.html | already done in E1 |
| Query-adaptive early termination: patience, learned stop (LAET), recall-predictor (DARTH), per-query ef (Ada-ef) | Elastic, https://www.elastic.co/search-labs/blog/hnsw-elasticsearch-adaptive-early-termination ; LAET, https://conglongli.github.io/paper/ann-sigmod2020.pdf ; DARTH, https://www.themoonlight.io/review/darth-declarative-recall-through-early-termination-for-approximate-nearest-neighbor-search ; Ada-ef, https://arxiv.org/pdf/2512.06636 | moves the whole recall/latency frontier, not one point |

| Step | Change | Adopt if | Drop if |
|---|---|---|---|
| E6 | Two-pass neighbour evaluation with `_mm_prefetch` (x86_64, gated, scalar fallback elsewhere) | >= 8% lower p50 at equal recall | < 8% |
| E7 | Patience stop: end level-0 search when the result set's k-th distance has not improved for P expansions (P tuned per dataset on a held-out query split, never on test queries) | >= 20% lower p50 at recall@10 within 0.003 of the full-ef run at the same nominal ef | recall loss > 0.003 |
| E8 (our hypothesis) | Replace the upper hierarchy with a navigation set: a sample of about sqrt(n) points scanned with SIMD to pick the entry point | lower p50 than the layered descent at equal recall, or equal with simpler/cheaper build | recall loss or slower |
| E9 (our hypothesis) | Node-block layout `[count, neighbour ids, vector]` so the neighbour list and the vector of one node share cache lines; combined with E2 reordering | >= 10% lower p50 over E2 | < 10% |
Order by expected gain per effort: E1 (done in code), E6, E7, E3, E2, E9, E8, E4, E5. Every step re-runs the correctness gates.

## E7 result (patience early stop): DROPPED

NYTimes, own F16 HNSW, one build, one session (08:34 IST 2026-11-03). Criterion: p50 at least 20% lower with recall within 0.003. Observed: pat16/32/64 are 0% to 30% slower at ef 64-512 (ef256 p50: base 1.55 ms, pat16 1.76, pat32 1.95, pat64 1.97); pat16 loses 0.002-0.003 recall. Raw: benchmarks/h2h/profiling/raw/s14/nytimes/ab/own_f16_patience.json. Patience stays default off. Next: E3 (SQ8 traversal), E5 (parallel build).
