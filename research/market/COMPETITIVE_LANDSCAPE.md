# Atlas: Competitive and Academic Landscape (Stage B, first pass)

Date: 2026-10-30. Status: **first pass from search results, not primary-source verified.**
Evidence class per row: P = peer-reviewed/arXiv paper, V = vendor claim, S = secondary summary, R = reproduced by us (none yet).
Every row must be re-checked against the primary paper or repository before it is used in a public claim.

## 1. Systems reviewed

| System | What it preserves as evidence | What is derived | Temporal model | Retrieval | Class |
|---|---|---|---|---|---|
| Graphiti / Zep (arXiv 2501.13956) | Episodes (raw message/text/JSON, non-lossy, with a reference timestamp) | Entity nodes, entity edges, community nodes | Bi-temporal edges: `valid_at`, `invalid_at`, `created_at`, `expired_at`. Contradicted edges are invalidated, not deleted | Hybrid: semantic + keyword + graph | P + S |
| Mem0 (arXiv 2504.19413) | Extracted memories; original turns are not the primary store | LLM-chosen ADD / UPDATE / DELETE / NOOP; graph variant adds a conflict detector and update resolver | No bi-temporal model reported in the paper. Current docs say they moved to ADD-only extraction | Vector, plus graph variant | P + V |
| Supermemory / SMFS | Raw documents, mounted as a local folder | Extracted memories from files in "memory paths" | Not established from sources read | Hybrid search; filesystem interface | V only |
| Graphify (Graphify-Labs) | Source code files | tree-sitter AST graph, no LLM for the code pass. Edges tagged EXTRACTED / INFERRED / AMBIGUOUS; semantic pass for docs uses a model | Rebuilt per commit via git hook; no history model reported | Query / path / explain over `graph.json` | V + S |
| Hindsight (arXiv 2512.12818, ACL 2026 demo) | World and experience networks kept distinct from synthesized observation and opinion networks | Opinions carry confidence that updates with evidence | Temporal filtering in recall | Parallel vector + keyword + graph + temporal; PostgreSQL + pgvector; MIT | P |
| MemTree (arXiv 2410.14052, ICLR 2025) | Conversation content in a dynamic tree | Hierarchical schema summaries | Not the focus | Tree traversal | P |
| MemForest, two different papers | See below | See below | Time-ordered trees (2605.23986) | Hierarchical temporal index (2605.23986); anchor-guided retrieval (2609.08273) | P |

### Naming correction for the PRD
"MemForest" is used by **two** distinct papers: arXiv 2605.23986 (hierarchical temporal indexing; MemTree is a component) and arXiv 2609.08273 (EventTree partitioning and progressive merging, NeurIPS 2026). The standalone MemTree is arXiv 2410.14052. The PRD's phrase "MemForest / EventTree / MemTree" conflates them. Cite each by arXiv id.

## 2. Findings that change the Atlas plan

1. **Tree-shaped agent memory is prior art.** MemTree (2024), MemForest (2026) and Hindsight's separated networks all exist. "Hierarchy plus graph for agent memory" is not novel by itself. The Yggdrasil framing is branding, not a research claim.
2. **Bi-temporal edges with invalidation are prior art in Graphiti.** Atlas matching it is table stakes. A differentiator would have to be something Graphiti does not do, for example transactional atomicity of episode + facts + provenance, or snapshot-pinned reads.
3. **Evidence versus derived knowledge is already separated by Graphiti (episodes) and Hindsight (networks).** Graphify separates extracted from inferred edges. Atlas's open question is enforcement at the storage layer, not the concept.
4. **Mem0 changed design after its paper.** Per the directive supplied 2026-10-30 (Mem0 V3 docs, not yet opened by me): the *add path* is an asynchronous, single-pass, ADD-only extraction pipeline, but separate update/delete APIs still exist. My earlier wording ("ADD/UPDATE/DELETE superseded") overstated this. Describe the 2025 paper, the current open-source code and the managed platform separately.
5. **Benchmark quality is contested.** Secondary sources question Zep's DMR benchmark (short conversations) and a disputed Zep-vs-Mem0 comparison. SMFS: the directive's reading of the technical report (not yet opened by me) is that xAFS is a benchmark name for 110 audited questions over 13 synthetic corpora, 440 trials per condition, with 53.8% fewer aggregate tokens and a 1.4 percentage-point lower aggregate pass rate versus a filesystem-native baseline, a larger pass-rate drop on its multi-hop subset, and a synthetic, system-authored corpus. The "3x" figure in the search snippet is a per-agent presentation, not the aggregate. Do not expand the xAFS acronym.
5a. **Dependency status.** Kuzu's repository was reported archived in Oct 2025 and Graphiti's README marks Kuzu deprecated (directive, unverified by me). Graphiti's supported backends include Neo4j, FalkorDB and Amazon Neptune. Canonical Graphify repo per the directive: `Graphify-Labs/graphify`; treat the personal-account repo as unverified fork/predecessor.
5b. **Guarantee layers.** "Not documented" is "unknown". Neo4j documents ACID transactions and Apache AGE builds on PostgreSQL's transactional layer, so Graphiti may inherit atomicity for the writes it groups into one transaction. Whether Graphiti groups an episode's entity, edge and provenance writes into one backend transaction must be read in its source before any gap is claimed.
6. **Graph retrieval can hurt.** One secondary source says Mem0's graph variant added noise on simple queries. This supports the PRD's bounded-expansion and precision-mode design, and is a hypothesis to test, not a result.

## 3. Open gap worth testing (hypothesis, not a claim)

None of the sources read describe storage-level guarantees for: (a) atomic write of episode + derived facts + provenance, (b) snapshot-pinned logical reads across concurrent agents, (c) authorization applied during graph expansion with no existence leak, (d) crash recovery of the memory graph. Those are database properties. Whether the listed systems lack them is **unverified**: Graphiti runs on Neo4j/other graph stores and Hindsight on PostgreSQL, so they may inherit such guarantees from the store. Check before claiming a gap.

## 4. Not yet researched (still required by Stage B)

A-Mem, Letta, Milvus, Qdrant, Weaviate, Pinecone, Chroma, LanceDB, pgvector, Neo4j, Kuzu, Apache AGE; benchmarks LongMemEval, LoCoMo, MemoryAgentBench, VIBE, VectorDBBench; ACORN, RaBitQ, TurboQuant; licenses of every dependency.

## Sources
- Zep / Graphiti: https://blog.getzep.com/beyond-static-knowledge-graphs/ , https://pypi.org/project/graphiti-core/0.6.1/ , https://lin-guanguo.github.io/llm-memory-research/graphiti.research/ (secondary)
- Mem0: https://docs.mem0.ai/core-concepts/memory-evaluation.md , https://www.emergentmind.com/papers/2504.19413 (secondary)
- Supermemory: https://supermemory.ai/research.md , https://supermemory.ai/docs/smfs/overview.md
- Graphify: https://github.com/Graphify-Labs/Graphify , https://graphify.com/what-is-graphify , https://betterstack.com/community/guides/ai/ai-development/graphify-codebase/
- Hindsight: https://arxiv.org/abs/2512.12818 , https://aclanthology.org/2026.acl-demo.27/
- MemTree: https://arxiv.org/abs/2410.14052
- MemForest: https://www.alphaxiv.org/abs/2605.23986 , https://arxiv.org/pdf/2609.08273 , https://neurips.cc/virtual/2026/poster/148407
