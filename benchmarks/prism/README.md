# Prism (research reference implementation)

Std-only Rust, detached workspace. Not part of the Atlas core build or audit.

- Randomised Hadamard rotation + nested 4-bit planes (any b-bit prefix is a valid quantiser).
- Error-gated cascade search (schedule 1,2,4 bits, calibrated sigma_b, exact rerank).
- MVCC snapshots: immutable segments, `RwLock<Arc<Snapshot>>`, single writer, compaction.

## Run

```
cd benchmarks/prism
cargo test --release                      # invariants I1-I4, nested, orthogonality, sigma
cargo clippy --release --all-targets -- -D warnings
cargo run --release --bin prism-bench     # writes results/results.json
```

Set `PRISM_CPU="<model>"` to record the CPU in results.json.

## Caveats

- Synthetic data only (`"synthetic": true`); recall numbers say nothing about real embeddings.
- Timings are from a non-idle machine; the decode loop is scalar (no LUT/SIMD), so the 4-bit
  scan is slower than the f32 exact scan. Bits-read is the meaningful metric here.
- No deletes in v0. Ids must be unique.
