// Builds the argument pages: manifesto, abstract, paper (arXiv layout), film, lab, evidence, and the prism.html redirect.
// Run: node tools/build-site.mjs   (then node tools/build-lab.mjs for the method/honesty pages)
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { esc, head, header, footer, REPO } from './shell.mjs';

const SITE = join(dirname(fileURLToPath(import.meta.url)), '..');
const DOCS = join(SITE, '..', 'docs', 'prism');
const read = (p) => readFileSync(p, 'utf8');
const doc = (n) => read(join(DOCS, n));
const frag = (n) => read(join(SITE, 'src', n));
const write = (slug, html) => writeFileSync(join(SITE, `${slug}.html`), html);
const scripts = (...s) => ['site', ...s].map((x) => `<script src="js/${x}.js" defer></script>`).join('\n');
const crumb = (t) => `<p class="crumb"><a href="index.html">Home</a><span>/</span><a href="manifesto.html">Manifesto</a><span>/</span><span>${esc(t)}</span></p>`;
const page = (cur, title, desc, css, bodyCls, body, js) =>
  `${head({ title, desc, extraCss: css })}\n<body class="gxp ${bodyCls}">\n<a class="skip" href="#main">Skip to content</a>\n${header(cur)}\n${body}\n${footer()}\n${scripts(...js)}\n</body>\n</html>\n`;

/* ---------- tiny markdown -> HTML (headings, tables, lists, code, inline) ---------- */
const slug = (s) => s.toLowerCase().replace(/<[^>]+>/g, '').replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
function inline(s) {
  const codes = [];
  s = s.replace(/`([^`]+)`/g, (_, c) => { codes.push(c); return `\u0000${codes.length - 1}\u0000`; });
  s = esc(s)
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/(^|[^*\w])\*([^*\n]+)\*(?!\w)/g, '$1<em>$2</em>')
    .replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g, '<a href="$2" rel="noopener">$1</a>');
  return s.replace(/\u0000(\d+)\u0000/g, (_, i) => `<code>${esc(codes[+i])}</code>`);
}
function md(src, { demote = 0, idPrefix = '' } = {}) {
  const L = src.split(/\r?\n/), out = [], heads = [];
  let i = 0;
  while (i < L.length) {
    const l = L[i];
    if (/^```/.test(l)) { const b = []; i++; while (i < L.length && !/^```/.test(L[i])) b.push(L[i++]); i++; out.push(`<pre><code>${esc(b.join('\n'))}</code></pre>`); continue; }
    const h = /^(#{1,4}) (.*)$/.exec(l);
    if (h) {
      const lvl = Math.min(4, h[1].length + demote), id = idPrefix + slug(h[2]);
      if (h[1].length > 1 || demote) { out.push(`<h${lvl} id="${id}">${inline(h[2])}</h${lvl}>`); if (lvl === 2) heads.push({ id, t: h[2] }); }
      i++; continue;
    }
    if (/^\|/.test(l) && /^\|[\s:|-]+\|?\s*$/.test(L[i + 1] || '')) {
      const cells = (r) => r.replace(/^\||\|\s*$/g, '').split('|').map((c) => c.trim());
      const th = cells(l); i += 2; const rows = [];
      while (i < L.length && /^\|/.test(L[i])) rows.push(cells(L[i++]));
      out.push(`<table><thead><tr>${th.map((c) => `<th>${inline(c)}</th>`).join('')}</tr></thead><tbody>${rows.map((r) => `<tr>${r.map((c) => `<td>${inline(c)}</td>`).join('')}</tr>`).join('')}</tbody></table>`);
      continue;
    }
    const li = /^(\s*)([-*]|\d+\.) (.*)$/.exec(l);
    if (li) {
      const ord = /\d/.test(li[2]), items = [];
      while (i < L.length && /^(\s*)([-*]|\d+\.) /.test(L[i])) { items.push(L[i].replace(/^\s*([-*]|\d+\.) /, '')); i++; }
      out.push(`<${ord ? 'ol' : 'ul'}>${items.map((t) => `<li>${inline(t)}</li>`).join('')}</${ord ? 'ol' : 'ul'}>`);
      continue;
    }
    if (!l.trim()) { i++; continue; }
    const p = [l]; i++;
    while (i < L.length && L[i].trim() && !/^(#{1,4} |```|\||\s*([-*]|\d+\.) )/.test(L[i])) p.push(L[i++]);
    out.push(`<p>${inline(p.join(' '))}</p>`);
  }
  return { html: out.join('\n'), heads };
}

/* ---------- manifesto ---------- */
const TENETS = [
  ['Memory should be fast enough to disappear.', 'An agent should consult everything it knows inside one thought, on a laptop, without a cluster.'],
  ['Nothing to train means no trained state to drift.', 'A compressor learned from yesterday’s data can be wrong about tomorrow’s. A fixed random rotation has no opinion about the data, and the error scale we gate on is measured and rechecked on shifted data.'],
  ['Pay for precision only when the question needs it.', 'Most of a corpus is clearly irrelevant to a given query, and one bit is enough to say so. Spend more bits only on the few vectors still in the running.'],
  ['A reader takes no lock and never sees half a write.', 'Every read happens against a named point in the log: the same question at the same point gives the same answer. Readers still share CPU and memory with writers, and the paper reports that cost.'],
  ['Say what was measured.', 'Every number carries its data, its machine and the version of the code that produced it. A claim without a measurement is a goal.'],
  ['Keep it open.', 'The reference implementation, the benchmark and the raw results are public, so anyone can disagree with us with evidence.'],
];
const BARS = [
  ['Speed on public data', 'Recall against latency curves versus other engines on SIFT1M, GIST1M and a current text-embedding set, on identical hardware.', 'Unmeasured'],
  ['Honest concurrency', 'P99 under agent-rate writes, over hours, on a quiet machine.', 'Open'],
  ['Durability', 'Power-loss tests and an independent review before anyone is asked to trust it with memory.', 'Open'],
  ['Isolation', 'Tenant boundaries proved by tests that try to break them.', 'Tested (builder-reported)'],
  ['Drop-in for agents', 'An embedded Rust library, SDKs and an MCP server. A Rust engine, HTTP API and demo exist; SDKs and MCP do not.', 'Open'],
  ['Reproducible', 'Every number from a public script, with a checksum, that a stranger can re-run.', 'Open'],
];
const FALSIFIERS = [
  'The saving in bytes read does not become a saving in time on real hardware.',
  'Real embeddings are far less friendly than our synthetic clusters.',
  'The empirical gate misses neighbours more often than its tuning suggests, and a provable gate costs too much to be worth it.',
  'An existing system, such as RaBitQ or TurboQuant combined with a segment design, already gives the same result.',
];
const CARDS = [['algorithm', 'The algorithm', 'Encode once, read progressively, rerank exactly.'], ['mathematics', 'The mathematics', 'The error formula, the gate lemma and its open problem.'], ['snapshot-protocol', 'The snapshot protocol', 'What it means for a reader to pin the log.'], ['paper', 'The research paper', 'Full text in arXiv layout, every table, save as PDF.'], ['evidence', 'The evidence', 'Every number, loaded from the raw benchmark results.'], ['film', 'The film', 'The mathematics in ninety seconds.']];
const SUB = 'max-width:64ch;margin-top:22px;font-size:17px;line-height:1.55';

const manifesto = () => page('manifesto', 'Manifesto', 'A statement of direction for an open-source vector database for agent memory: what we believe, what we are building, and what would prove us wrong.', [], 'pg-manifesto', `
<main id="main">
<header class="pg-hero">
  <p class="crumb"><a href="index.html">Home</a><span>/</span><span>Manifesto</span></p>
  <h1>Memory fast enough to disappear.</h1>
  <p class="lead">An open-source vector database for agents that read, act, write and read again within milliseconds. This is our statement of direction: what we believe, what we are building, and exactly what would prove us wrong.</p>
  <div class="badges"><span class="chipx hot">Statement of direction</span><span class="chipx">Research Note v1.0</span><span class="chipx">Preprint, not peer reviewed</span></div>
  <div class="mf-meta"><div><b>1.06</b>bits read per coordinate, z = 3</div><div><b>1,000 / 1,000</b>top-10 matches on one synthetic set</div><div><b>0</b>speed claims we can support yet</div></div>
  <aside class="hero-card glass" aria-label="Status at a glance"><i class="hc-grip" aria-hidden="true"></i>
    <p class="hc-loc">Research Note v1.0 &middot; synthetic data, one machine</p>
    <div class="hc-row"><div class="hc-h"><b>Bits read</b><span>measured</span></div><div class="hc-v"><s>z = 3</s><em>1.06</em><s>per coordinate</s></div></div>
    <div class="hc-row"><div class="hc-h"><b>Time saving</b><span>not shown</span></div><div class="hc-v"><s>0.61 ms</s><em>vs</em><s>0.55 ms exact</s></div></div></aside>
  <ul class="hero-strip" aria-label="What this page covers"><li><b>Measured</b>bits read, recall, error per bit</li><li><b>Proposed</b>nested planes, log-pinned snapshots</li><li><b>Unproven</b>a time saving over an exact scan</li><li><b>Open</b>real embeddings, durability, SDKs</li><li><b>Falsifiable</b>four ways we could be wrong</li></ul>
  <a class="mf-cue" href="#declaration" aria-label="Scroll to the declaration"><i></i>Read</a>
</header>
<section class="mf-decl" id="declaration"><div class="wrapx">
  <span class="lblx reveal-in">What we see</span>
  <p class="big reveal-in">An agent does not work like a report. <em>It reads what it knows, acts, writes what it learned, and reads again.</em></p>
  <p class="reveal-in" style="${SUB};margin-top:34px;color:rgba(255,255,255,.78);font-size:18px">The storage under most agents was designed for the opposite rhythm: load in bulk, build an index, then query for a long time. When the two meet, writes slow reads, tail latency spikes, and a memory that should be instant becomes the part of the loop everyone waits for. We also see that moving bytes is expensive. <strong>We have not shown that fewer bytes means less time</strong>; see the falsifiers below.</p>
</div></section>
<section class="sx" id="believe"><div class="wrapx">
  <span class="lblx">What we believe</span><h2 class="reveal-in">Six commitments.</h2>
  <div class="mf-tenets">${TENETS.map(([t, p], i) => `<article class="tenet glass reveal-in"><b class="n">${String(i + 1).padStart(2, '0')}</b><h3>${esc(t)}</h3><p>${esc(p)}</p></article>`).join('')}</div>
</div></section>
<section class="sx" id="building"><div class="wrapx">
  <span class="lblx">What we are building</span><h2 class="reveal-in">One stored code. Progressive precision. A snapshot per reader.</h2>
  <p class="reveal-in" style="${SUB};font-size:18px;color:rgba(255,255,255,.86)">Prism: each vector is stored once as nested bit planes, searched by a cascade that reads more planes only for candidates still in contention, and served to readers through snapshots tied to the write-ahead log.</p>
  <div class="mf-grid">${CARDS.map(([s, t, d], i) => `<a class="mf-card glass reveal-in" href="${s}.html"><b>${String(i + 1).padStart(2, '0')}</b><h3>${t}</h3><p>${d}</p><span>Open page &rarr;</span></a>`).join('')}</div>
</div></section>
<section class="sx" id="north"><div class="wrapx">
  <span class="lblx">North star</span><h2 class="reveal-in">The bar a reader would apply.</h2>
  <p class="reveal-in" style="${SUB};color:rgba(255,255,255,.78)">A proposed goal, not a claim: an open-source vector database that agents can use as memory. We have no customers and no comparison with any other system. What we can do is write down the bar and try to clear it in public.</p>
  <div style="overflow-x:auto"><table class="bars"><thead><tr><th>Bar</th><th>What it requires</th><th>Status</th></tr></thead><tbody>${BARS.map(([a, b, s]) => `<tr><td>${esc(a)}</td><td>${esc(b)}</td><td><span class="chipx">${esc(s)}</span></td></tr>`).join('')}</tbody></table></div>
</div></section>
<section class="sx" id="falsifiers"><div class="wrapx">
  <span class="lblx">Thesis and falsifiers</span><h2 class="reveal-in">What would prove us wrong.</h2>
  <p class="reveal-in" style="${SUB};color:rgba(255,255,255,.86)"><strong>Thesis (proposed).</strong> Nested bit planes plus log-pinned snapshots can improve the memory, recall and latency trade-off when a store is read while it is written. So far one synthetic benchmark shows bits read falling to about 1.06 per coordinate, and <strong>no established time saving</strong> over an optimised exact scan (inconclusive: the sign reversed between two runs). We will publish the result either way. A negative result is acceptable. An unsupported positive claim is not.</p>
  <div class="falsify">${FALSIFIERS.map((f) => `<div class="glass reveal-in">${esc(f)}</div>`).join('')}</div>
</div></section>
<section class="sx" id="open"><div class="wrapx">
  <span class="lblx">Open by construction</span><h2 class="reveal-in">Free to use, and free to inspect.</h2>
  <p class="reveal-in" style="${SUB};color:rgba(255,255,255,.8)">The core database must not need a paid API, a hosted service, telemetry, or a language model to store, search and recover vectors. The Rust workspace declares Apache-2.0; a top-level licence file, third-party notices and an SBOM are still open. Dependencies are checked with cargo-deny on every verification run. Every number comes from synthetic data with fixed seeds and committed raw results.</p>
  <p class="reveal-in" style="margin-top:34px;display:flex;gap:12px;flex-wrap:wrap"><a class="gx-btn" href="paper.html">Read the paper</a><a class="gx-btn ghost" href="${REPO}">Inspect the source</a><a class="gx-btn ghost" href="evidence.html">View the experiments</a></p>
</div></section>
</main>`, []);


const playgroundPage = () => page('playground', 'Playground', 'Try Bat DB agent memory in your browser: remember, recall, change a decision, time-travel and check tenant isolation.', ['playground'], 'pg-playground', `
<main id="main" class="pgx">
<h1>Give an agent a memory. Watch it use it.</h1>
<p class="lead">Five steps: write decisions, ask a question, change a decision, ask what you believed before, and prove another tenant sees nothing. Click any call to see its request and response.</p>
<p class="pgx-note"><b>This page is a JavaScript simulation</b> of the memory API with a toy 64-dimension embedder, so you can try the idea with nothing installed. It is not the Rust engine and its latencies mean nothing. The real engine serves a live playground at <b>/playground</b> when you run <b>atlas-cli</b>.</p>
<div class="pgx-cap-wrap"><p id="pgx-cap" role="status" aria-live="polite">Press <b>Play 40s demo</b> for a narrated walk-through, or drive it yourself.</p><div class="pgx-progress"><i id="pgx-prog"></i></div><span id="pgx-time"></span></div>
<p id="pgx-stats" class="pgx-dim"></p>
<div class="pgx-grid">
<section class="pgx-col" aria-label="Guided steps"><h2 class="pgx-h">Story</h2><ol id="pgx-steps"></ol>
<div class="pgx-run"><button class="pgx-btn" id="pgx-play" type="button">Play 40s demo</button></div><div class="pgx-run"><button class="pgx-btn ghost" id="pgx-all" type="button">Run steps</button><button class="pgx-btn ghost" id="pgx-reset" type="button">Reset</button></div>
<div class="pgx-ten" role="radiogroup" aria-label="Tenant"><label><input type="radio" name="pgx-tenant" value="team-a" checked>team-a</label><label><input type="radio" name="pgx-tenant" value="team-b">team-b</label></div></section>
<section class="pgx-col" aria-label="Transcript"><h2 class="pgx-h">Transcript</h2><div id="pgx-feed" aria-live="polite"><div class="pgx-empty" id="pgx-empty">Press a step on the left, or run the whole story.</div></div>
<form id="pgx-ask"><input id="pgx-q" type="text" value="why did we choose group commit?" aria-label="Ask a question"><button class="pgx-btn" type="submit">Recall</button></form>
<div class="pgx-chips"><button type="button" class="pgx-chip">why did we choose group commit?</button><button type="button" class="pgx-chip">what happens on kill -9?</button><button type="button" class="pgx-chip">how fast is restart?</button><button type="button" class="pgx-chip">who can read this?</button></div>
<h2 class="pgx-h" style="margin-top:18px">Add a memory</h2><div class="pgx-form"><input type="hidden" id="pgx-sup"><input id="pgx-title" type="text" placeholder="Title, e.g. Use mmap checkpoints" aria-label="Memory title"><textarea id="pgx-body" rows="2" placeholder="Detail the agent should recall" aria-label="Memory body"></textarea><button class="pgx-btn" id="pgx-save" type="button">Save memory</button></div></section>
<section class="pgx-col" aria-label="Memory graph"><h2 class="pgx-h">Memory graph</h2><svg id="pgx-graph" viewBox="0 0 380 300" role="img" aria-label="Memory graph"></svg>
<div class="pgx-slider"><input id="pgx-ver" type="range" min="0" max="0" value="0" disabled aria-label="Log version"><span id="pgx-verl">now</span></div>
<h2 class="pgx-h">Inspector</h2><div id="pgx-insp"><p class="pgx-dim">Select a call, or click a node in the graph.</p></div><div class="pgx-run" style="margin-top:8px"><button class="pgx-btn ghost" id="pgx-copy" type="button">Copy call as Python</button></div></section>
</div></main>`, ['playground']);

/* ---------- abstract ---------- */
function abstractPage() {
  const raw = doc('ABSTRACT.md').split(/\r?\n/).filter((l) => l.trim() && !l.startsWith('#'));
  const status = raw[0], body = raw.slice(1).join(' ');
  const sentences = body.split(/(?<=\.)\s+(?=[A-Z])/);
  const third = Math.ceil(sentences.length / 3);
  const paras = [0, 1, 2].map((k) => sentences.slice(k * third, (k + 1) * third).join(' ')).filter(Boolean);
  return page('abstract', 'Abstract', 'The whole Prism research note in one page.', [], 'pg-abstract', `
<main id="main">
<header class="pg-hero">${crumb('Abstract')}<h1>The whole note, in one page.</h1>
  <p class="lead">${inline(status)}</p>
  <div class="badges"><span class="chipx hot">Preprint</span><span class="chipx">Synthetic data</span><span class="chipx">Unreproduced</span></div></header>
<section class="sx"><div class="wrapx abs-wrap">
  <article class="abs-card glass"><div id="abs-text">${paras.map((p) => `<p>${inline(p)}</p>`).join('')}</div>
    <p style="margin-top:28px;display:flex;gap:10px;flex-wrap:wrap"><button class="gx-btn" type="button" data-copy="#abs-text">Copy abstract</button><a class="gx-btn ghost" href="paper.html">Read the full paper</a></p></article>
  <aside class="abs-rail" aria-label="Headline numbers">
    <div class="fact glass"><b>1.06</b><span>bits read per coordinate per vector at z = 3, against 4 for a flat 4-bit scan (measured, synthetic).</span></div>
    <div class="fact glass"><b>0.998</b><span>recall@10 at z = 2. At z = 3 all 1,000 queries matched exact search: an observation, not a guarantee.</span></div>
    <div class="fact glass"><b>+21%</b><span>median P99 latency with a concurrent writer over seven alternating trials (range 12% to 34%).</span></div>
    <div class="fact glass warn"><b>Unproven</b><span>The saving in bits read was not shown to become a saving in time. The sign reversed between runs.</span></div>
  </aside></div></section>
</main>`, []);
}

/* ---------- paper (arXiv layout) ---------- */
function paperPage() {
  const main = doc('PAPER.md');
  const titleLine = /^# (.*)$/m.exec(main)[1];
  const startBody = main.indexOf('\n## 1.');
  const intro = main.slice(0, main.indexOf('\n## Abstract')).split(/\r?\n/).slice(2).join('\n');
  const body = md(main.slice(startBody));
  const absRaw = doc('ABSTRACT.md').split(/\r?\n/).filter((l) => l.trim() && !l.startsWith('#')).slice(1).join(' ');
  const APPX = [['A', 'Mathematics and algorithm', 'MATH_AND_ALGORITHM.md'], ['B', 'Certified interval cascade', 'CERTIFIED_CASCADE.md'], ['C', 'Related work and claim audit', 'RELATED_WORK_AND_CLAIM_AUDIT.md'], ['D', 'Reproduction of the synthetic pilot', 'STAGE_D_REPRODUCTION.md'], ['E', 'Snapshot protocol', 'SNAPSHOT_PROTOCOL.md']];
  const appx = APPX.map(([k, t, f]) => {
    const r = md(doc(f).replace(/^# .*\n/, ''), { demote: 1, idPrefix: `app-${k.toLowerCase()}-` });
    return { k, t, html: `<section class="part" id="appendix-${k.toLowerCase()}"><p class="part-t">Appendix ${k}</p><h2>${esc(t)}</h2>${r.html.replace(/<h2 /g, '<h3 ').replace(/<\/h2>/g, '</h3>')}</section>` };
  });
  const toc = [...body.heads.map((h) => `<a href="#${h.id}">${esc(h.t)}</a>`), ...appx.map((a) => `<a href="#appendix-${a.k.toLowerCase()}">Appendix ${a.k}. ${esc(a.t)}</a>`)].join('');
  return page('paper', 'Research paper', 'Prism: nested bit planes, an error-gated cascade and log-pinned snapshots. Full text in arXiv layout, with every table and appendix.', [], 'pg-paper', `
<main id="main">
<div class="pp-bar"><span>Research Note v1.0 &middot; preprint</span><span class="sp"></span><a class="pb alt" href="${REPO}/blob/main/docs/prism/PAPER.md" rel="noopener">Markdown source</a><button type="button" data-print>Save as PDF</button></div>
<div class="pp-shell"><nav class="pp-toc" aria-label="Paper sections"><p>Contents</p>${toc}</nav>
<article class="sheet">
  <div class="arx-id"><span><b>Bat DB</b> &middot; Research Note v1.0 &middot; [cs.DB &middot; cs.IR]</span><span>Preprint, not peer reviewed &middot; not submitted to arXiv &middot; 2026-10-26</span></div>
  <h1>${esc(titleLine)}</h1>
  <p class="byline">Bat DB</p><p class="affil">No authors or affiliations are claimed beyond the project</p>
  <div class="abstract-box"><b class="t">Abstract</b><p>${inline(absRaw)}</p></div>
  ${md(intro).html}
  ${body.html}
  ${appx.map((a) => a.html).join('\n')}
</article></div>
</main>`, []);
}

/* ---------- film, lab, evidence (reuse the interactive fragments) ---------- */
const inner = (f) => /<div class="wrap">([\s\S]*)<\/div>\s*<\/section>\s*$/.exec(frag(f))?.[1] ?? frag(f);
const filmPage = () => page('film', 'Film', 'The Prism mathematics as a ninety-second film: nine scenes you can pause, scrub and replay.', ['prism'], 'pg-film', `
<main id="main"><section class="theatre"><div class="wrapx">${crumb('Film')}<h1>The mathematics, as a film.</h1>
<div class="film-host">${inner('_film.html').replace(/<p class="mono lbl">Film<\/p>\s*<h2[^>]*>[^<]*<\/h2>/, '')}</div></div></section></main>`, ['prism-film']);
const labPage = () => page('lab', 'Interactive lab', 'Move the sliders: rotation, the error-gated cascade and log-pinned snapshots, running in your browser.', ['prism'], 'pg-lab', `
<main id="main"><header class="pg-hero" style="min-height:52svh">${crumb('Interactive lab')}<h1>Try the three ideas.</h1><p class="lead">Everything here runs in your browser on synthetic data. Nothing is sent anywhere.</p></header>${frag('_lab.html')}</main>`, ['prism']);
const evidencePage = () => page('evidence', 'Evidence', 'Every Prism measurement, loaded from the raw benchmark results, with the weak points and what we did about them.', ['prism'], 'pg-evidence', `
<main id="main"><header class="pg-hero" style="min-height:58svh">${crumb('Evidence')}<h1>What was measured.</h1><p class="lead">Loaded from <code>data/prism-results.json</code>, written by the Rust benchmark in <code>benchmarks/prism</code>. Synthetic data, one machine, not reproduced by anyone else.</p></header>
<section class="sx"><div class="wrapx">${frag('_kpis.html').replace(/<section[^>]*>|<\/section>/g, '')}</div></section>
${frag('_evidence.html')}
${frag('_h2h.html')}
${frag('_limits.html')}</main>`, ['prism', 'prism-evidence']);

/* ---------- prism.html: redirect stub so old links keep working ---------- */
write('prism', `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta http-equiv="refresh" content="0;url=manifesto.html"><link rel="canonical" href="manifesto.html"><title>Moved | Bat DB</title></head><body><p>The Prism pages moved under the <a href="manifesto.html">Manifesto</a>.</p></body></html>\n`);
write('manifesto', manifesto());
write('abstract', abstractPage());
write('paper', paperPage());
write('film', filmPage());
write('lab', labPage());
write('evidence', evidencePage());
write('playground', playgroundPage());
console.log('manifesto abstract paper film lab evidence prism(redirect)');
