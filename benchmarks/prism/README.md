# Prism (research reference implementation)

Std-only Rust, no unsafe, detached workspace. Not part of the Atlas core build or audit.

- Randomised Hadamard rotation + nested 4-bit planes (any b-bit prefix is a valid quantiser).
- Error-gated cascade search (schedule 1,2,4 bits, exact rerank). Gates: global `z*sigma_b`,
  per-vector residual (`PerVector`, heuristic), Cauchy-Schwarz (`Deterministic`, sound).
- LUT kernel (`src/lut.rs`): per-query byte tables give each plane's dot product with 32 lookups;
  the cascade keeps each survivor's partial plane sums, so a stage reads only its new planes.
  The scalar decode path (`quant::score`, `search_cascade`) is kept as the reference.
- Each segment stores per-vector residual norms `||x - x_hat_b||`, b = 1..4.
- MVCC snapshots: immutable segments, `RwLock<Arc<Snapshot>>`, single writer, compaction.

## Run

```
cd benchmarks/prism
cargo test --release                      # 14 tests: invariants, nested, sigma, LUT == reference, gates
cargo clippy --release --all-targets -- -D warnings
cargo run --release --bin prism-bench     # ~2 min; writes results/results.json
cargo run --release --bin prism-timing    # ~9 min; interleaved timing, writes results/timing_interleaved.json
```

`prism-timing [--n 20000,100000,400000] [--reps 20] [--queries 200]` times the exact scans, a flat 4-bit LUT and
the cascade in a rotating order per query, repeated, and reports each kernel's p50 ratio to the faster exact scan with a
bootstrap 95% interval and recall against exact. Results record the command, an LF-normalized source digest
(`src_fnv1a64_lf`) and a dataset checksum (`dataset_fnv1a64`). Raw bytes digest `src_fnv1a64` depends on line endings.

Set `PRISM_CPU="<model>"` to record the CPU. Old keys of results.json are unchanged; new data
is under `"extra"` (kernel_timing, datasets, error_dist, gates, concurrency_repeats).
`src_fnv1a64` covers every `.rs` under `src/` (including `src/bin/`).

## Findings (synthetic data, 12-thread non-idle Windows box, single-thread timings)

- Kernel: LUT cuts the cascade ~7x (scalar z=2 4.5 ms -> 0.61 ms p50 in the first run). A single sequential
  run could not separate it from the optimised f32 scan (sign flipped between two runs). The interleaved
  run (`prism-timing`, see docs/prism/STAGE_D_REPRODUCTION.md D.5) measured the cascade at 0.62-0.70 of the
  faster exact scan at n = 20k, 100k and 400k, intervals excluding 1, recall at z=3 equal to 1.0. Still synthetic,
  one thread, machine not idle, exact baseline not hand-written SIMD: an observation, not a speed claim.
- Error: the score error is close to Gaussian in the bulk (excess kurtosis -0.02..0.23) but
  the tails are heavier than 3.29/3.89: max |e|/sigma is 5.2-7.1 over 20M pairs, so zero
  misses over all pairs needs z ~ 7, far above the z=2..3 operating points.
- Per-vector gate (delta=1e-3, h = r*sqrt(2 ln(2/delta)/P)): measured violation rate of
  |s_hat - s| > h is 8e-5..5.5e-4 per pair (b=1 worst), below delta on these data but not
  guaranteed (heuristic; depends on the rotation). Zero misses, but it reads more bits than
  global z=3 (e.g. dataset b: 15.2M vs 11.4M) and sits between z=3 and z=4: no gain.
- Deterministic gate (|s_hat - s| <= r): sound (0 violations) but prunes almost nothing at
  d=256: stage 1 keeps all 20000 on datasets b-d and 17274 on a (19-20M bits vs ~5.4M).
- Harder data costs bits: at z=2 the cascade reads 1.04-1.05 bits/coord (in-distribution
  clustered, overlapping, n=100k) but 1.30 (isotropic) and 1.40 (OOD queries); at z=4, OOD
  reads ~3.1 bits/coord. The ~1.77 bits of the numpy prototype is not reproduced.
- Concurrency: writer ON vs OFF, 7 alternating trials, median p99 ratio 1.21 (min 1.12, max
  1.34), reader throughput 5255 vs 6007 q/s. The writer costs readers ~12% throughput; the
  ratio is consistently >1, so this is not just noise, but the writer is throttled (~1900 vec/s).

## Caveats

- Synthetic data only (`"synthetic": true`); recall numbers say nothing about real embeddings.
- Timings come from a non-idle machine. Dataset time_p50 uses the first 200 queries only.
- No deletes in v0. Ids must be unique. macOS and real-embedding runs are not done.
