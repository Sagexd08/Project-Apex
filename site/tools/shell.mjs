// Shared header, mega-menu and footer for every page. Plain HTML strings; no framework.
export const REPO = 'https://github.com/Sagexd08/Project-Apex';
export const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

export const GROUPS = [
  { name: 'The argument', pages: [
    { slug: 'manifesto', title: 'Manifesto', desc: 'Why we are building this, and what would prove us wrong' },
    { slug: 'abstract', title: 'Abstract', desc: 'The whole research note in one page' },
    { slug: 'paper', title: 'Research paper', desc: 'arXiv-style, with every table. Save as PDF' },
    { slug: 'evidence', title: 'Evidence', desc: 'Every measurement, loaded from the raw results' },
  ] },
  { name: 'The method', pages: [
    { slug: 'algorithm', title: 'Algorithm', desc: 'Encode, cascade, rerank, snapshot' },
    { slug: 'mathematics', title: 'Mathematics', desc: 'Derivations, lemma, open problem' },
    { slug: 'film', title: 'Film', desc: 'The mathematics in ninety seconds' },
    { slug: 'lab', title: 'Interactive lab', desc: 'Move the sliders, see the geometry' },
    { slug: 'snapshot-protocol', title: 'Snapshot protocol', desc: 'What a reader pin has to mean' },
  ] },
  { name: 'The honesty', pages: [
    { slug: 'related-work', title: 'Related work', desc: 'Prior art and the claim audit' },
    { slug: 'tournament', title: 'Algorithm tournament', desc: 'Seven candidates, no winner yet' },
    { slug: 'loopholes', title: 'Loophole audit', desc: 'Where an easy-data win can break' },
    { slug: 'roadmap', title: 'Roadmap', desc: 'Done, next, and what we cannot do' },
  ] },
];
export const ALL = GROUPS.flatMap((g) => g.pages);

const CHEV = '<svg viewBox="0 0 12 12" aria-hidden="true"><path d="M2 4.2 6 8l4-3.8" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>';

export function menu(cur) {
  let n = 0;
  return `<div class="gx-menu" id="gx-menu" role="group" aria-label="Manifesto pages">${GROUPS.map((g) =>
    `<div class="gx-col"><h4>${esc(g.name)}</h4>${g.pages.map((p) => `<a href="${p.slug}.html"${p.slug === cur ? ' aria-current="page"' : ''}><b>${String(++n).padStart(2, '0')}</b>${esc(p.title)}<small>${esc(p.desc)}</small></a>`).join('')}</div>`).join('')}</div>`;
}

export const trigger = (cur) => `<div class="gx-dd"><a href="manifesto.html"${ALL.some((p) => p.slug === cur) ? ' aria-current="page"' : ''}>Manifesto</a><button class="gx-arrow" type="button" aria-label="Show Manifesto pages" aria-haspopup="true" aria-expanded="false" aria-controls="gx-menu">${CHEV}</button>${menu(cur)}</div>`;

export function header(cur) {
  return `<header class="gx" id="gx"><a class="gx-logo" href="index.html" aria-label="Bat DB, home"><i></i>Bat DB</a>
<button class="gx-burger" type="button" aria-expanded="false" aria-controls="gx-links">Menu</button>
<ul class="gx-links" id="gx-links">
<li><a href="index.html">Home</a></li><li><a href="index.html#product">Product</a></li><li><a href="index.html#benchmarks">Benchmarks</a></li>
<li>${trigger(cur)}</li><li><a href="${REPO}" rel="noopener">Source &#8599;</a></li></ul>
<div class="gx-cta"><a href="index.html#status">What&rsquo;s done</a></div></header><div class="gx-prog" id="gx-prog" aria-hidden="true"></div>`;
}

export function footer() {
  const col = (g) => `<div><h4>${esc(g.name)}</h4>${g.pages.map((p) => `<a href="${p.slug}.html">${esc(p.title)}</a>`).join('')}</div>`;
  return `<footer class="footx"><div class="cols">
<div><h4>Bat DB</h4><p>An open-source vector database for AI agent memory, written in Rust. Research Note v1.0, preprint, not peer reviewed. Early development; not ready for production.</p><a href="index.html">Home</a><a href="${REPO}">Repository</a></div>
${GROUPS.map(col).join('')}</div>
<p class="fine">Synthetic data, one machine. No comparison with other systems. Nothing on these pages claims to be first, fastest or best.</p></footer>`;
}

export function head({ title, desc, extraCss = [] }) {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>${esc(title)} | Bat DB</title>
<meta name="description" content="${esc(desc)}">
<meta name="theme-color" content="#071a3d">
<link rel="icon" href="assets/favicon.ico" sizes="any">
<link rel="icon" type="image/png" sizes="32x32" href="assets/favicon-32.png">
<link rel="apple-touch-icon" href="assets/apple-touch-icon.png">
<link rel="preload" href="fonts/geist.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="fonts/geist-mono.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="css/main.css">
${extraCss.map((c) => `<link rel="stylesheet" href="css/${c}.css">`).join('\n')}
<link rel="stylesheet" href="css/site.css">
</head>`;
}
