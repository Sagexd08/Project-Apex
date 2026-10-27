# Stage D: reproduction of the synthetic pilot (2026-10-27)

Status: Research Note v1.0, not peer reviewed. Run by the author on one machine; no independent verification. Result: **deterministic quantities reproduce exactly; the time comparison does not reproduce and is inconclusive.**

## D.1 What was run

```
cd benchmarks/prism
cargo test --release                       # 12 passed, 0 failed
cargo clippy --release --all-targets -- -D warnings   # clean
PRISM_CPU="AMD Ryzen 5 7640HS (12 threads), Windows 11, NOT idle (Docker containers and an IDE running)" \
  cargo run --release --bin prism-bench    # writes results/results.json
```

| Item | Value |
|---|---|
| Parameters | `n = 20000`, `d = 256`, `nq = 1000`, `k = 10`, `seed = 7`, synthetic clustered data (from `results.json` `params`) |
| Toolchain | rustc 1.95.0, release profile, std only, no dependencies |
| Machine | AMD Ryzen 5 7640HS, 12 threads, Windows 11. **Not idle**: other containers and an IDE were running. The original run recorded `cpu_note: "unknown"` |
| Atlas digest at run time | `95656fed761009b50a47b9cd6f10a23b` (unrelated to Prism; Prism is a detached workspace) |
| Prism source, raw bytes of working copy (CRLF) | `7f8c5061ca1f7648` (what `prism-bench` recorded for this run) |
| Prism source, LF-normalized = committed blobs at `aba1d21` and `HEAD` | `093fb242cdcae348` |
| Raw output | `benchmarks/prism/results/repro_2026-10-27.json`, `benchmarks/prism/results/repro_2026-10-27.stdout.txt` |
| Original results (kept unmodified) | `benchmarks/prism/results/results.json`, recorded source hash `103fcb801fb94647` |

## D.2 The recorded source hash: explained

The original `results.json` records `103fcb801fb94647`, but the sources committed with it hash to `093fb242cdcae348`. Checked by recomputing the benchmark's own FNV-1a 64 over the 9 `.rs` files in sorted path order (an independent Node implementation reproduces this run's recorded value, so the method is right):

- The committed content of `aba1d21` reproduces `103fcb801fb94647` exactly when `src/bin/prism-bench.rs`, `src/search.rs` and `src/store.rs` have CRLF line endings and the other six files have LF. That is a plausible state of the working copy during the original run.
- So the original results were produced from source **content identical to the committed source**; only line endings differed.
- The hash covers raw bytes, so it changes with line endings (`autocrlf=true` here). **Recommendation (not applied, to avoid changing the source being measured):** hash LF-normalized bytes so a digest names content, not a checkout.
- The earlier draft commit `37e1bc8` records `50b8bcc7fdb6bf7c`, which no line-ending combination reproduces. That run is superseded and was not reproduced.

## D.3 Comparison of the two runs

| Class | Count | Result |
|---|---|---|
| Numeric leaves compared | 914 (same keys in both) | |
| Identical | 795 | Recall, miss rate, survivors per stage, bits read (prefix and incremental, mean and p95), reranks, flat-baseline recall, error statistics |
| Timing values | about 114 | Differ, as expected |
| Metadata | 2 | Source hash (D.2) and CPU note |
| Window-dependent counters | 3 | Queries completed in a fixed concurrent window and writer throughput |

Deterministic checks the run itself makes against exact search:
- `huge_z_cascade_equals_exact`, `lut_cascade_with_huge_z_equals_exact`, `exact_fast_matches_exact`, `lut_scores_equal_reference_decode`, `nested_prefix_equals_direct_quantisation`, `compaction_does_not_change_results`, `deterministic_gate_never_misses_on_200_queries`, `mixture_cascade_recall_is_high_at_z3`: all pass.
- At z = 3 recall is 1.0 and the miss rate 0.0; at z = 2 recall 0.9982, miss rate 0.017; at z = 1 recall 0.8559 (identical in both runs).
- Bits read at z = 3: mean 5,426,000 incremental bits per query, which is 1.06 bits per coordinate per vector (5,426,000 / (20,000 x 256)), against 4 for a flat 4-bit scan.

## D.4 The time comparison does not reproduce

Single-thread p50 and p95 per query, milliseconds, same machine, minutes apart:

| Kernel | First run p50 | Re-run p50 | First run p95 | Re-run p95 |
|---|---|---|---|---|
| exact_scan_ref (scalar decode) | 4.28 | 3.51 | 4.96 | 3.96 |
| **exact_scan_opt** (optimised f32 scan) | **0.550** | **0.633** | 0.78 | 0.82 |
| flat4_lut_rerank100 | 1.31 | 0.87 | 1.57 | 1.29 |
| **cascade_lut_z2** | **0.606** | **0.477** | 0.79 | 0.65 |
| cascade_lut_z3 | 0.664 | 0.522 | 0.86 | 0.80 |
| cascade_scalar_z2 | 4.54 | 3.48 | 5.63 | 4.40 |

- First run: cascade z = 2 was 10 percent slower than the optimised scan at p50 (0.606 against 0.550 ms).
- Re-run: it was 25 percent faster (0.477 against 0.633 ms).
- The optimised scan got slower between runs while everything else got faster by 18 to 34 percent: the pattern of interference or frequency scaling, not of a code change (the code is identical).
- Concurrent-read jitter ratio (p99 with writer over p99 without): median 1.21 in the first run, 1.15 in the re-run; single trials ranged 1.12 to 1.34 first, and 0.71 to 1.82 in the re-run. A ratio below 1 is noise, which shows what the machine does to one trial.

**Conclusion.** The bytes-read saving is deterministic and reproduces. Whether it becomes a time saving at n = 20,000 is **inconclusive**: the sign flipped between two single runs, each configuration is timed once and in sequence, and the machine was not idle. No speed claim is made in either direction. Every page that stated "not faster" was changed to say this.

## D.5 Interleaved timing run (added after D.4)

Tool: `cargo run --release --bin prism-timing` (output `results/timing_interleaved.json` and `.stdout.txt`). Kernels are timed in a rotating order per query, 20 repetitions of 200 queries, one thread, synthetic clustered data, d = 256, k = 10. Ratio = kernel p50 over the faster of two exact scans (`exact_scan_opt` and a streaming exact scan, `search_exact_stream`, tested equal to the first including ties), per repetition; interval = bootstrap over the 20 repetitions. Recall is against the exact scan on the first 50 queries. The results file records the command, the LF-normalized source digest and the dataset checksum.

| n | exact (faster) p50 ms | cascade z=2 ratio [95%] | recall | cascade z=3 ratio [95%] | recall |
|---|---|---|---|---|---|
| 20,000 | 0.712 | 0.649 [0.608, 0.665] | 0.998 | 0.666 [0.632, 0.681] | 1.000 |
| 100,000 | 4.058 | 0.677 [0.671, 0.686] | 1.000 | 0.697 [0.691, 0.708] | 1.000 |
| 400,000 | 19.723 | 0.615 [0.603, 0.631] | 0.988 | 0.662 [0.649, 0.683] | 1.000 |

In this run the cascade was faster than the stronger exact baseline at all three sizes, with intervals that exclude 1. The earlier sign flip (D.4) is not reproduced here. Limits, which keep this a note and not a claim: the machine was still **not idle**; one synthetic dataset; single thread; the exact baselines are not hand-written SIMD, so a stronger exact scan could narrow the gap; the intervals cover run-to-run variation inside one process, not different machines. Treat it as a first positive timing observation under those conditions, and re-run on an idle machine before any public speed statement.

Still needed: an idle machine with recorded frequency, SIMD exact baseline, real data, a second machine.

## D.6 Stage D acceptance

| Condition (master prompt) | Status |
|---|---|
| Synthetic pilot reproduced or marked unreproduced | Deterministic results reproduced; timing marked inconclusive |
| Tested against exact ground truth | Yes, inside the benchmark and by 12 tests; adversarial inputs not yet (Stage E) |
| Result tied to a source digest | Yes (D.1); the original's digest explained (D.2) |
| Raw outputs preserved | Yes |
| Dataset checksum | Recorded by the current tools (`dataset_fnv1a64`); the original results file lacks it |
| Independent verification | None |
