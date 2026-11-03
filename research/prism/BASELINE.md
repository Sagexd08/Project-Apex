# Prism research baseline (P1, 2026-10-30)

Read-only audit. No build, test, benchmark, Docker command or browser was run. Source digests below were computed with `bash scripts/digest.sh`.

## Git
- Branch `main`, HEAD `785478751d345261b11f99e926b14b496dd488b5` (26 commits). No worktree created.
- Working tree dirty: 58 paths (33 modified, rest untracked). Not committed; preserve as is.
- Modified/untracked groups: `site/` (Prism site rework), `adapters/atlas-usearch/` (+ `tests/f16.rs`), `benchmarks/h2h/` (S14 harness, `.s14-target/`, `profiling/`, `src/bin/`), `prototypes/`, `research/`, `.claude/settings.json`.
- `crates/`, `ci/`, `Cargo.toml`, `Cargo.lock`, `deny.toml` are NOT in the dirty list, so the Gate 2 source tree matches HEAD.

## Source digest (scripts/digest.sh)
- `files=60 digest=9b97729a3ea10a342c6192cf1db1fd86` at the time of this audit. Not yet compared to the last digest recorded in `docs/GATES.md` (unverified).

## Active jobs
- **S14 benchmark is RUNNING.** Docker container `atlas-h2h` (ca619d41c4a9), up about 2 h, PID 1 `python hnsw_bench.py /cache/sift .../raw/s14/sift/hnswlib.json --reps 3`. Raw output dir `benchmarks/h2h/profiling/raw/`. Treat S14 as OPEN (user, 2026-10-30).
- Other containers (Supabase stack) are unrelated to Atlas.

## Existing Prism material (pre-dates this task)
- `docs/prism/`: ABSTRACT, CERTIFIED_CASCADE, MANIFESTO, MATH_AND_ALGORITHM, PAPER, RELATED_WORK_AND_CLAIM_AUDIT (87 lines, partial Stage B), SNAPSHOT_PROTOCOL, STAGE_A_AUDIT, STAGE_D_REPRODUCTION, WORDING_PASS.
- `docs/PRISM_MASTER_PROMPT.md` (1087 lines), `docs/HANDOFF_PRISM*.md`, `benchmarks/prism/` (per HANDOFF_PRISM; not inspected in P1).
- `research/market/`, `research/ATLAS_PRD_AMENDMENTS.md`.

## Discrepancies to resolve
1. The task brief said there were no commits; there are 26. HANDOFF_PRISM.md says main = `3494d81`; HEAD is now `7854787`.
2. Prism docs live in `docs/prism/`, not `research/prism/`. P2 extends `docs/prism/RELATED_WORK_AND_CLAIM_AUDIT.md` and mirrors it here only if the user wants a second copy.

## Boundaries
No change to production crates, WAL contract, Gate 2 evidence, or any existing file. Gate 2 stays OPEN.
