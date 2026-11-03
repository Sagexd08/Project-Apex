# Amendments to "Atlas Enhanced PRD v2.0" (2026-10-30)

Apply to your copy of the PRD. Based on `research/market/COMPETITIVE_LANDSCAPE.md` (first pass, not primary-source verified).

## A. Corrections
1. Section 13 table, row "MemForest / EventTree / MemTree": replace with three rows.
   - MemTree: arXiv 2410.14052 (ICLR 2025), dynamic tree memory for LLM conversations.
   - MemForest (hierarchical temporal indexing): arXiv 2605.23986, May 2026; MemTree is a component.
   - MemForest (EventTree partitioning and progressive merging): arXiv 2609.08273, NeurIPS 2026.
2. Appendix A: the two MemForest links are different papers. Label each by title.
3. Section 13, Mem0: add "ADD/UPDATE/DELETE is the 2025 paper design; current docs describe ADD-only extraction. Verify against current docs."
4. Section 13, Graphify: add "Repository owner is inconsistent across secondary sources; confirm the canonical GitHub URL."
5. Section 13, Supermemory/SMFS: add "Token-reduction figures (3x Claude, 1.75x Codex, 110 questions) are vendor-reported; xAFS definition not found."

## B. New text for Section 14 (Differentiation hypotheses), prepend
> Prior art already covers: hierarchical/tree agent memory (MemTree, MemForest), bi-temporal fact invalidation with preserved episodes (Graphiti/Zep), separation of evidence from beliefs (Hindsight), and extracted-vs-inferred edge labels (Graphify). These are not Atlas differentiators. The remaining candidate differentiator is **storage-level guarantees**: atomic episode+fact+provenance writes, snapshot-pinned logical reads, authorization during graph expansion without existence leaks, and crash recovery of the memory graph. Whether competing systems lack these is unverified (they may inherit them from Neo4j or PostgreSQL) and must be checked before any claim.

## C. New hypothesis rows (Section 14 table)
| Hypothesis | Potential value | Evidence required |
|---|---|---|
| Transactional memory writes | Episode, facts and provenance commit or fail together | Crash/fault-injection tests on the prototype; comparison with Graphiti on Neo4j and Hindsight on PostgreSQL |
| Leak-free authorized traversal | No existence/count/timing side channel across tenants | Adversarial leakage tests at every hop depth |
| Graph expansion may hurt precision | Justifies precision mode as default | A/B of vector-only vs graph-expanded retrieval on simple and multi-hop questions (one source reports noise from Mem0 graph on simple queries) |

## D. Section 12 addition (benchmarks)
Add: "Published memory-benchmark comparisons between vendors are contested (e.g. DMR uses short conversations that fit in modern context windows). Atlas must re-run baselines itself on LongMemEval, LoCoMo and MemoryAgentBench with identical models, prompts and budgets, and publish raw traces."

## E. Section 20 (open decisions), add
- Which of Graphiti and Hindsight is the primary baseline for the first workload comparison?
- Is the storage-guarantee gap (Section B) confirmed after reading their storage layers?
