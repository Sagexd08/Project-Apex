# Prism: related work and claim audit (Stage B, 2026-10-27)

Status: Research Note v1.0, not peer reviewed. This is a **partial** primary-source audit. The "Depth" column says exactly what was read; nothing marked "abstract" was checked beyond the abstract page, and nothing here was reproduced.

## 1. Sources actually read

| Source | Depth | What was taken from it |
|---|---|---|
| RaBitQ, Gao and Long, SIGMOD 2024, arXiv 2405.12497 | Full text: intro, Theorem 3.2, rerank use, related-work | Estimator and bound (section 2) |
| Quantization for Vector Search under Streaming Updates, Aden-Ali, Ferhatosmanoglu, Greaves-Tunnell, Mishra, Wagner, arXiv 2512.18335 (Dec 2025) | Full text: Definition 2.1, Theorem 3.1, CoDEQ overview, baselines | Definition of dynamic consistency, cost bound, baselines |
| TurboQuant, Zandieh, Daliri, Hadian, Mirrokni, ICLR 2026 | Abstract page only | Rotation, scalar quantizer, 1-bit residual stage, near-optimal distortion |
| FreshDiskANN, Singh et al., arXiv 2105.09613 | Abstract only | Real-time updates, billion-point claim |
| ACORN, Patel et al., arXiv 2403.04871 | Abstract only | Predicate-agnostic filtered HNSW |
| Filtered-ANN benchmark, Iff et al., arXiv 2507.21989 (v3 Apr 2026) | Abstract only | Eleven methods, arxiv-for-fanns dataset |
| When More Cores Hurts, Ockerman et al., arXiv 2606.08950 | Abstract only | Throughput and scaling result |
| RocksDB Snapshot wiki; SQLite WAL doc; Linux RCU doc | Official pages, summarised | Snapshot semantics |

**Not read:** ScaNN / anisotropic quantization, FAISS and OPQ papers, SeRF, Filtered-DiskANN, UNG, VIBE, ANN-Benchmarks, VectorDBBench, the OceanBase streaming study, SIFT1M/GIST1M dataset terms. Entries for these in the matrix come from general background and are marked **Unread**.

## 2. Primary-source facts that matter

**RaBitQ (read).** Each unit data vector `o` becomes a D-bit code by a random rotation of a binary codebook. For a query `q` the estimator `<o_bar,q> / <o_bar,o>` is **unbiased** for `<o,q>` (Theorem 3.2, eq. 13), with a probabilistic bound (eq. 14):

```
P[ | <o_bar,q>/<o_bar,o>  -  <o,q> |  >  sqrt( (1 - <o_bar,o>^2) / <o_bar,o>^2 ) * eps0 / sqrt(D-1) ]  <=  2 exp(-c0 eps0^2)
```

where `eps0` sets the failure probability and `c0` is a constant. The authors state the bound as `O(1/sqrt(D))` with high probability, and use it **to decide which candidates to re-rank by exact distance**. They report that PQ lacks such a bound and can fall below 60% recall even with re-ranking on some data.

**Streaming quantization (read).** Definition 2.1: a dynamic index is *dynamically consistent* if its answers after `Build(X); Insert(y)` equal those of `Build(X ∪ {y})` (and likewise for delete), for each fixed random seed. Theorem 3.1 gives a dynamically consistent structure for the streaming `(eps, delta)`-ANN problem with memory `O(n log N / eps^2 (log(1/(eps delta)) + log log N))` bits and **one read and one write I/O per update**, each of size `O(eps^-2 log N)` words, using full-precision points on disk. The practical method CoDEQ is **data-dependent** (median-based kd-tree partitions that stay stable under change), preceded by a standard data-oblivious random rotation. Baselines: FrozenPQ, RaBitQ, OnlinePQ.

**TurboQuant (abstract).** Random rotation makes coordinates near-independent, so an optimal scalar quantizer is applied per coordinate; a 1-bit Quantized JL step on the residual removes inner-product bias; distortion is within about 2.7 times the information-theoretic lower bound; data-oblivious.

**HPC paper (abstract).** Its stated result is about **throughput**: more cores can reduce query throughput by up to 30.67 percent, and 16 to 256 workers gives only 5.46 times. The abstract makes **no P99 claim**. Any P99-inflation number must come from another source.

**Snapshots (official docs).** RocksDB ties a snapshot to a sequence number and makes compaction preserve what live snapshots see. SQLite readers fix a WAL end mark; a checkpoint must stop at it, long readers can stall checkpoints and let the WAL grow without bound. RCU guarantees data seen in a read-side section stays unreclaimed until a grace period ends.

## 3. Comparison matrix

| Family | Representative | Existing guarantee | Update behaviour | Filtering | Cost | What Prism might add |
|---|---|---|---|---|---|---|
| Rotation + scalar quantization, data-oblivious | TurboQuant (abstract) | Distortion within about 2.7x of the lower bound; unbiased inner product with residual stage | Nothing to train | Not addressed | One scalar quantizer per coordinate plus 1-bit residual | Nothing new in the encoder. Prism's version has **no residual stage** and is biased toward zero |
| Binary code with probabilistic bound | RaBitQ (full text) | Unbiased estimator, `O(1/sqrt D)` bound with failure probability, used to decide re-ranking | Codebook is a random rotation, so no training, but updates are not the paper's subject | Not addressed | D bits per vector, bitwise or SIMD estimation | The closest prior art for Prism's gate. Prism's per-vector gate is a heuristic cousin of this bound |
| Trained product quantization | PQ, OPQ (Unread) | None; per RaBitQ's paper, no error bound | Codebooks drift; FrozenPQ/OnlinePQ are baselines in 2512.18335 | Via the index | Training time, codebooks | A baseline, not a competitor on guarantees |
| Score-aware quantization | ScaNN (Unread) | Not reviewed | Trained | Via the index | Not reviewed | Unknown |
| Dynamically consistent quantization | CoDEQ and Theorem 3.1, 2512.18335 (full text) | Answers equal a from-scratch build after each update; bounded I/O per update; `(eps, delta)`-ANN guarantee in theory | The paper's subject; data-dependent partitions with limited reassignment | Not addressed | 1 read + 1 write I/O per update (theory); stores full-precision data on disk | **The closest prior art for any streaming claim.** Prism must say why a data-oblivious code is preferable, and must not claim to be the first updateable quantizer |
| Streaming graph index | FreshDiskANN (abstract) | Reports 5-recall@5 above 95 percent under thousands of concurrent updates per second; billion points on one SSD workstation | Real-time insert and delete without full rebuild | Not in abstract | Disk-backed graph, 5 to 10x cheaper to keep fresh than periodic rebuild, as reported | Prism has no graph; it is a scan accelerator |
| Filtered graph search | ACORN; arxiv-for-fanns (abstracts) | Reports 2 to 1000x throughput at fixed recall over baselines; no formal guarantee in the abstract | Graph-dependent | Core topic; predicate-agnostic | HNSW extended for predicate subgraphs | Prism's cascade has **no filter story**; use pre-filter plus exact scan until one exists |
| Snapshots and logs | RocksDB, SQLite WAL, RCU (docs) | Consistent point-in-time view; reclamation waits for readers | Writers do not block readers | n/a | Retained versions, WAL growth, stalled checkpoints | Borrowed, not new. Prism must state its own reclamation and backpressure rules |

## 4. Claim audit

| Prism claim | Verdict |
|---|---|
| "No training means no drifting codebook" | Shared with TurboQuant and RaBitQ. **Narrow:** true for the code, but data-dependent partitions or scales (our clip constant, calibrated sigma) can still go stale |
| Nested 1, 2, 3, 4 bit prefixes of one code | Not found in the sources read. **Unchecked** against the full matryoshka and multi-stage quantization literature |
| Error-gated cascade with exact rerank | Re-ranking guided by an estimator bound is **RaBitQ's method**. Prism's cascade adds staged bit depth. Overlap is real |
| Per-vector bound from stored residual norm | Weaker than RaBitQ's probabilistic bound (Prism's deterministic Cauchy-Schwarz bound is sound but prunes almost nothing at d = 256) |
| Log-pinned snapshot per reader | Established practice (RocksDB, SQLite). The new part, if any, is the interaction with progressive codes and has no evidence yet |
| "No retraining under streaming" | 2512.18335 shows data-oblivious methods as baselines and builds a data-dependent method that beats them in its experiments. Prism cannot claim an advantage without comparing against it |

## 5. Three lists

**Already solved by prior work**
- Random rotation plus scalar quantization, data-oblivious (TurboQuant, RaBitQ).
- An unbiased estimator with a probabilistic error bound that guides re-ranking (RaBitQ).
- Dynamically consistent updates with bounded I/O, in theory and practice (2512.18335).
- Real-time graph-index updates (FreshDiskANN).
- Sequence-number snapshots and reclamation rules (RocksDB, SQLite, RCU).

**Not solved by the designs reviewed (with the depth caveat above)**
- Progressive multi-bit reads from **one** stored code with a bound that is tight enough to prune at d = 256. Prism's measured provable bound prunes almost nothing; RaBitQ's form may fix this (Stage C, section C.3).
- A measured read-while-write comparison of a data-oblivious code against a dynamically consistent data-dependent one, with tail latency over hours.

**Potential Prism contribution, pending proof and measurement**
1. Nested planes as a **storage layout** that lets one copy serve several precisions. Unproven benefit; the only time measurement shows no gain over an optimised scan.
2. A per-vector progressive bound in the RaBitQ style applied stage by stage. Not yet derived or tested.
3. Snapshot semantics for a progressively read code. Not yet modelled.

If these three do not survive Stages E and F, the honest conclusion is that Prism is a re-implementation of known ideas with a measurement, which is still a publishable negative result.

## 6. Questions still open

- Full read of TurboQuant, FreshDiskANN, ACORN, 2507.21989 and 2606.08950 (method and evaluation, not abstract).
- Does 2512.18335's evaluation use datasets and hardware we can reproduce?
- RaBitQ's constant `c0` and its practical `eps0`; how the bound transfers from 1-bit D-dimensional codes to Prism's 4-plane code.
- Terms and checksums of SIFT1M and GIST1M; licence of the arxiv-for-fanns embeddings and the model that produced them.
