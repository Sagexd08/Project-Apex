# Benchmark charter
Source of truth for scoreboards, protocol and claim rule: `docs/BENCHMARK_NUMBER_ONE_PLAN.md` sections 0 and Phase 1.2. Summary:
- Scoreboards S-ANN, S-DB, S-MEM; never merged.
- "#1" only on a scoreboard where Atlas leads on equal hardware, fairly tuned, second-run reproduced, raw data public. Until then the site says "measured".
- Enforced in code: `benchmarks/h2h/manifest.py` (manifest incl. source digest and idle-host measurement) and `summarize.py` (no valid manifest = NOT RANKABLE).
- Out of scope until evidenced: power loss, macOS.
- Provenance: Stage B source claims were supplied by the user as Gemini-verified. This repo has not re-read them; they are tagged "user-attested" in SOURCE_REGISTER_STAGE_B.md and need a primary-source recheck before any public claim.
