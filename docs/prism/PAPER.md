# Prism: nested bit planes, an error-gated cascade and log-pinned snapshots for vector memory that is read while it is written

Project Apex research draft, 2026-10-26. Not peer reviewed. All measurements are on synthetic data on one machine, produced by `benchmarks/prism` (source checksum `50b8bcc7fdb6bf7c`, rustc 1.95.0). Derivations are in `MATH_AND_ALGORITHM.md`.

## Abstract

See `ABSTRACT.md`.

## 1. Introduction

An agent reads what it knows, acts, writes what it learned and reads again, often within milliseconds. Vector storage for this loop faces two pressures. A query over `n` vectors is mostly a question of how many bytes are pulled through memory. And the store changes while it is being queried, so any structure that must be rebuilt or retrained when data changes either goes stale or makes someone wait.

Measurements from other work show the second pressure is real. A study of vector databases under concurrent inserts reports P99 latency rising by roughly 280% to 345%, attributed to brute-force scans of unindexed segments and to lock contention (arXiv 2606.08950). A vendor benchmark reports P99 inflation between 1.1x and 10.3x across six engines under streaming load (OceanBase, 2026). A separate paper notes that data-dependent quantizers degrade or require rebuilds under insertions and deletions (arXiv 2512.18335). We have not independently reproduced any of these.

We propose Prism, built on three observations:

1. A random rotation makes any unit vector look statistically the same, so a quantizer applied after rotation needs no training and cannot drift.
2. A uniform quantizer's codes are nested: the top `b` bits of a 4-bit code are a valid `b`-bit code. Storing the code as bit planes lets one copy serve every precision.
3. Most of a corpus can be rejected at one bit. A gate based on the score error decides which candidates deserve more bits.

A fourth element, snapshots keyed to the write-ahead-log position, addresses concurrency.

## 2. Related work

Random hyperplane signs estimate angles (Goemans and Williamson, 1995; Charikar, 2002), which is the basis for 1-bit codes. Randomized Hadamard transforms give fast approximate rotations (Ailon and Chazelle, 2006). TurboQuant (ICLR 2026) rotates vectors, quantizes each coordinate, and adds a 1-bit residual transform for unbiased inner products, with distortion within about 2.7x of a lower bound as the authors report. RaBitQ (SIGMOD 2024) gives a 1-bit-per-dimension code with a theoretical error bound that supports reliable reranking, and a 2025 extension covers moderate compression rates. Multi-stage filtering with progressively more precise distance estimates is a standard idea in similarity search.

What we claim is narrower than any of these: the combination of nested bit planes, a gate over several stages, and snapshots tied to the log, evaluated for read-while-write. A fuller related-work pass could reduce the claim further. We read abstracts and summaries of the cited papers, not all of the full texts; the citations must be checked against the primary sources before submission.

References to verify: Goemans and Williamson, J. ACM 42(6), 1995. Charikar, STOC 2002. Ailon and Chazelle, STOC 2006. Gao and Long, RaBitQ, SIGMOD 2024 (arXiv 2405.12497). Zandieh et al., TurboQuant, ICLR 2026 (arXiv 2504.19874).

## 3. Design

**Rotation.** Pad to `P`, the next power of two, and apply three rounds of a seeded random sign diagonal followed by a Walsh-Hadamard transform scaled by `1/sqrt(P)`. Cost `O(P log P)` per vector; the only state is the seed.

**Planes.** Clip at `C/sqrt(P)` with `C = 3`, quantize each coordinate uniformly to 16 levels, and store the four index bits as four separate packed planes. Plane 0 is the sign bit. Full-precision vectors are kept for the final rerank and can live on a slower tier.

**Cascade.** At stage `j` with `b_j` bits, estimate every candidate's score, take the `k`-th best estimate `tau_j`, and keep candidates with `s_hat + z*sigma_j >= tau_j - z*sigma_j`. `sigma_j` is the measured standard deviation of the estimate error. After the last stage, rerank survivors exactly.

**Snapshots.** Segments are immutable. A commit seals a new segment and publishes a new snapshot (a list of segments and a log position). A reader clones the snapshot pointer and then takes no further locks. Invariants: repeatability, no torn reads, monotonic pins, and compaction that never changes the visible set.

## 4. Analysis

**Error per bit.** For fine cells, the score error has variance about `Delta_b^2 / 12`, giving `sigma_b ~ C / (sqrt(3P) * 2^b)`. Each added bit halves it. The error does not depend on the data.

**Gate safety.** If every true top-k item and one witness item have error within `z*sigma`, no top-k item is dropped (lemma, proof in `MATH_AND_ALGORITHM.md` section 4). A union bound over `k+1` items and `S` stages gives a failure probability of at most `(k+1)*S*Pr[|error| > z*sigma]`. For a sub-Gaussian error and a 1% failure budget with `k = 10`, `S = 3`, this requires `z` near 4.2.

**Cost.** Bits read are `B = n*m_1 + n_1*m_2 + n_2*m_3`, with a floor of `n` bits per query. Prism is a scan accelerator and does not remove the scan.

## 5. Implementation

A zero-dependency Rust crate, `benchmarks/prism`, about 700 lines of source: rotation, quantizer, store with snapshots, search, a deterministic generator, a benchmark binary and seven tests (norm preservation, nesting, sigma within 20% of the formula, exact result at very large `z`, compaction transparency, a 2-second one-writer four-reader invariant test, and cascade recall at `z = 3`). The scoring loop is scalar, with no lookup tables or SIMD.

## 6. Evaluation

Setup: 20,000 unit vectors, `d = 256`, 50 Gaussian clusters, 1,000 queries from the same mixture, `k = 10`, seed 7. Synthetic data. One thread unless stated. The machine was not idle and its CPU model was not recorded.

### 6.1 Quantization error and drift

| bits | measured sigma | theory | MSE original data | MSE shifted data |
|---|---|---|---|---|
| 1 | 0.0610 | 0.0541 | 0.856 | 0.835 |
| 2 | 0.0272 | 0.0271 | 0.189 | 0.188 |
| 4 | 0.0070 | 0.0068 | 0.0123 | 0.0120 |

The formula holds within 13% at 1 bit and within 3% at 4 bits. Reconstruction error is within about 3% on shifted data at every width: the quantizer does not depend on the distribution.

### 6.2 Search quality and bits read

Bits are counted per coordinate per vector (a flat 4-bit scan reads 4; float32 is 32). "New planes" counts only planes read for the first time; "prefix" counts a full re-read at each stage.

| method | recall@10 | queries missing a neighbour | bits, new planes | bits, prefix | survivors per stage | exact reranks (mean) |
|---|---|---|---|---|---|---|
| flat 1-bit, rerank 100 | 0.770 | | 1 | 1 | | 100 |
| flat 2-bit, rerank 100 | 0.780 | | 2 | 2 | | 100 |
| flat 4-bit, rerank 100 | 0.998 | | 4 | 4 | | 100 |
| cascade z = 1 | 0.856 | 78.6% | 1.031 | 1.061 | 328 / 141 / 35 | 35 |
| cascade z = 1.5 | 0.977 | 21.0% | 1.047 | 1.093 | 394 / 269 / 66 | 66 |
| cascade z = 2 | 0.998 | 1.7% | 1.056 | 1.111 | 399 / 356 / 107 | 107 |
| cascade z = 2.5 | 0.9999 | 0.1% | 1.059 | 1.118 | 399 / 390 / 154 | 154 |
| cascade z = 3 | 1.000 | 0% | 1.060 | 1.120 | 399 / 398 / 204 | 204 |
| cascade z = 4 | 1.000 | 0% | 1.060 | 1.120 | 399 / 399 / 296 | 296 |

At `z = 3` the cascade returned the exact top 10 for all 1,000 queries while reading 1.06 bits, 3.8x fewer than a flat 4-bit scan and about 30x fewer than float32. Most of the cost is the first stage, which reads one bit of every vector, so the result sits near the one-bit floor.

### 6.3 Why stage 1 is so effective here, and a warning

Stage 1 keeps about 399 vectors for every `z` from 2 to 4. The generator makes 50 clusters of about 400 vectors, and the query comes from one of them. The 1-bit stage isolates that cluster, and the later stages only choose among its members. This is the easy case.

Our first prototype (numpy) drew the queries from a separate set of cluster centres and read about 1.77 bits per coordinate at `z = 2`. We guessed that the query distribution explained the difference. Section 6.6 tests that guess: out-of-distribution queries on the Rust code read 1.40 bits, not 1.77, so the guess was incomplete and the remaining gap is unexplained. The practical point is that the saving depends on how much structure the data and queries have, and real embeddings may land anywhere in this range.

### 6.4 Time

| method | p50 ms | p95 ms |
|---|---|---|
| exact float scan | 3.65 | 4.51 |
| flat 4-bit + rerank 100 | 7.09 | 9.42 |
| cascade z = 2 | 4.37 | 5.54 |
| cascade z = 3 | 4.68 | 6.21 |

**In this first run the cascade was slower than an exact float scan.** Section 6.6 repeats the comparison with a lookup-table kernel and an optimised baseline. The scoring loop decodes planes coordinate by coordinate in scalar code, so it does extra work per bit read, and a 20,000-vector float scan fits in cache. Whether a lookup-table or SIMD kernel changes this is untested, and we make no speed claim.

### 6.5 Concurrent writes

One writer committing 20-vector batches every 10 ms (1,886 vectors/s, 94 commits/s, compacting every 32 commits) against four reader threads for five seconds.

| | p50 ms | p95 ms | p99 ms |
|---|---|---|---|
| writer off | 5.30 | 9.68 | 19.51 |
| writer on | 5.27 | 7.79 | 10.91 |

The P99 ratio, concurrent over serial, is 0.56. A ratio below 1 means run-to-run noise on a busy machine exceeds any effect of the writer; it is not evidence that writing helps. The experiment shows the snapshot mechanism holds its invariants under load (the tests assert this) but does not yet measure the jitter ratio with useful precision.

### 6.6 The weak points, re-measured

After section 6.1 to 6.5 we added a lookup-table scoring kernel with incremental planes, harder data sets, error-tail measurements, per-vector gates and repeated concurrency trials. All numbers are in `results.json` under `extra` (source checksum `103fcb801fb94647`). Same caveats: synthetic data, one non-idle machine.

**Kernel.** Scoring uses a per-query byte lookup table, so each plane costs `P/8` table reads per vector, and each survivor keeps its partial sums so a later stage reads only new planes.

| path (n = 20,000, single thread) | p50 ms | p95 ms |
|---|---|---|
| exact scan, plain | 4.28 | 4.96 |
| exact scan, optimised | 0.55 | 0.78 |
| flat 4-bit lookup + rerank 100 | 1.31 | 1.57 |
| cascade lookup, z = 2 | 0.61 | 0.79 |
| cascade lookup, z = 3 | 0.66 | 0.86 |
| cascade scalar, z = 2 | 4.54 | 5.63 |

The kernel makes the cascade about 7 times faster than the scalar version. It does not beat an optimised exact float scan, which at this size fits in cache. Our earlier comparison in 6.4 used the plain exact scan and was not a fair baseline.

**Harder data** (cascade, z = 2; bits per coordinate per vector; time over the first 200 queries):

| data | n | recall@10 | bits z = 2 | bits z = 4 | p50 ms |
|---|---|---|---|---|---|
| clustered, in-distribution | 20,000 | 0.998 | 1.06 | 1.06 | 0.47 |
| clustered, out-of-distribution queries | 20,000 | 0.998 | 1.40 | 3.06 | 0.88 |
| isotropic Gaussian | 20,000 | 0.999 | 1.30 | 2.90 | 0.90 |
| overlapping clusters | 20,000 | 1.000 | 1.04 | 1.36 | 0.49 |
| clustered | 100,000 | 0.997 | 1.05 | 1.06 | 3.74 |

The cascade keeps its saving on data without cluster structure, but a conservative gate width costs most of it there: at `z = 4` the harder sets read about 3 bits, close to a flat 4-bit scan.

**Gates.** Error shape is close to Gaussian (excess kurtosis between -0.02 and 0.23) but the tails are heavier: over 20 million pairs per set the largest error was 5.2 to 7.1 standard deviations, so a gate with zero misses over every pair would need `z` near 7.1. A per-vector bound `|s_hat - s| <= r_x * sqrt(2 ln(2/delta) / d)` with `delta = 0.001`, where `r_x` is the stored residual norm of the vector, was violated on between 0.008% and 0.055% of pairs, below `delta`, so it held empirically. It read about as many bits as global `z = 4` (2.97 against 3.06 bits on the out-of-distribution set) and more than `z = 3` (2.22). The deterministic Cauchy-Schwarz bound `|s_hat - s| <= r_x` never missed and kept every vector on three of the four sets, reading 4.0 bits. A provable guarantee therefore costs most of the saving on hard data; this is the main unresolved result.

**Concurrency.** Seven alternating writer-off and writer-on trials of 3 seconds with four lookup-table readers and a writer committing about 1,900 vectors per second. The median ratio of P99 latency, concurrent over serial, was 1.21 (range 1.12 to 1.34), and every trial was above 1. Median reader throughput fell from 6,007 to 5,255 queries per second (-12%). The earlier ratio of 0.56 was noise. Readers take no lock after pinning, so the cost is shared CPU and memory bandwidth, which we have not profiled. Snapshots remove blocking, not contention.

## 7. Limitations

All data is synthetic. The gate width is calibrated, and the provable alternatives cost most of the saving on hard data (6.6). The scan floor is one bit per vector. The lookup-table cascade is not faster than an optimised exact scan at 20,000 vectors, and larger n and SIMD are untested. A concurrent writer raises median P99 by about 21% and cuts reader throughput by about 12%. There are no deletes, no filtered search and no integration with the Atlas `AnnIndex` trait. There is no comparison with RaBitQ, product quantization, HNSW or any database. Timing is from one non-idle machine whose CPU model was not recorded.

## 8. Future work

1. A SIMD kernel, and timing at larger n where memory bandwidth dominates.
2. A sharper per-vector bound in the style of RaBitQ, to close the gap between the empirical `z` near 2 and a provable width.
3. Real embedding sets (for example SIFT1M, GIST1M and a recent text-embedding set).
4. A quiet-machine concurrent benchmark with a multi-core writer and profiling of the cost of contention.
5. A partitioning index in front of the cascade, to go below the one-bit floor, and filtered search.
6. Baselines on identical hardware: flat float, product quantization, RaBitQ, TurboQuant-style, HNSW.
7. Deletes in the snapshot model, and a `PrismIndex` implementing `AnnIndex` fed from the WAL.

## 9. Reproducibility

```
cd benchmarks/prism
cargo test --release
cargo run --release --bin prism-bench
```

Output: `benchmarks/prism/results/results.json` (checksum `103fcb801fb94647`), which records the rustc version, the machine note, the parameters, `synthetic: true` and an FNV-1a checksum of `src/*.rs` (it does not cover `src/bin`). The same file is served to the site as `site/data/prism-results.json`.
