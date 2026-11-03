# S13 adapter profile: where the Atlas vs hnswlib gap comes from

Diagnostic only. Not a production benchmark and not recovery acceptance. S9 evidence in `results/` is untouched.

## Conditions
- Source digest (`scripts/digest.sh`, core crates): `files=60 digest=9b97729a3ea10a342c6192cf1db1fd86`. Profiler lives in `benchmarks/h2h/atlas-bench/src/bin/profile.rs` (not in the digest).
- Same Docker image as S9 (`sha256:5eedccbc...`, rust 1.99.1, `--cpus 8`), host Ryzen 5 7640HS, WSL2, 12 vCPU, other containers running (see `results/glove/config.json`).
- USearch 2.26.6, connectivity 32, expansion_add 200 (identical to adapter). SIMD dispatch available: serial, haswell, skylake, icelake, genoa (NumKong), so no scalar fallback.
- Single thread, 2000 queries, 200 warm-up, median of 3 passes, per-query timing (no per-candidate clocks), K=10. Graph built in parallel (8 threads) and cached; search cost does not depend on build threads.
- Recall vs the S9 exact ground truth (`gt.bin`). Raw: `raw/<ds>.json`, `raw/<ds>.log`. Command: `bash benchmarks/h2h/run.sh <ds> profile --ef 64,256,512 --queries 2000 --reps 3 [--ip] [--f16]`.
- The rescore stage is a line-for-line replica of `UsearchIndex::rescore`. On fashion (n<=100k) the real `UsearchIndex::search` was timed too: 1.608 ms vs replica 1.646 ms at ef=512 (replica valid).
- hnswlib figures below are the S9 p50 values (not re-run this session; different run, p50 vs mean). A same-session hnswlib baseline is still needed before quoting final ratios.

## Stage results at ef=512 (mean ms/query)
| Dataset (dim) | Native USearch search | Exact rescore | Full adapter | Recall@10 | Fixed per-call cost | Allocs/query |
|---|---|---|---|---|---|---|
| fashion (784) | 1.555 | 0.010 | 1.525 | 0.9994 | 0.020 | 5 (1.4 KB) |
| glove (100) | 3.710 | 0.005 | 3.680 | 0.9633 | 0.015 | 5 |
| nytimes (256) | 5.604 | 0.014 | 5.620 | 0.9574 | 0.029 | 5 |
| sift (128) | 2.320 | 0.006 | 2.297 | 0.9996 | 0.007 | 5 |

p50/p95/p99 for every stage and ef are in the raw JSON. Bytes copied per query: 120 B out of FFI (10 keys + distances) plus 10 x dim x 4 B read by rescoring; metadata is `Value::Null` in this bench, so the metadata clone cost is understated for payload-heavy workloads.

## Bottleneck (measured)
- Native `Index::search` is 99%+ of adapter latency on every dataset. Rescoring is 0.2-0.6%, allocations 5/query, FFI/fixed cost 0.5-5% (tiny-index probe, includes thread-context and Matches allocation).
- So batching FFI, trimming candidate copies, partial top-k and allocation reuse can recover at most about 1-2% of latency. They are not worth doing for this gap.
- The Atlas-vs-hnswlib gap is inside USearch's traversal. At ef=512 (Atlas mean / S9 hnswlib p50): fashion 1.74x, glove 2.24x, nytimes 2.38x, sift 2.17x. Nytimes and glove are the worst, sift third.
- Build-time gap is mostly a harness asymmetry: hnswlib built with 8 threads, Atlas serial (`build_threads: 1`); 7x glove build gap is about 8x threads.

## Levers tested
| Lever | Result |
|---|---|
| F16 graph + exact f32 rescore from `FlatIndex` | Faster at equal recall (ef=512): fashion 1.555 -> 1.021 ms (-34%), glove 3.71 -> 3.10 (-16%), nytimes 5.60 -> 4.40 (-21%), sift 2.32 -> 1.80 (-22%). Recall within 0.001 of f32 at ef=512 (glove ef=64 -0.005; nytimes +0.004). Remaining ratio to hnswlib: 1.16x fashion, 1.89x glove, 1.86x nytimes, 1.70x sift. |
| Angular: IP on normalized data instead of Cos | glove -7% (ef=512), nytimes -5%; ef=256 glove -19% looks noisy. Small; recall unchanged. Needs adapter-side normalization, so the Metric contract must be handled carefully. |
| Oversample 50 candidates + exact rescore | No recall gain on fashion f32 and +0.1-0.2 ms cost. Not useful for f32. Small recall gain on f16 nytimes (+0.004 at ef=512) at +0.1 ms. |
| 8-thread QPS | Too noisy (3.2k-5.7k QPS swings with other containers). Discarded. |

## Conclusion and proposed changes
1. The adapter is not the cause. Do not spend effort on FFI batching or top-k work.
2. Adopt an F16 (or evaluate I8) traversal graph with exact f32 rescore (the f32 copy already exists in `FlatIndex`). First correctness-preserving win: 16-34% lower latency, graph memory roughly halves. Needs: exact-search recall test per dataset, filtered-search check (filtered walk uses the same graph), and persistence format check before it ships.
3. After F16 a 1.7-1.9x gap to hnswlib remains on 100-256 dim data. That is not memory bandwidth. Open hypotheses, untested: per-node layout (vector and neighbor list in separate allocations vs hnswlib's single contiguous chunk), visited-set and candidate-queue implementation, distance dispatch overhead. Next measurement: `perf stat` (cache misses, IPC) if available in the container, plus a same-session hnswlib run on the identical pass harness.
4. Only then re-run the head-to-head, restricted to datasets with an actionable improvement (all four benefit from F16; glove and nytimes first).

## Limits of this evidence
Single machine, WSL2, noisy neighbours, one Linux container run. No power-loss, recovery, or independent-review claims. S13 recovery work remains separate and still outstanding.
