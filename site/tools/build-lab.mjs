import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const SITE = join(dirname(fileURLToPath(import.meta.url)), '..');
const REPO = 'https://github.com/Sagexd08/Project-Apex';

const FIGS = {
  planes: `<svg viewBox="0 0 400 340" role="img" aria-label="Four stacked bit planes"><g>
    <path class="fa ac" d="M80 40H340L380 90H40Z"/><path class="fa" d="M80 100H340L380 150H40Z"/>
    <path class="fa dim" d="M80 160H340L380 210H40Z"/><path class="fa dim" d="M80 220H340L380 270H40Z"/>
    ${[0, 1, 2, 3, 4, 5, 6, 7, 8, 9].map((i) => `<circle class="ff" cx="${96 + i * 24}" cy="66" r="2.4"/>`).join('')}
    <text class="ft" x="40" y="302">plane 1 sign bit .. plane 4 finest cell</text></g></svg>`,
  graph: `<svg viewBox="0 0 400 340" role="img" aria-label="Related research families around Prism"><g class="fa dim">
    <path d="M200 170L90 70M200 170L310 70M200 170L60 190M200 170L340 190M200 170L120 290M200 170L290 290"/></g>
    ${[['PQ', 90, 70], ['RaBitQ', 310, 70], ['Turbo', 60, 190], ['ScaNN', 340, 190], ['Fresh', 120, 290], ['ACORN', 290, 290]].map(([t, x, y]) => `<circle class="fa" cx="${x}" cy="${y}" r="26"/><text class="ft" x="${x}" y="${y + 3}" text-anchor="middle">${t}</text>`).join('')}
    <circle class="fa ac" cx="200" cy="170" r="34" stroke-dasharray="4 4"/><text class="ft" x="200" y="174" text-anchor="middle" style="fill:var(--accent)">Prism ?</text></svg>`,
  funnel: `<svg viewBox="0 0 400 340" role="img" aria-label="Cascade: fewer candidates at each stage"><g>
    <rect class="fa ac" x="50" y="50" width="300" height="34" rx="4"/><rect class="fa" x="110" y="110" width="180" height="34" rx="4"/>
    <rect class="fa" x="150" y="170" width="100" height="34" rx="4"/><rect class="fa dim" x="180" y="230" width="40" height="34" rx="4" stroke-dasharray="4 3"/>
    <text class="ft" x="200" y="71" text-anchor="middle">1 bit · all n</text><text class="ft" x="200" y="131" text-anchor="middle">2 bits · survivors</text>
    <text class="ft" x="200" y="191" text-anchor="middle">4 bits</text><text class="ft" x="200" y="251" text-anchor="middle">rerank</text>
    <text class="ft" x="200" y="300" text-anchor="middle">widths are schematic, not measured</text></g></svg>`,
  bell: `<svg viewBox="0 0 400 340" role="img" aria-label="Error distribution with plus and minus z sigma marks"><g>
    <path class="fa" d="M20 270C100 270 140 60 200 60S300 270 380 270"/><path class="fa dim" d="M20 270H380"/>
    <path class="fa ac" d="M130 270V150M270 270V150" stroke-dasharray="4 4"/>
    <text class="ft" x="130" y="292" text-anchor="middle">-z σ</text><text class="ft" x="270" y="292" text-anchor="middle">+z σ</text>
    <text class="ft" x="200" y="40" text-anchor="middle">score error ε</text></g></svg>`,
  grid: `<svg viewBox="0 0 400 340" role="img" aria-label="Seven candidates A to G, no winner"><g>
    ${['A', 'B', 'C', 'D', 'E', 'F', 'G'].map((t, i) => { const x = 40 + (i % 4) * 88, y = 80 + Math.floor(i / 4) * 100; return `<rect class="fa ${t === 'C' ? 'ac' : 'dim'}" x="${x}" y="${y}" width="72" height="72" rx="8"/><text class="ft" x="${x + 36}" y="${y + 40}" text-anchor="middle" style="font-size:16px">${t}</text>`; }).join('')}
    <text class="ft" x="40" y="300">no winner declared</text></g></svg>`,
  timeline: `<svg viewBox="0 0 400 340" role="img" aria-label="Log with writer head and two reader pins"><g>
    <path class="fa dim" d="M30 150H370"/>${[0, 1, 2, 3, 4, 5, 6, 7].map((i) => `<path class="fa" d="M${50 + i * 44} 142V158"/><text class="ft" x="${50 + i * 44}" y="178" text-anchor="middle">${i + 1}</text>`).join('')}
    <path class="fa ac" d="M138 100V140M138 100l-6 8M138 100l6 8"/><text class="ft" x="138" y="90" text-anchor="middle">reader A</text>
    <path class="fa ac" d="M270 100V140M270 100l-6 8M270 100l6 8"/><text class="ft" x="270" y="90" text-anchor="middle">reader B</text>
    <rect class="ff" x="350" y="136" width="14" height="28" rx="3"/><text class="ft" x="357" y="210" text-anchor="middle">writer</text>
    <rect class="fa dim" x="50" y="240" width="130" height="30" rx="4"/><rect class="fa dim" x="186" y="240" width="130" height="30" rx="4"/><rect class="fa ac" x="322" y="240" width="48" height="30" rx="4"/>
    <text class="ft" x="30" y="296">sealed segments, immutable</text></g></svg>`,
  audit: `<svg viewBox="0 0 400 340" role="img" aria-label="Audit grid with open items"><g>
    ${Array.from({ length: 24 }, (_, i) => { const x = 50 + (i % 6) * 50, y = 60 + Math.floor(i / 6) * 56; const open = [1, 4, 8, 11, 13, 14, 19, 22].includes(i); return `<rect class="${open ? 'ff' : 'fa dim'}" x="${x}" y="${y}" width="34" height="34" rx="6" ${open ? 'opacity=".85"' : ''}/>`; }).join('')}
    <text class="ft" x="50" y="310">filled = open item</text></g></svg>`,
  steps: `<svg viewBox="0 0 400 340" role="img" aria-label="Staircase of milestones"><g>
    <path class="fa" d="M30 290H110V230H190V170H270V110H350V50"/>
    <circle class="ff" cx="70" cy="290" r="6"/><circle class="fa" cx="150" cy="230" r="6"/><circle class="fa dim" cx="230" cy="170" r="6"/><circle class="fa dim" cx="310" cy="110" r="6"/>
    <text class="ft" x="30" y="316">we are here: first step</text></g></svg>`,
};

const PAGES = [
  { slug: 'manifesto', title: 'Manifesto', lead: 'Why agents need a better-measured memory layer, what Prism is trying to show, and what would prove it wrong.', fig: 'planes', desc: 'The statement of direction and the falsifiers', badges: ['proposed'] },
  { slug: 'related-work', title: 'Related work', lead: 'Where Prism sits among quantization, streaming-index, filtered-search and snapshot systems, and what is already known.', fig: 'graph', desc: 'Prior art and the claim audit', badges: ['reported'] },
  { slug: 'algorithm', title: 'Algorithm', lead: 'One stored code, read progressively: encode once, score with a few bits, spend more only where the answer is still in doubt.', fig: 'funnel', desc: 'Encode, cascade, rerank, snapshot', badges: ['implemented', 'measured'] },
  { slug: 'mathematics', title: 'Mathematics', lead: 'The geometry, the rotation, the quantizer error, the pruning gate and its proof, with every assumption stated.', fig: 'bell', desc: 'Derivations, lemma, open problem', badges: ['proved', 'proposed'] },
  { slug: 'tournament', title: 'Algorithm tournament', lead: 'Seven candidate designs, one common yardstick. No winner is declared until the evidence exists.', fig: 'grid', desc: 'Candidates A to G, keep or defer', badges: ['unmeasured'] },
  { slug: 'snapshot-protocol', title: 'Snapshot protocol', lead: 'A reader pins a point in the log. What that has to mean, which states are legal, and what is still undefined.', fig: 'timeline', desc: 'Visibility, publication, reclamation', badges: ['proposed'] },
  { slug: 'loopholes', title: 'Loophole audit', lead: 'The places where an algorithm that looks right on easy data can break. Open items stay open until a test closes them.', fig: 'audit', desc: 'Math, storage, filter, concurrency', badges: ['open'] },
  { slug: 'roadmap', title: 'Roadmap', lead: 'What is done, what is next, and what we cannot do here. Ordered by what would change our minds.', fig: 'steps', desc: 'Stages A to J and the licence gate', badges: ['open'] },
];

const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/"/g, '&quot;');
const badge = (b) => `<span class="badge b-${b}">${b === 'reported' ? 'Reported, not reproduced' : b[0].toUpperCase() + b.slice(1)}</span>`;

function bar(cur) {
  const items = PAGES.map((p, i) => `<a href="${p.slug}.html"${p.slug === cur ? ' aria-current="page"' : ''}><b>${String(i + 1).padStart(2, '0')}</b>${esc(p.title)}<small>${esc(p.desc)}</small></a>`).join('');
  return `<header class="lb-bar" role="banner"><a class="logo" href="index.html" aria-label="Project Apex home"></a>
  <a class="lb-brand" href="prism.html">Prism</a>
  <button class="lb-burger" type="button" aria-expanded="false" aria-controls="lb-nav">Menu</button>
  <ul class="lb-nav" id="lb-nav" aria-label="Primary">
    <li><a href="prism.html#abstract">Abstract</a></li><li><a href="prism.html#film-sec">Film</a></li>
    <li><button class="lb-dd cur" type="button" aria-expanded="false" aria-haspopup="true" aria-controls="lb-menu">Manifesto &#9662;</button>
      <div class="lb-menu" id="lb-menu">${items}</div></li>
    <li><a href="prism.html#lab">Lab</a></li><li><a href="prism.html#paper">Paper</a></li>
    <li><a href="prism.html#evidence">Evidence<i class="ping"></i></a></li>
    <li><a href="${REPO}">Source &#8599;</a></li>
  </ul></header><div class="lb-prog" aria-hidden="true"></div>`;
}

function foot() {
  const col = PAGES.map((p) => `<a href="${p.slug}.html">${esc(p.title)}</a>`).join('');
  return `<footer class="lb-foot"><div class="cols">
    <div><h4>Project Apex / Prism</h4><p>An open research prototype about progressively refined vector codes and per-reader snapshots. Research Note v1.0, preprint, not peer reviewed.</p></div>
    <div><h4>Pages</h4>${col}</div>
    <div><h4>Evidence</h4><a href="prism.html#evidence">Measurements</a><a href="prism.html#paper">Research note</a><a href="prism.html#lab">Interactive lab</a></div>
    <div><h4>Open source</h4><a href="${REPO}">Repository</a><a href="${REPO}/tree/main/benchmarks/prism">Benchmark source</a><a href="${REPO}/issues">Issues</a></div></div>
    <p class="fine">Synthetic data, one machine. No comparison with other systems. Nothing on these pages is a claim to be first, fastest or best.</p></footer>`;
}

function build(p, i) {
  const body = readFileSync(join(SITE, 'src', `${p.slug}.html`), 'utf8');
  const toc = [...body.matchAll(/<section id="([^"]+)"[^>]*>\s*<h2>(.*?)<\/h2>/g)].map((m) => `<li><a href="#${m[1]}">${m[2]}</a></li>`).join('');
  const prev = PAGES[i - 1], next = PAGES[i + 1];
  const pn = (q, dir) => (q ? `<a href="${q.slug}.html"><small>${dir}</small><span>${esc(q.title)}</span></a>` : `<a href="prism.html"><small>${dir === 'Previous' ? 'Overview' : 'Back to'}</small><span>Prism overview</span></a>`);
  const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>${esc(p.title)} | Prism | Project Apex</title>
<meta name="description" content="${esc(p.lead)}">
<meta name="theme-color" content="#232323">
<link rel="icon" href="assets/favicon.ico" sizes="any">
<link rel="preload" href="fonts/geist.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="fonts/geist-mono.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="css/main.css">
<link rel="stylesheet" href="css/lab.css">
</head>
<body class="lab">
<a class="skip" href="#main">Skip to content</a>
${bar(p.slug)}
<main id="main">
<section class="lh" aria-labelledby="h1">
  <div class="lh-l">
    <p class="crumb"><a href="prism.html">Prism</a><span>/</span><a href="manifesto.html">Manifesto</a>${p.slug === 'manifesto' ? '' : `<span>/</span><span>${esc(p.title)}</span>`}</p>
    <h1 id="h1">${esc(p.title)}</h1>
    <p class="lead">${esc(p.lead)}</p>
    <div class="status-row">${p.badges.map(badge).join(' ')}</div>
    <p class="meta">Research Note v1.0 &middot; preprint, not peer reviewed &middot; synthetic data, one machine</p>
  </div>
  <div class="lh-r">${FIGS[p.fig]}<span class="num" aria-hidden="true">${String(i + 1).padStart(2, '0')}</span></div>
</section>
<div class="lb-main">
  <nav class="toc" aria-label="On this page"><p>On this page</p><ol>${toc}</ol></nav>
  <article class="art">${body}</article>
</div>
<nav class="pn" aria-label="Page navigation">${pn(prev, 'Previous')}${pn(next, 'Next')}</nav>
</main>
${foot()}
<script src="js/lab.js" defer></script>
</body>
</html>
`;
  writeFileSync(join(SITE, `${p.slug}.html`), html);
  return `${p.slug}.html`;
}

console.log(PAGES.map(build).join('\n'));
