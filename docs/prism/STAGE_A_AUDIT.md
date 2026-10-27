# Stage A: repository audit and production boundary (2026-10-27)

Scope: master prompt section 1. Read-only inspection, no source changes. Status: **Measured** (commands below).

| Item | Finding |
|---|---|
| Branch / head | `mvp-cli-and-prism-lab` at `6686ba8`, working tree clean before this stage |
| Atlas source digest | `95656fed761009b50a47b9cd6f10a23b` (47 files, `bash scripts/digest.sh`). Windows and Linux Docker verification both passed 149 tests on this digest (builder-run, not independently reviewed) |
| Active jobs | No `cargo` or `rustc` process; no Atlas verification or review running; Docker only runs unrelated containers |
| Production boundary | Prism is `benchmarks/prism`, a **detached, std-only workspace** (`[workspace]` in its own `Cargo.toml`, no dependencies, no `unsafe`). No `atlas-*` crate depends on it. Prism research code must stay there; the WAL contract and Gate 2 evidence are not touched by stages A to D |
| Existing Prism artifacts | `benchmarks/prism/{src,tests,results/results.json}` (917 lines of Rust, 12 tests per its README), `docs/prism/{ABSTRACT,MANIFESTO,MATH_AND_ALGORITHM,PAPER,WORDING_PASS}.md`, the site |
| Path convention | The master prompt names `research/prism/`; this repo already uses `benchmarks/prism/` for code and `docs/prism/` for documents. Stages A to D use those, and create no `research/` directory |
| Licence state | Workspace manifests declare `Apache-2.0`. **There is no `LICENSE` file, no third-party notice inventory, no SBOM.** Section 0 says to audit before changing: that audit is not done, and no licence file was added |
| Dependencies | Prism: none. Atlas: `crc32fast`, `serde`, `serde_json`, `thiserror`, all exact-pinned; `cargo-deny` passes (advisories, bans, licences, sources). USearch is **not** adopted and **not** audited |
| Git | No remote changes were made in this stage. Nothing was committed by Stage A itself |

## Open risks found

1. No `LICENSE` file while manifests claim Apache-2.0 (Section 0.7 gap).
2. The earlier Prism numbers: `results.json` records `cpu_note: "unknown"`, no dataset checksum and no per-result command (from the wording pass). Stage D addresses this.
3. `docs/` is gitignored except `docs/prism` and `docs/brand`, so Atlas gate evidence is local-only.
