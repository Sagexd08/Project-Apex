# Prism wording pass (Research Note v1.0)

Scope: docs/prism/*.md, site/prism.html, site/css/prism.css, site/js/prism-evidence.js. No crates, ci, scripts, Cargo, benchmark source or GATES.md touched. Nothing committed.
Evidence file for all numbers: `benchmarks/prism/results/results.json` (checksum `103fcb801fb94647`). Re-derived from it this pass: bits 1.06 and 1.056 (bits_incr_mean / (20000*256)), recall 0.998 at z=2, 3.8x (4/1.06), 30x (32/1.06), sigma ratios 13% and 3%, kurtosis -0.02 to 0.23, max error 5.2 to 7.1 sigma, per-vector violation rate 0.0076% to 0.055%, deterministic-bound bits 3.73 to 4.0, median P99 ratio 1.21 (1.12 to 1.34), QPS 6007 to 5255, harder-set bits and recall, kernel timings. All matched PAPER 6.1, 6.2, 6.6.

## Claims changed

| Where | Before | After | Evidence / status |
|---|---|---|---|
| ABSTRACT, PAPER 6.2, site abstract, KPIs | "returned the exact top 10 for every query" at z=3 | "same top 10 as exact search for all 1,000 queries ... observation, not a guarantee" | results.json `cascade` z=3 miss_rate 0; heuristic gate, relabelled |
| ABSTRACT, PAPER header, MATH header, site | (no statement) | global sigma gate = heuristic; certified = deterministic bound, reads 3.7 to 4.0 bits | results.json `extra.gates` rows |
| ABSTRACT, PAPER 3-4, site KPI and evidence | (absent) | bits read is not memory; float32 retained; total about 1.125x float32; 4-bit planes alone 1/8 | arithmetic (4d + 32d bits vs 32d). Float32 on a slower tier is Proposed, unmeasured |
| ABSTRACT | "published measurements show tail latency inflating" | "other work reports ... (reported, not reproduced here)" | relabelled |
| ABSTRACT | "never waits ... never sees a partial write" | "takes no lock after pinning, does not see a partial write (implemented and tested for one writer, four readers)" | relabelled; tests not re-run this pass |
| ABSTRACT, PAPER, site | "Research draft" | "Research Note v1.0 (preprint, not peer reviewed)"; no authors or affiliations | master prompt section 7 |
| PAPER 6.4 | exact 3.65/4.51, flat4 7.09/9.42, z2 4.37/5.54, z3 4.68/6.21 ms (stale earlier run, checksum 50b8...) | current `wall_clock`: 3.75/5.09, 6.81/9.66, 3.45/4.14, 5.70/14.54 ms; bold "slower than exact scan" removed; marked superseded by 6.6 | results.json `wall_clock`; the file does not record the kernel per row |
| PAPER 6.5 | 5.30/9.68/19.51 vs 5.27/7.79/10.91 ms, ratio 0.56 (stale) | 5.09/6.40/6.80 vs 5.97/8.09/8.93 ms, ratio 1.31, one run, not conclusive | results.json `concurrency` |
| PAPER header | checksum 50b8bcc7fdb6bf7c | 103fcb801fb94647 | results.json |
| PAPER 6.6 | per-vector bound "held empirically" | adds "probabilistic, not certified" | relabelled |
| PAPER 2 | "What we claim is narrower" | "What we propose ... we do not claim priority" | related-work pass incomplete |
| PAPER 4, MATH lemma, site | "z near 4.2" | adds Proved under sub-Gaussian assumption; measured tails heavier, so z gate is a heuristic | MATH section 4; results.json `error_dist` |
| MANIFESTO | "wins on time, and should also win on energy" | "may win ... not yet shown" | time saving not shown: 0.61 vs 0.55 ms |
| MANIFESTO | "cannot drift", "never waits for a writer" | "no trained state to drift", "takes no lock"; readers still share CPU and memory (median P99 +21%) | results.json `concurrency_repeats` |
| MANIFESTO, site north star | "The best open-source vector database of this decade ... better" | "Proposed goal, not a claim ..." | superlative removed; no comparison exists |
| MANIFESTO | falsification list only | explicit thesis plus "fails if" | master prompt section 8 |
| site hero | "One stored code. Any precision." + draft tags | master prompt headline, supporting text, status line, two primary and two secondary actions | master prompt section 9 |
| site title, meta, ticker, footer | "research draft", "simulation only" | Research Note v1.0, preprint, not peer reviewed, research prototype | |
| site evidence section | no status note | note: Measured, synthetic; bits read not memory; z gate heuristic | |

## Status vocabulary
Defined in the PAPER.md header (Proved / Implemented / Tested / Measured / Reported-not-reproduced / Proposed / Unmeasured / Failed) and used inline in ABSTRACT, PAPER and MATH. Not applied row by row to the site evidence tables.

## Could not trace or still open
- The seven Rust tests and the 2-second invariant test (PAPER 5): not run this pass.
- PAPER 6.3 "numpy prototype read 1.77 bits" and MATH sigma 0.0576 from the numpy prototype: no file under benchmarks/prism/results.
- External numbers (arXiv 2606.08950 280% to 345%, OceanBase 1.1x to 10.3x, arXiv 2512.18335, TurboQuant 2.7x): from abstracts only, unverified; labelled reported.
- results.json `cpu_note` is "unknown", run on a non-idle machine; dataset checksum and command per result not recorded in the file.
- No .tex/.pdf/.docx or references.bib (master prompt section 7). Not created.
- The site paper accordion is a hand-kept condensed copy of PAPER.md; only the edits above were synced.
- Related-work and novelty pass incomplete; the narrowed claim is tentative.

## Site UX (read from CSS and JS, no screenshots, no browser run)
- Focus: global `:focus-visible` outline in main.css, with dark-section variant.
- Reduced motion: CSS blocks in main.css and prism.css disable animation and transitions; main.js, prism.js and prism-film.js check the media query (film does not autoplay).
- Links: all in-page anchors (`#abstract #film-sec #manifesto #lab #algorithm #paper #evidence`) resolve to existing ids; external links use https://github.com/Sagexd08/Project-Apex. No fake live metrics; evidence is fetched from `data/prism-results.json`. Button handlers exist in prism.js and prism-film.js but were not exercised.
- Mobile (~375px): hero single column below 900px, KPI grid two columns, wide tables scroll inside `.tbl.wrapc`. h1 max-width raised from 12ch to 18ch for the longer headline; wraps to about four lines at 375px (not checked visually).
- Not done: console-error check; the master prompt navigation entries Related work, Tournament, Loopholes, Roadmap have no pages and were not added.

## Lint/build
`site/package.json` has only the dependency `@lucasmarkes/hairline` and no scripts, so there is no lint, build or test command. Only check run: Node parse of `site/js/prism-evidence.js` (ok).
