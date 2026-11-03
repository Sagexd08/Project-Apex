# Prism: related work and claim audit (Stage B, 2026-10-27)

Status: Research Note v1.0, not peer reviewed. This is a **partial** primary-source audit. The "Depth" column says exactly what was read; nothing marked "abstract" was checked beyond the abstract page, and nothing here was reproduced.

## 1. Sources actually read

| Source | Depth | What was taken from it |
|---|---|---|
| RaBitQ, Gao and Long, SIGMOD 2024, arXiv 2405.12497 | Full text: intro, Theorem 3.2, rerank use, related-work | Estimator and bound (section 2) |
| Quantization for Vector Search under Streaming Updates, Aden-Ali, Ferhatosmanoglu, Greaves-Tunnell, Mishra, Wagner, arXiv 2512.18335 (Dec 2025) | Full text: Definition 2.1, Theorem 3.1, CoDEQ overview, baselines | Definition of dynamic consistency, cost bound, baselines |
| TurboQuant, Zandieh, Daliri, Hadian, Mirrokni, ICLR 2026 | **P2 update:** arXiv 2504.19874 abstract plus about the first 40 percent of the HTML (intro, sections 2 to 3.2). Theorem statements from the intro. Rest, and the ICLR PDF, not read | Rotation, scalar quantizer, 1-bit residual stage, near-optimal distortion (see section 7) |
| FreshDiskANN, Singh et al., arXiv 2105.09613 | **P2 update:** abstract, intro, sections 1 to 3 in full; sections 4 to 6 summarised only; text ends mid-evaluation | Real-time updates, billion-point claim, update algorithms (section 7) |
| ACORN, Patel et al., arXiv 2403.04871 | **P2 update:** abstract, intro and method in full (about 39k characters); remainder summarised | Predicate-agnostic filtered HNSW (section 7) |
| Filtered-ANN benchmark, Iff et al., arXiv 2507.21989 (v3 Apr 2026) | **P2 update:** first about 39k characters in full; last about 11k characters not read | Eleven methods, arxiv-for-fanns dataset (section 7) |
| When More Cores Hurts, Ockerman et al., arXiv 2606.08950 | **P2 update:** first about 39k characters in full; rest summarised. P99 figures below come from a summary and are **not yet checked against the PDF** | Throughput, scaling **and tail-latency** results (section 2 corrected) |
| RocksDB Snapshot wiki; SQLite WAL doc; Linux RCU doc | Official pages, summarised | Snapshot semantics |

**Not read (original Stage B list):** ScaNN / anisotropic quantization, FAISS and OPQ papers, SeRF, Filtered-DiskANN, UNG, VIBE, ANN-Benchmarks, VectorDBBench, the OceanBase streaming study, SIFT1M/GIST1M dataset terms. Entries for these in the matrix come from general background and are marked **Unread**. **P2 changed this for some of them; the authoritative status of every source is the register in section 7.**

## 2. Primary-source facts that matter

**RaBitQ (read).** Each unit data vector `o` becomes a D-bit code by a random rotation of a binary codebook. For a query `q` the estimator `<o_bar,q> / <o_bar,o>` is **unbiased** for `<o,q>` (Theorem 3.2, eq. 13), with a probabilistic bound (eq. 14):

```
P[ | <o_bar,q>/<o_bar,o>  -  <o,q> |  >  sqrt( (1 - <o_bar,o>^2) / <o_bar,o>^2 ) * eps0 / sqrt(D-1) ]  <=  2 exp(-c0 eps0^2)
```

where `eps0` sets the failure probability and `c0` is a constant. The authors state the bound as `O(1/sqrt(D))` with high probability, and use it **to decide which candidates to re-rank by exact distance**. They report that PQ lacks such a bound and can fall below 60% recall even with re-ranking on some data.

**Streaming quantization (read).** Definition 2.1: a dynamic index is *dynamically consistent* if its answers after `Build(X); Insert(y)` equal those of `Build(X ∪ {y})` (and likewise for delete), for each fixed random seed. Theorem 3.1 gives a dynamically consistent structure for the streaming `(eps, delta)`-ANN problem with memory `O(n log N / eps^2 (log(1/(eps delta)) + log log N))` bits and **one read and one write I/O per update**, each of size `O(eps^-2 log N)` words, using full-precision points on disk. The practical method CoDEQ is **data-dependent** (median-based kd-tree partitions that stay stable under change), preceded by a standard data-oblivious random rotation. Baselines: FrozenPQ, RaBitQ, OnlinePQ.

**TurboQuant (abstract).** Random rotation makes coordinates near-independent, so an optimal scalar quantizer is applied per coordinate; a 1-bit Quantized JL step on the residual removes inner-product bias; distortion is within about 2.7 times the information-theoretic lower bound; data-oblivious.

**HPC paper (abstract, corrected in P2).** The abstract's stated result is about **throughput**: more cores can reduce query throughput by up to 30.67 percent, and 16 to 256 workers gives only 5.46 times. The abstract makes **no P99 claim**. *Correction:* the paper's body does report tail latency. Per the P2 fetch summary (not verified against the PDF), P99 rose by an average of 280.10 percent on Pes2o-VE and 344.77 percent on Yandex-T2I, with mean latency up 34.99 (Milvus), 102.74 (Qdrant) and 66.52 percent (Weaviate), on HPC systems (Aurora, Polaris, up to 256 workers on 64 nodes). These numbers are about scaling workers on a cluster, **not** about concurrent writes. They must not be attached to the OceanBase streaming study or to Prism's read-while-write jitter.

**Snapshots (official docs).** RocksDB ties a snapshot to a sequence number and makes compaction preserve what live snapshots see. SQLite readers fix a WAL end mark; a checkpoint must stop at it, long readers can stall checkpoints and let the WAL grow without bound. RCU guarantees data seen in a read-side section stays unreclaimed until a grace period ends.

## 3. Comparison matrix

| Family | Representative | Existing guarantee | Update behaviour | Filtering | Cost | What Prism might add |
|---|---|---|---|---|---|---|
| Rotation + scalar quantization, data-oblivious | TurboQuant (abstract) | Distortion within about 2.7x of the lower bound; unbiased inner product with residual stage | Nothing to train | Not addressed | One scalar quantizer per coordinate plus 1-bit residual | Nothing new in the encoder. Prism's version has **no residual stage** and is biased toward zero |
| Binary code with probabilistic bound | RaBitQ (full text) | Unbiased estimator, `O(1/sqrt D)` bound with failure probability, used to decide re-ranking | Codebook is a random rotation, so no training, but updates are not the paper's subject | Not addressed | D bits per vector, bitwise or SIMD estimation | The closest prior art for Prism's gate. Prism's per-vector gate is a heuristic cousin of this bound |
| Trained product quantization | PQ, OPQ (Unread) | None; per RaBitQ's paper, no error bound | Codebooks drift; FrozenPQ/OnlinePQ are baselines in 2512.18335 | Via the index | Training time, codebooks | A baseline, not a competitor on guarantees |
| Score-aware quantization | Anisotropic VQ / ScaNN, arXiv 1908.10396 (P2: sections 1 to 4 read) | Loss decomposition, closed-form codeword update, loss non-increasing (Thms 3.2 to 4.2); assumes queries uniform on the sphere; no error bound on a returned score | Trained codebooks; updates not covered in what was read | Not covered | Learned codebooks, single-thread evaluation | A trained alternative aimed at MIPS. Not a competitor on guarantees or on streaming |
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

*(Stage B list. Superseded by section 10 after P2.)*

---

# P2 update (2026-10-30)

Scope: documents and web only. No build, test, benchmark, Docker or browser work; S14 untouched. Baseline: HEAD `785478751d345261b11f99e926b14b496dd488b5` on `main`, 58 uncommitted paths, source digest `9b97729a3ea10a342c6192cf1db1fd86` (files=60). That digest equals the latest one recorded in `docs/GATES.md` (line 91); `docs/GATES.md` was not modified. Evidence labels: **[R]** read in the text, **[A]** author-reported result, **[S]** search-result summary only (the page was not opened), **[I]** my inference, **[U]** unread. Fetches were done by a read-only fetch agent; "read" means the agent read that text, and quoted numbers are its transcription. Spot-check any number before it goes in the paper.

## 7. Source register

| # | Source (title, authors, date, URL) | Status | What it proposes and shows | Guarantees and assumptions | Meaning for Prism |
|---|---|---|---|---|---|
| 1 | TurboQuant: Online Vector Quantization with Near-optimal Distortion Rate. Zandieh, Daliri, Hadian, Mirrokni. arXiv v1 28 Apr 2025; ICLR 2026. https://arxiv.org/abs/2504.19874 ; ICLR abstract page https://proceedings.iclr.cc/paper_files/paper/2026/hash/5c802ef38ab6e366c2ea06eee554c088-Abstract-Conference.html | **Partial full text.** Abstract and about 40% of the HTML read; ICLR PDF not read | Random rotation, then per-coordinate Lloyd-Max scalar codebooks for Beta-distributed coordinates. Inner-product variant: b-1 bits of MSE quantization plus a 1-bit QJL step on the residual. **[A]** Single A100; 4-bit quantization time at d=1536: 0.0013 s vs PQ 239.75 s vs RaBitQ 2267.59 s; LongBench-E 50.06 full vs 49.44 at 2.5 bits | Thm 1: D_mse <= (sqrt3*pi/2)*4^-b. Thm 2: unbiased inner product, D_prod <= (sqrt3*pi^2*||y||^2/d)*4^-b. Thm 3: lower bounds. Assume unit-norm x. Bounds are on **expected** distortion, not worst case. Admits the MSE-only quantizer is biased (about 2/pi at b=1) | Rotation + scalar code + data-oblivious is **TurboQuant's design**. Our biased, residual-free code is the version TurboQuant says needs the QJL step for unbiased inner products. A bound on expected distortion cannot justify a deterministic prune [I] |
| 2 | Accelerating Large-Scale Inference with Anisotropic Vector Quantization. Guo, Sun, Lindgren, Geng, Simcha, Chern, Kumar. arXiv 1908.10396 (v1 Aug 2019, v5 Dec 2020). https://arxiv.org/abs/1908.10396 | **Partial full text.** Sections 1 to 4 read; experiments and appendix summarised. Venue not verified | Score-aware loss that weights the parallel residual more than the orthogonal one; closed-form codeword update; extended to PQ. **[A]** Glove1.2M, Amazon-670k, SIFT1M; single thread; Amazon-670k Recall1@1 at 2048 bits 0.782 (PQ) to 0.875 | Thm 3.2 assumes queries uniform on the sphere. Loss non-increasing so iteration converges | A trained MIPS baseline. Streaming and filtering not covered in what was read |
| 3 | FreshDiskANN. Singh, Subramanya, Krishnaswamy, Simhadri. arXiv v1 20 May 2021. https://arxiv.org/abs/2105.09613 | **Partial full text.** Sections 1 to 3 read; 4 to 6 summarised; text ends mid-evaluation | Vamana-style graph with alpha-pruning; lazy delete with batch consolidation; SSD-resident index plus in-memory TempIndexes merged by StreamingMerge. **[A]** SIFT1M/GIST1M/Deep1M/SIFT100M on Azure E64d_v4; 1800 inserts and 1800 deletes/s steady, 1000 searches/s at 95+% 5-recall@5, under 128 GB DRAM | **No formal theorems** in the visible text; consistency is *quiescent consistency*; alpha>1 assumed | Real-time updates are prior art. Admitted limits: recall settles lower after merges (PQ distances), about 20 cycles to stabilise, latency spikes in delete/patch phases, inserts throttled |
| 4 | ACORN. Patel, Kraft, Guestrin, Zaharia. arXiv v1 7 Mar 2024; SIGMOD 2024. https://arxiv.org/abs/2403.04871 | **Partial full text.** First about 39k characters read; rest summarised | Predicate-subgraph traversal over HNSW (ACORN-gamma, ACORN-1). **[A]** "2 to 1000x higher throughput at fixed recall"; m5d.24xlarge; SIFT1M, Paper, TripClick, LAION 1M/25M | No connectivity guarantee for arbitrary data; bounds assume no predicate clustering and decay in gamma. Admits time-to-index up to 11x HNSW, index up to 1.3x, per-dataset tuning, and that FilteredDiskANN/NHQ fail on high-cardinality sets | The strongest predicate-agnostic filtered baseline. Updates not discussed in what was read |
| 5 | Benchmarking Filtered ANN Search Algorithms on Transformer-based Embedding Vectors. Iff, Brugger, Chrapek, Kochergin, Besta, Hoefler. arXiv 2507.21989 v3, 1 Apr 2026; SIGIR 2026. https://arxiv.org/abs/2507.21989 ; code github.com/spcl/fanns-benchmark | **Partial full text.** First about 39k characters read; last about 11k not read | Eleven methods: ACORN, CAPS, FDANN (filtered and stitched), NHQ (kgraph, nsw), SeRF, iRangeGraph, DIGRA, DSG, UNG. Dataset arxiv-for-fanns: 2.7M+ arXiv abstracts, 11 attributes, embeddings by stella_en_400M_v5 ("v5", no finer version seen), 4096 dim, normalised; small 1k, medium 100k, large 2.7M. **[A]** specialised methods up to 10x faster than general ones; recall can fall below 0.2 at large scale | Euclidean only, one embedding model; DSG/DIGRA excluded at large scale. **Licence and checksums not seen**; HF datasets SPCL/arxiv-for-fanns-small/-medium/-large | The reference for filtered evaluation. Dataset use needs licence, model-version and checksum checks first |
| 6 | When More Cores Hurts: The Vector Database Scaling Paradox in HPC. Ockerman et al. arXiv v1 8 Jun 2026 (cs.DC). https://arxiv.org/abs/2606.08950 | **Partial full text** (first about 39k characters). P99 figures from a summary, **unverified** | Qdrant, Milvus, Weaviate on Aurora and Polaris. **[A]** throughput down up to 30.67%; 16 to 256 workers gives 5.46x; P99 up 280.10% (Pes2o-VE) and 344.77% (Yandex-T2I) | Concerns worker scaling on clusters, not concurrent writes | Context only. Do not cite for streaming jitter |
| 7 | VIBE: Vector Index Benchmark for Embeddings. Jaasaari, Hyvonen, Ceccarello, Roos, Aumuller. arXiv 2505.17810 (23 May 2025); site says JMLR/DMLR 2026. https://arxiv.org/abs/2505.17810 ; https://vector-index-bench.github.io/ | **[S]** and site front page. Paper body not read; OOD section not read | Benchmark of 21 to 22 ANN implementations on 12 in-distribution and 6 to 8 OOD datasets; per-query difficulty and OOD breakdowns. Counts differ between versions | Not reviewed | Candidate harness and a source of OOD datasets. Our Stage D OOD-query claim should be checked against it |
| 8 | SeRF: Segment Graph for Range-Filtering ANN Search. Zuo, Qiao, Zhou, Li, Deng. PACMMOD 2(1), SIGMOD 2024. doi 10.1145/3639324 ; code github.com/xiaozxiong/SeRF | **[S]** search summary only; no arXiv version found | Segment graph for half-bounded ranges at the cost of one index; 2D segment graph O(n log n) average-case size | Not reviewed | Range-filter baseline. A cascade has no range-filter story |
| 9 | Navigating Labels and Vectors: A Unified Approach to Filtered ANN Search (UNG). Cai, Shi, Chen, Zheng. SIGMOD 2024 per one listing (my earlier "SIGMOD 2025" was unsupported) | **[S]** search summary only; no arXiv id found | Label navigating graph over label-set containment; each query touches two NSW graphs | Not reviewed | Label-filter baseline for containment predicates |
| 10 | Filtered-DiskANN. Gollapudi et al. WWW 2023, pp. 3406-3416. doi 10.1145/3543507.3583552 ; author PDF harsha-simhadri.org/pubs/Filtered-DiskANN23.pdf | **[S]** abstract-level; ACM returned 403 | Two algorithms, one streaming-capable, one batch; **[A]** order of magnitude better for filtered queries | A 2025 paper says OR filters only, no AND (**[S]**, second-hand) | Label-filter baseline with streaming support |
| 11 | Optimized Product Quantization for ANN Search. Ge, He, Ke, Sun. CVPR 2013 pp. 2946-2953. https://openaccess.thecvf.com/content_cvpr_2013/html/Ge_Optimized_Product_Quantization_2013_CVPR_paper.html | **[S]** abstract paraphrase only | Joint optimisation of space decomposition and codebooks; parametric solution provably optimal for Gaussian data | Gaussian assumption for the provable variant | Trained-codebook baseline. **Correction:** the arXiv id 1311.4055 given earlier was wrong (it is an unrelated graph paper) |
| 12 | Product Quantization for NN Search. Jegou, Douze, Schmid. TPAMI 2011 | **[U]** HAL page blocked (Anubis) | n/a | n/a | Baseline; claims taken only from RaBitQ's description |
| 13 | Why Streaming Workloads Break Most Vector Databases (OceanBase). https://en.oceanbase.com/blog/streaming-workload-benchmark-5-vector-databases-compared | **[S]** the page returned HTTP 429 to the fetcher; numbers come from a search summary, and the summary says the posts were updated and are not fully consistent | **Vendor-authored.** Cohere-10M (768 d, cosine) on five engines (seekdb, Elasticsearch, Milvus, Qdrant, Chroma) with Zilliz's VectorDBBench StreamingPerformanceCase. **[A, via summary]** concurrent P99 jitter 1.1x (seekdb 1.3.0) to 10.3x (Elasticsearch 9.3.3); Milvus 9.7x | The benchmark tool is maintained by Zilliz (Milvus vendor); seekdb is OceanBase's own engine. Raw output: github.com/oceanbase/vdb-streambench (**not fetched**) | The source of the J99 definition. Not a neutral ranking. Reproduce or leave as "reported, not reproduced" |
| 14 | RaBitQ (Gao, Long, SIGMOD 2024, arXiv 2405.12497); Streaming-update quantization (arXiv 2512.18335); RocksDB, SQLite WAL and RCU docs | Unchanged from section 1 | see sections 2 and 3 | | |
| 15 | ANN-Benchmarks (Aumuller, Bernhardsson, Faithfull); VectorDBBench; SIFT1M/GIST1M terms; DEEP1B; SPANN; Faiss | **[U]** | n/a | n/a | Still unread; entries must stay **Unread** |

## 8. Claim-by-claim novelty assessment

Classes: **Prior art** (published, same idea), **Engineering combination** (known parts joined; value is a measurement), **Candidate** (not found in the sources read, unproven), **Unsupported**. All verdicts hold only for the sources read; the unread list in the register limits them.

| # | Prism claim | Class | Basis |
|---|---|---|---|
| C1 | Data-oblivious rotation + per-coordinate scalar code needs no training | **Prior art** | TurboQuant (read in part), RaBitQ (read) |
| C2 | Near-optimal distortion from rotation + scalar | **Prior art, and not ours to claim** | TurboQuant Thms 1 to 3 (expected distortion) |
| C3 | Unbiased inner-product estimates | **Not met by Prism's code.** Prism's code lacks a residual stage and is biased; TurboQuant's Thm 2 needs the QJL residual step | TurboQuant, Prism pilot |
| C4 | Estimator-bound-guided re-ranking with exact distances | **Prior art** (RaBitQ) | RaBitQ section on re-ranking |
| C5 | Stage-wise cascade over 1/2/3/4-bit prefixes of a single code | **Candidate, weakly searched.** Not seen in the sources read, but the multi-stage, residual and matryoshka quantization literature was not searched. **Unchecked** | Register entries 1 to 3 |
| C6 | Provable per-vector bound that prunes | **Engineering, and weaker than the closest work.** Prism's own measurement says the provable bound prunes almost nothing at d=256; RaBitQ's bound is probabilistic and unbiased-centred | Existing section 4 |
| C7 | Deterministic interval cascade (certified, with exact final rerank) | **Candidate.** The bound is elementary. Whether it prunes is an empirical question. The sources read give no equivalent deterministic prune | Master-prompt Candidate D; not tested here |
| C8 | Heuristic sigma/z gate | **Heuristic, not a guarantee.** Must never be labelled certified | Own Stage D results |
| C9 | No retraining under streaming | **Narrowed.** True for the code. Dynamically consistent data-dependent methods exist (2512.18335 builds one and reports it beats data-oblivious baselines); FreshDiskANN also handles updates. No comparison exists | Register entries 3, 14 |
| C10 | Updatable index with bounded update cost | **Prior art** (FreshDiskANN, 2512.18335 Thm 3.1) | Entries 3, 14 |
| C11 | Log-pinned snapshot per reader | **Prior art** (RocksDB, SQLite, RCU) | Entry 14 |
| C12 | Snapshots for a *progressively read* code (planes pinned consistently with segment publication) | **Candidate, unmodelled.** Plausibly an engineering combination, since planes of one vector live in one immutable segment | Existing section 5, item 3 |
| C13 | Filtered search | **No Prism contribution.** ACORN, UNG, SeRF, Filtered-DiskANN, CAPS and others cover it. Use pre-filter plus exact scan as the Prism baseline | Entries 4, 5, 8 to 10 |
| C14 | Read-while-write tail-latency advantage of immutable snapshots | **Unsupported.** Local measurement: median P99 +21 percent, throughput -12 percent, on a non-idle machine; no hour-long run; no comparison to a mutable baseline | HANDOFF_PRISM |
| C15 | Speed advantage | **Unsupported, locally contradicted.** LUT cascade 0.61 ms vs optimised exact scan 0.55 ms at n=20k | HANDOFF_PRISM |

**Summary.** The defensible position today is that Prism is an **engineering study of known components**, with two candidate contributions (C7 certified interval prune, C12 snapshot/plane interaction) that are unproven and may fail. C5 needs a further literature search before any novelty is claimed.

## 9. Evidence caveats

- Search-result summaries ([S]) are secondary; none of them may be cited as a reading of the paper.
- The OceanBase figures differ between its two posts per the summary; treat them as unsettled until the repo output is read.
- The P99 numbers in entry 6 were read from a summary of the HPC paper, not from the PDF.
- Fetch agent transcription of numbers (TurboQuant timings, ScaNN recall, HPC percentages) was not independently checked.
- Standing correction: the earlier claim "SIGMOD 2025" for UNG is not supported; and 1311.4055 is not OPQ.

## 10. Unresolved research questions

1. Does a deterministic interval bound over 1 to 4 plane prefixes prune anything useful at d = 256 to 1536 on real embeddings, versus RaBitQ's probabilistic bound? (Needs the Stage C derivation, then measurement, after S14.)
2. Is C5 (nested prefixes of one code) already in the multi-stage, residual or matryoshka quantization literature? Not searched yet.
3. What are RaBitQ's `c0` and practical `eps0`, and does its bound transfer to a 4-plane code? (RaBitQ's full text was read in Stage B; the transfer was not worked out.)
4. Can 2512.18335's datasets, code and hardware be reproduced? Its evaluation section was not re-read in P2.
5. Do the OceanBase raw outputs support its jitter figures, and are they reproducible with VectorDBBench's StreamingPerformanceCase on a controlled machine?
6. What are the licence, checksum and exact model revision for arxiv-for-fanns? SIFT1M and GIST1M terms?
7. Read next, in full: TurboQuant remainder and the ICLR PDF; FreshDiskANN sections 4 to 6; HPC PDF P99 section; VIBE OOD section; SeRF, UNG, Filtered-DiskANN; PQ (TPAMI 2011); ANN-Benchmarks and VectorDBBench documentation; SPANN.
8. What does a log position need to carry to name a *reconstructible* snapshot (applied index state, segment publication, tombstones, reclamation)? RocksDB/SQLite/RCU were read only through official summaries.
