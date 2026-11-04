// Generates site/benchmarks.html from the raw head-to-head JSON. Every number on the page is read from those files.
import { readFileSync, writeFileSync, mkdirSync, copyFileSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { head, header, footer, esc, REPO } from './shell.mjs';

const SITE = join(dirname(fileURLToPath(import.meta.url)), '..');
const RES = join(SITE, '..', 'benchmarks', 'h2h', 'results');
const DS = process.argv[2] || 'fashion';
const DIR = join(RES, DS);
const ENGINES = [['atlas', 'Bat DB (Atlas)'], ['hnswlib', 'hnswlib'], ['qdrant', 'Qdrant']];
const rd = (f) => JSON.parse(readFileSync(join(DIR, f), 'utf8'));
const data = {};
for (const [k] of ENGINES) if (existsSync(join(DIR, `${k}.json`))) data[k] = rd(`${k}.json`);
const cfg = existsSync(join(DIR, 'config.json')) ? rd('config.json') : {};
mkdirSync(join(SITE, 'data', 'bench', DS), { recursive: true });
for (const f of ['atlas.json', 'hnswlib.json', 'qdrant.json', 'config.json']) if (existsSync(join(DIR, f))) copyFileSync(join(DIR, f), join(SITE, 'data', 'bench', DS, f));

const label = Object.fromEntries(ENGINES);
const f1 = (x) => (x >= 100 ? Math.round(x).toLocaleString('en-US') : x.toFixed(x < 10 ? 2 : 1));
const sweep = (k) => (data[k]?.sweep || []).map((s) => ({ ef: s.ef, r: s.median.recall_at_10, q: s.median.qps_seq, p50: s.median.p50_ms, p99: s.median.p99_ms, c8: s.qps_conc8 }));
const pm = (x) => x.selectivity_permille ?? Math.round(parseFloat(String(x.selectivity)) * 10);
const filt = (k) => Object.fromEntries((data[k]?.filtered || []).map((x) => [pm(x), x]));

/* chart: recall (x) vs sequential QPS (y, log) */
function chart() {
  const W = 640, H = 340, L = 56, R = 16, T = 14, B = 44, shade = { atlas: '#ffffff', hnswlib: '#a3a3a3', qdrant: '#525252' };
  const pts = ENGINES.flatMap(([k]) => sweep(k).map((p) => ({ k, ...p })));
  if (!pts.length) return '';
  const rs = pts.map((p) => p.r), qs = pts.map((p) => p.q);
  const rmin = Math.min(...rs) - 0.002, rmax = Math.max(...rs) + 0.001, qmin = Math.log10(Math.min(...qs) * 0.8), qmax = Math.log10(Math.max(...qs) * 1.2);
  const x = (r) => L + ((r - rmin) / (rmax - rmin)) * (W - L - R), y = (q) => T + (1 - (Math.log10(q) - qmin) / (qmax - qmin)) * (H - T - B);
  let s = `<svg viewBox="0 0 ${W} ${H}" role="img" aria-label="Recall at 10 against queries per second, one client, log scale" class="bn-chart">`;
  for (let e = Math.ceil(qmin); e <= Math.floor(qmax); e++) { const yy = y(10 ** e); s += `<line x1="${L}" x2="${W - R}" y1="${yy}" y2="${yy}" class="bn-grid"/><text x="${L - 8}" y="${yy + 4}" text-anchor="end" class="bn-ax">${(10 ** e).toLocaleString('en-US')}</text>`; }
  for (let r = Math.ceil(rmin * 100) / 100; r <= rmax; r += 0.01) s += `<text x="${x(r)}" y="${H - 24}" text-anchor="middle" class="bn-ax">${r.toFixed(2)}</text>`;
  s += `<text x="${(L + W - R) / 2}" y="${H - 4}" text-anchor="middle" class="bn-ax">recall@10 (higher is better)</text><text transform="translate(14 ${H / 2}) rotate(-90)" text-anchor="middle" class="bn-ax">queries per second, one client (higher is better)</text>`;
  for (const [k] of ENGINES) {
    const p = sweep(k); if (!p.length) continue;
    s += `<polyline fill="none" stroke="${shade[k]}" stroke-width="2" ${k === 'qdrant' ? 'stroke-dasharray="5 4"' : ''} points="${p.map((a) => `${x(a.r)},${y(a.q)}`).join(' ')}"/>`;
    s += p.map((a) => `<circle cx="${x(a.r)}" cy="${y(a.q)}" r="3.5" fill="${shade[k]}"><title>${esc(label[k])} ef=${a.ef}: recall ${a.r.toFixed(4)}, ${f1(a.q)} QPS</title></circle>`).join('');
  }
  s += '</svg><p class="bn-leg">' + ENGINES.filter(([k]) => data[k]).map(([k, n]) => `<span><i style="background:${shade[k]}"></i>${esc(n)}</span>`).join('') + '</p>';
  return s;
}

const sweepTable = () => {
  const efs = [...new Set(ENGINES.flatMap(([k]) => sweep(k).map((p) => p.ef)))].sort((a, b) => a - b);
  const rows = efs.flatMap((ef) => ENGINES.filter(([k]) => data[k]).map(([k, n]) => { const p = sweep(k).find((a) => a.ef === ef); return p ? `<tr><td>${ef}</td><td>${esc(n)}</td><td>${p.r.toFixed(4)}</td><td>${f1(p.p50)}</td><td>${f1(p.p99)}</td><td>${f1(p.q)}</td><td>${f1(p.c8)}</td></tr>` : ''; }));
  return `<div class="bn-scroll"><table class="bn-t"><thead><tr><th>ef</th><th>Engine</th><th>recall@10</th><th>p50 ms</th><th>p99 ms</th><th>QPS, 1 client</th><th>QPS, 8 workers</th></tr></thead><tbody>${rows.join('')}</tbody></table></div>`;
};

const SELS = [[100, '10%'], [50, '5%'], [10, '1%'], [1, '0.1%']];
const filtTable = () => {
  const rows = SELS.flatMap(([m, n]) => ENGINES.filter(([k]) => data[k]).map(([k, nm]) => { const x = filt(k)[m]; return x ? `<tr><td>${n}</td><td>${esc(nm)}</td><td>${esc(x.algorithm || x.path || '')}</td><td>${(x.recall_at_10).toFixed(3)}</td><td>${f1(x.p50_ms)}</td><td>${f1(x.p99_ms)}</td><td>${x.leaks_in_200q ?? 0}</td></tr>` : ''; }));
  return `<div class="bn-scroll"><table class="bn-t"><thead><tr><th>Selectivity</th><th>Engine</th><th>Path</th><th>recall@10</th><th>p50 ms</th><th>p99 ms</th><th>Leaks / 200 q</th></tr></thead><tbody>${rows.join('')}</tbody></table></div>`;
};

/* headline cards, computed (never typed) */
function cards() {
  const a = filt('atlas')[10], h = filt('hnswlib')[10], q = filt('qdrant')[10];
  const c = [];
  if (a && h && q) c.push([`${f1(a.p50_ms)} ms`, `p50 at 1% filter selectivity. hnswlib ${f1(h.p50_ms)} ms, Qdrant ${f1(q.p50_ms)} ms.`, `${f1(h.p50_ms / a.p50_ms)}x faster than hnswlib`]);
  const sa = sweep('atlas'), sh = sweep('hnswlib'), sq = sweep('qdrant');
  if (sa.length && sh.length) { const i = sa.findIndex((p) => p.ef === 64); if (i >= 0 && sh[i]) c.push([`${f1(sa[i].c8)} QPS`, `with 8 workers at ef=64 (recall ${sa[i].r.toFixed(3)}). hnswlib ${f1(sh[i].c8)}.`, sa[i].c8 > sh[i].c8 ? 'ahead of hnswlib' : 'behind hnswlib']); }
  if (sa.length && sq.length) { const i = sa.findIndex((p) => p.ef === 64); if (i >= 0 && sq[i]) c.push([`${f1(sa[i].q / sq[i].q)}x`, `Qdrant throughput on one client at ef=64. Qdrant is a server over HTTP; Atlas is called in-process here.`, 'read the caveats']); }
  return c.map(([n, d, t]) => `<article class="bn-card"><b>${esc(n)}</b><p>${esc(d)}</p><span>${esc(t)}</span></article>`).join('');
}
const loses = () => {
  const sa = sweep('atlas'), sh = sweep('hnswlib'); if (!sa.length || !sh.length) return '';
  const r = sa.map((p, i) => (sh[i] ? sh[i].q / p.q : 0)).filter(Boolean); const b = (data.hnswlib?.build_seconds || 0) / (data.atlas?.build_seconds || 1);
  return `<li><b>Single-client speed.</b> hnswlib is ${f1(Math.min(...r))}x to ${f1(Math.max(...r))}x faster than Bat DB at the same recall on this run. We are working on it; it is the main thing a leaderboard such as ann-benchmarks measures.</li><li><b>Build time.</b> Bat DB built the index in ${f1(data.atlas.build_seconds)} s, hnswlib in ${f1(data.hnswlib.build_seconds)} s (${f1(1 / b)}x slower). Bat DB built single-threaded in this harness.</li>`;
};

const meta = data.atlas ? `${(data.atlas.n || 0).toLocaleString('en-US')} vectors, ${data.atlas.dim} dimensions, M=${data.atlas.params?.M}, ef_construction=${data.atlas.params?.ef_construction}, ${data.atlas.reps} repetitions (median)` : '';
const body = `<main id="main" class="bn">
<h1>Benchmarks you can rerun.</h1>
<p class="lead">Bat DB against hnswlib and Qdrant on a public dataset, with the raw results and the one command that produced them. This is our own head-to-head, <b>not a public leaderboard rank</b>. We show where we win and where we lose.</p>
<p class="bn-meta">Dataset: <b>${esc(DS)}</b> (ann-benchmarks HDF5). ${esc(meta)}. Run ${esc((cfg.date_utc || '').slice(0, 10))} on ${esc(cfg.host_cpu || 'one machine')}, Docker ${cfg.engine_cpu_limit || ''} CPUs, ${esc(cfg.engine_mem_limit || '')} RAM. Source digest <code>${esc(String(cfg.core_digest || 'n/a').slice(0, 12))}</code>.</p>
<section aria-label="Headline numbers" class="bn-cards">${cards()}</section>
<h2>Recall against speed</h2>${chart()}
<h2>Filtered search</h2><p class="bn-p">Each engine uses its own native filtered path. Bat DB switches to an exact scan over allowed ids when the filter is very selective, which is why it is fast there. Leaks count results outside the filter; the right number is 0.</p>${filtTable()}
<h2>All measurements</h2>${sweepTable()}
<h2>Where we lose</h2><ul class="bn-list">${loses()}</ul>
<h2>Caveats</h2><ul class="bn-list"><li>Bat DB and hnswlib run in-process. Qdrant is a server reached over HTTP, so its latency includes the network stack. Compare Bat DB with hnswlib for engine speed, and with Qdrant for end-to-end behaviour.</li><li>One machine, synthetic-free public data, ${esc(String(data.atlas?.reps || ''))} repetitions. No claim here is a ranking on ann-benchmarks or VectorDBBench.</li><li>Raw files: ${ENGINES.filter(([k]) => data[k]).map(([k, n]) => `<a href="data/bench/${esc(DS)}/${k}.json">${esc(n)}</a>`).join(', ')}, <a href="data/bench/${esc(DS)}/config.json">run configuration</a>.</li></ul>
<h2>Reproduce</h2><pre class="bn-pre">git clone ${esc(REPO)}
cd Project-Apex
bash benchmarks/h2h/run.sh ${esc(DS)} all</pre>
</main>`;

const css = `.bn{max-width:1000px;margin:0 auto;padding:120px 20px 80px;color:#e5e5e5}.bn h1{font-size:clamp(36px,5.5vw,68px);letter-spacing:-.04em;line-height:1;font-weight:400;color:#fff}.bn .lead{color:#a3a3a3;font-size:18px;line-height:1.5;max-width:64ch;margin-top:18px}.bn .lead b{color:#fff;font-weight:500}.bn h2{font-size:26px;font-weight:500;color:#fff;margin:56px 0 14px;letter-spacing:-.02em}.bn-meta,.bn-p{color:#a3a3a3;font-size:14px;line-height:1.55;margin-top:16px;max-width:76ch}.bn code{font-family:var(--mono);background:#1c1c1c;border:1px solid #262626;border-radius:4px;padding:1px 5px;font-size:.9em}.bn-cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(250px,1fr));gap:14px;margin-top:30px}.bn-card{background:#141414;border:1px solid #262626;border-radius:14px;padding:20px;display:flex;flex-direction:column;gap:8px}.bn-card b{font-size:34px;font-weight:500;letter-spacing:-.03em;color:#fff}.bn-card p{color:#a3a3a3;font-size:14px;line-height:1.45}.bn-card span{margin-top:auto;font-family:var(--mono);font-size:11.5px;text-transform:uppercase;color:#d4d4d4}.bn-chart{width:100%;height:auto;background:#141414;border:1px solid #262626;border-radius:14px}.bn-grid{stroke:#262626}.bn-ax{fill:#a3a3a3;font:11px var(--mono)}.bn-leg{display:flex;gap:18px;flex-wrap:wrap;margin-top:10px;font-size:13px;color:#a3a3a3}.bn-leg i{display:inline-block;width:12px;height:12px;border-radius:3px;margin-right:6px;vertical-align:-1px}.bn-scroll{overflow-x:auto;border:1px solid #262626;border-radius:12px}.bn-t{width:100%;border-collapse:collapse;font-size:13.5px;min-width:620px}.bn-t th{background:#1c1c1c;color:#fff;font:400 11.5px var(--mono);text-transform:uppercase;letter-spacing:.04em;text-align:left;padding:10px 12px;border-bottom:1px solid #404040}.bn-t td{padding:9px 12px;border-bottom:1px solid #262626;color:#d4d4d4;font-variant-numeric:tabular-nums}.bn-t tr:last-child td{border-bottom:0}.bn-list{margin:0 0 0 20px;color:#d4d4d4;line-height:1.6;display:grid;gap:10px;max-width:76ch}.bn-list b{color:#fff;font-weight:500}.bn-list a{color:#fff;text-underline-offset:3px}.bn-pre{background:#141414;border:1px solid #262626;border-radius:12px;padding:16px;font:13px/1.6 var(--mono);color:#e5e5e5;overflow:auto}
`;
writeFileSync(join(SITE, 'css', 'benchmarks.css'), css);
const html = `${head({ title: 'Benchmarks', desc: `Bat DB against hnswlib and Qdrant on ${DS}: recall, speed, filtered search, raw data and the command to reproduce it.`, extraCss: ['benchmarks'] })}\n<body class="gxp pg-benchmarks">\n<a class="skip" href="#main">Skip to content</a>\n${header('benchmarks')}\n${body}\n${footer()}\n<script src="js/site.js" defer></script>\n</body>\n</html>\n`;
writeFileSync(join(SITE, 'benchmarks.html'), html);
console.log('benchmarks.html from', DIR, Object.keys(data).join(','));
