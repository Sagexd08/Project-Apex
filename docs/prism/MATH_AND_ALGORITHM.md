# Prism: mathematics and algorithm, end to end

Status: research draft. Derivations are first-order and state their assumptions. Empirical checks are in `PAPER.md` section 6 and `benchmarks/prism/results/results.json`. Nothing here is a benchmark against another system.

Notation: `d` is the vector dimension, `P` the next power of two at or above `d`, `n` the corpus size, `k` the result size, `q` a query and `x` a stored vector, both unit length. `s(q,x) = <q,x>` is cosine similarity.

## 1. Geometry: why bits can rank neighbours

For unit vectors, `<q,x> = cos θ`, so ranking by inner product is ranking by angle.

Take a random hyperplane through the origin with unit normal `g`. The sign bits `sign<g,q>` and `sign<g,x>` differ exactly when the hyperplane separates the two vectors, which happens with probability

```
Pr[ sign<g,q> != sign<g,x> ] = θ / π            (Goemans and Williamson)
```

With `d` independent hyperplanes, the Hamming distance `H` between the two bit strings is Binomial(`d`, `θ/π`). That gives an angle estimator and its spread:

```
θ̂ = π · H / d
E[θ̂] = θ
std(θ̂) = π · sqrt( p (1 - p) / d ),      p = θ / π
std(cos θ̂) ≈ sin θ · std(θ̂)
```

| true angle | std of θ̂ at d = 256 | std of cos θ̂ |
|---|---|---|
| 30° | 4.19° | 0.037 |
| 60° | 5.30° | 0.080 |
| 90° | 5.62° | 0.098 |
| 120° | 5.30° | 0.080 |

Reading: a 1-bit code is sharpest for near neighbours (small `sin θ`) and blurriest for unrelated vectors. Nearest-neighbour search only needs to be sharp where the answer is, so the cheapest code is already most precise where it matters. The error shrinks as `1/sqrt(d)`; that is a property of dimension, not of tuning.

## 2. Rotation: making every vector look the same

A stored vector can have any shape: a few large coordinates, or many small ones. A scalar quantizer does badly on the first shape and well on the second. A random rotation removes the difference.

If `R` is a Haar-random orthogonal matrix and `x` is a unit vector, each coordinate of `Rx` has density proportional to `(1 - t²)^((d-3)/2)`, mean 0 and variance `1/d`. For large `d` this is close to `N(0, 1/d)`, and coordinates are nearly independent. The distribution of `Rx` does not depend on `x`. This is the reason the quantizer needs no training: the data is made to look the same before it is quantized.

A dense `R` costs `O(d²)` per vector and `d²` floats to store. Prism uses a randomized Hadamard rotation instead:

```
rotate(x):
    z = zero_pad(x, P)
    repeat 3 times:
        z = z ⊙ D_r              # D_r: seeded random ±1 diagonal
        z = FWHT(z) / sqrt(P)    # fast Walsh-Hadamard transform, O(P log P)
    return z
```

Each round is orthogonal, so `‖z‖` is preserved. Three rounds are a standard approximation of a Haar rotation. The only state is the seed. The approximation is an assumption of the analysis below, and the test suite checks norm preservation and the measured error against the formula.

## 3. Quantizer: nested bit planes

Let `s_c = C / sqrt(P)` with clip constant `C = 3` (three standard deviations of a coordinate). For a rotated coordinate `z`, the 4-bit index and its levels are:

```
Δ_4   = 2 s_c / 16
idx_4 = clamp( floor( (z + s_c) / Δ_4 ), 0, 15 )
```

The `b`-bit index is the top `b` bits of `idx_4`:

```
idx_b = idx_4 >> (4 - b),      Δ_b = 2 s_c / 2^b
recon_b(idx) = (idx + 0.5) · Δ_b - s_c
```

Nesting holds because `Δ_b = 2^(4-b) Δ_4` and clipping is applied to the same interval, so `floor((z + s_c) / Δ_b) = floor(idx_4 / 2^(4-b))`. Plane 0, the most significant bit, is `[z ≥ 0]`: it is the SimHash bit of section 1. Each extra plane halves the cell width.

Storage layout: four planes, each `P` bits per vector, packed in `u64` words. One stored copy therefore serves 1, 2, 3 or 4 bit scoring, and a query reads only the planes it needs.

### Error of the estimated score

The estimate at `b` bits is `ŝ_b = <Rq, recon_b(Rx)>`, and the error is

```
ε_b = ŝ_b - s = Σ_i (Rq)_i · e_i,      e_i = recon_b(z_i) - z_i
```

For fine cells, `e_i` is close to uniform on `[-Δ_b/2, Δ_b/2]` with variance `Δ_b² / 12`, and `Σ (Rq)_i² = 1`. So

```
Var(ε_b) ≈ Δ_b² / 12 = C² / (3 P · 4^b)
σ_b ≈ C / ( sqrt(3 P) · 2^b )
```

For `P = 256`, `C = 3`: `σ_1 ≈ 0.054`, `σ_2 ≈ 0.027`, `σ_4 ≈ 0.0068`. A first numpy prototype measured 0.0576, 0.0272, 0.0069. The coarse 1-bit case sits above the formula because the uniform-error assumption is weakest there and clipping adds error.

Consequences:

- Each added plane halves the error. Four planes cut it by 16.
- The error does not depend on the data, only on `P`, `C` and `b`. This is the formal version of "nothing to train, nothing to drift".
- This is an MSE quantizer, so `ŝ_b` is slightly biased toward zero. A residual sign bit (the QJL step in TurboQuant) makes it unbiased. Prism v0 does not include it.

## 4. The gate: when it is safe to drop a candidate

Let `t` be the true k-th best score, and `τ_j` the k-th best estimate among current candidates at stage `j`. The gate keeps a candidate `c` if

```
ŝ_j(c) + z σ_j  ≥  τ_j - z σ_j
```

**Lemma (gate safety).** Suppose, at stage `j`, every true top-k item `c` and one witness item `w` satisfy `|ε_j| ≤ z σ_j`. Then no true top-k item is dropped.

Proof. At least `k` candidates have estimate `≥ τ_j`. Fewer than `k` items have true score `> t`, so at least one of those `k` candidates, call it `w`, has `s(w) ≤ t`. Then `τ_j ≤ ŝ_j(w) ≤ s(w) + zσ_j ≤ t + zσ_j`. For a true top-k item `c`, `ŝ_j(c) ≥ s(c) - zσ_j ≥ t - zσ_j ≥ τ_j - 2zσ_j`, which is the gate condition. ∎

The lemma is conditional. To make it unconditional, bound the probability that the assumption fails. Over `S` stages and `k + 1` items, a union bound gives

```
Pr[ any true top-k item is dropped ]  ≤  (k + 1) · S · Pr[ |ε| > z σ ]
```

If the error is sub-Gaussian with variance proxy `σ²`, `Pr[|ε| > zσ] ≤ 2 exp(-z²/2)`, so a failure budget `δ` needs

```
z  ≥  sqrt( 2 ln( 2 (k + 1) S / δ ) )
```

For `k = 10`, `S = 3`, `δ = 0.01`: `z ≈ 4.2`. A Hoeffding bound on a bounded uniform error has variance proxy `Δ²/4`, three times the true variance, which pushes the provable `z` higher still. In the experiments the useful range is `z ≈ 2` to `3`.

**Open problem.** The provable `z` (above 4) costs most of the saving that an empirical `z` of 2 gives. Closing that gap, with per-vector bounds of the kind RaBitQ derives, is the main research question. Until it is closed, the correct statement is: the gate is empirically calibrated and its miss rate is measured, not guaranteed.

**Measured tails.** Over 20 million (query, vector) pairs per set, the score error was close to Gaussian in shape (excess kurtosis -0.02 to 0.23) with maximum |error|/sigma of 5.2 to 7.1, so zero misses over every pair needs z near 7.1. A per-vector bound using the stored residual norm `r_x`, `|s_hat - s| <= r_x * sqrt(2 ln(2/delta) / d)`, held on at least 99.94% of pairs at delta = 0.001 but read about as many bits as z = 4. The deterministic bound `|s_hat - s| <= r_x` (Cauchy-Schwarz) never missed and pruned almost nothing at d = 256, because it ignores that the error direction is random relative to the query. See `PAPER.md` section 6.6.

## 5. Cost model: bytes touched

Stage 1 reads `b_1` bits for all `n` vectors. Stage `j > 1` reads the new planes for the `n_{j-1}` survivors. Define `m_j = b_j - b_{j-1}` with `b_0 = 0`:

```
B = n · m_1 + n_1 · m_2 + n_2 · m_3          bits per query, incremental planes
B_prefix = n · b_1 + n_1 · b_2 + n_2 · b_3   if each stage re-reads its whole prefix
```

The experiments report both. A rerank fetches `n_S` full-precision vectors.

**Survivor count, first order.** A candidate survives stage `j` if its estimate is within about `2zσ_j` of the threshold, so

```
n_j  ≈  k  +  n · g(t) · 2 z σ_j
```

where `g(t)` is the density of the similarity distribution at the threshold (per unit similarity, per vector). The excess over `k` is proportional to `σ_j`, so each extra bit halves it. The approximation needs the window `2zσ_j` to be small compared to the scale on which `g` changes. At 1 bit the window is wide and the formula overestimates; the measured survivor counts are in the paper.

**Floor.** With one bit per vector read in stage 1, `B ≥ n` bits. Prism accelerates a scan. It does not remove the scan. Going below `n` bits per query needs a partitioning index (cluster or graph) in front of it, which is future work and is where the ANN spike in `docs/ANN_SPIKE.md` is relevant.

**What is stored.** Prism does not reduce stored bytes: it keeps the planes (`P/2` bytes per vector at 4 bits) and full-precision vectors for the rerank. It reduces bytes read per query. The full-precision vectors can sit on a slower tier because only `n_S` of them are fetched per query.

## 6. Tail latency

Per-query cost varies with how many vectors sit near that query's threshold, `n · g(t_q)`. Queries in dense regions have more survivors. The experiments report the mean and the 95th percentile of bits read and of reranks, because the tail depends on the spread of `g(t_q)` across queries, not on the mean.

For reads against writes, the design aim is that a reader's cost is independent of the writer's activity, which would make the concurrent P99 equal the serial P99 apart from memory-bandwidth sharing. The experiment measures the ratio concurrent P99 over serial P99 as a jitter ratio.

## 7. Snapshots

**Definitions.** The log assigns each commit a sequence number `ℓ ∈ ℕ`. A snapshot pin `p = (ε, ℓ)` names an epoch `ε = (seed, d, schedule)` and a position `ℓ`. The visible set is

```
V(p) = { r : lsn(r) ≤ ℓ }        (deletes are future work: add  and  del_lsn(r) > ℓ )
```

**Invariants.**

- I1 Repeatability. For a fixed pin and query, the result is identical every time, regardless of concurrent commits.
- I2 No torn reads. A commit is visible whole (all planes and the full-precision vector) or not at all.
- I3 Monotonicity. Successive pins by one reader have non-decreasing `ℓ`.
- I4 Compaction transparency. Compaction changes layout, never `V`.

**Mechanism.** Segments are immutable once sealed. The store holds a pointer to the current snapshot, a list of segment pointers plus its `ℓ`. A commit builds one new sealed segment and swaps in a new snapshot that lists the old segments plus the new one. A reader clones the current pointer (a read lock held for a pointer copy) and then never takes a lock again. I1 and I2 follow because a snapshot never changes after publication, and I3 follows because the pointer only moves forward. I4 holds because compaction replaces several segments with one holding the same records.

Because the quantizer has no trained state, a writer encodes a new vector from the seed alone. There is no codebook to retrain, no global rebuild, and no moment when stored codes and the quantizer disagree.

Cost: many tiny segments make a scan visit many small blocks. Compaction merges them, and the benchmark reports throughput with and without compaction.

## 8. Algorithms

```
ENCODE(x, seed):                          # writer, per vector, O(P log P)
    z   = rotate(x, seed)
    idx = clamp(floor((z + s_c) / Δ_4), 0, 15)
    for plane p in 0..4: bit p of every coordinate -> packed words
    return (planes, x)                     # x kept for rerank

COMMIT(batch):                             # single writer
    seg  = seal(ENCODE(v) for v in batch)
    lock writer mutex
        snap' = Snapshot{ lsn = cur.lsn + 1, segments = cur.segments + [seg] }
        publish snap'                      # pointer swap under a short write lock
    return snap'.lsn

SEARCH(q, k, schedule=[b_1..b_S], z, snap):
    rq    = rotate(q, seed)
    cand  = all vectors in snap
    for j in 1..S:
        est = { c : ŝ_{b_j}(rq, c) }       # read planes up to b_j
        τ   = k-th largest of est
        cand = { c : est[c] + z σ_j ≥ τ - z σ_j }
    return top-k of cand by exact <q, x>, ties by ascending id

CALIBRATE(sample of (q, x) pairs):
    σ_b = std( ŝ_b(q, x) - <q, x> )  for b in schedule
```

`SEARCH` scores each survivor from planes it already holds plus new planes; the reference implementation counts bits both ways. `CALIBRATE` runs once per epoch.

## 9. Mapping to Atlas

`AnnIndex` in `crates/atlas-index` has `insert`, `remove`, `search` and `search_filtered`. A `PrismIndex` would implement it, with the commit path fed from the WAL (the log position is `ℓ`). Filtered search needs the allow-list applied inside the cascade (drop non-allowed candidates before stage 1), and the ANN spike showed that very selective filters need an exact scan of the small allowed set instead. That integration is not done and is outside the current gates.

## 10. Reproducing

```
cd benchmarks/prism
cargo test --release
cargo run --release --bin prism-bench
```

Results go to `benchmarks/prism/results/results.json` with a checksum of the source files that produced them.
