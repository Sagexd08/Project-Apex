# The Prism manifesto

Written 2026-10-26. This is a statement of direction. Measurements are in `PAPER.md`, and anything not measured there is a belief, not a result.

## What we see

An agent does not work like a report. It reads what it knows, acts, writes what it learned, and reads again, often within a few milliseconds. The storage under most agents was designed for the opposite rhythm: load in bulk, build an index, then query for a long time. When the two meet, writes slow reads, the tail latency spikes, and a memory that should be instant becomes the part of the loop everyone waits for.

We also see that moving bytes is expensive. A search over a million vectors is mostly a question of how many bytes the machine must pull through memory. Arithmetic is cheap by comparison. A system that asks the question with fewer bytes wins on time, and it should also win on energy.

## What we believe

1. **Memory should be fast enough to disappear.** An agent should be able to consult everything it knows inside one thought, on a laptop, without a cluster.
2. **Nothing to train means nothing to drift.** A compressor learned from yesterday's data is wrong about tomorrow's. A fixed random rotation has no opinion about the data, so it cannot fall out of date.
3. **Pay for precision only when the question needs it.** Most of a corpus is clearly irrelevant to a given query. One bit is enough to say so. Spend more bits only on the few vectors still in the running.
4. **A reader never waits for a writer, and never sees half a write.** Every read happens against a named point in the log. The same question at the same point gives the same answer.
5. **Say what was measured.** Every number carries its data, its machine and the version of the code that produced it. A number from synthetic data says so. A claim without a measurement is a goal.
6. **Keep it open.** The reference implementation, the benchmark and the raw results are public, so anyone can disagree with us with evidence.

## What we are building

A storage layout, Prism, where each vector is stored once as nested bit planes, searched by a cascade that reads more planes only for the candidates still in contention, and served to readers through snapshots tied to the write-ahead log. It started as a question: could an agent memory be both fresh and fast, without a rebuild step between the two?

## North star

The best open-source vector database of this decade, built by one person with agents, for agents. We want model labs and model hubs to choose it because it is better, not because it is ours. That is a goal. We have no customers and we claim none. What we can do is write down the bar they would apply, and clear it in public:

1. **Speed on public data.** Recall against latency curves versus the leading engines on SIFT1M, GIST1M and a current text-embedding set, on identical hardware.
2. **Honest concurrency.** P99 under agent-rate writes, over hours, on a quiet machine.
3. **Durability.** Power-loss tests and an independent review before anyone is asked to trust it with memory.
4. **Isolation.** Tenant boundaries proved by tests that try to break them.
5. **Drop-in for agents.** An embedded Rust library, SDKs and an MCP server, so an agent can use it without glue.
6. **Reproducible.** Every number from a public script, with a checksum, that a stranger can re-run.

## What would make us wrong

- If the saving in bytes read does not become a saving in time on real hardware.
- If real embeddings are far less friendly than our synthetic clusters.
- If the empirical gate misses neighbours more often than its tuning suggests, and a provable gate costs too much to be worth it.
- If an existing system, such as RaBitQ or TurboQuant combined with a segment design, already gives the same result.

We will publish the result either way.

## What it is for

A person running an agent on their own machine, with their own data, who should not need to rent a cluster to give that agent a memory. A small team that wants retrieval it can reason about and replay. A research community that deserves an open, reproducible reference for the read-while-writing case, not just for the static one.

The goal is modest in scope and large in reach: make good memory cheap enough to be everywhere.
