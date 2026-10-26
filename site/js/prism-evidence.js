// Renders the evidence section from data/prism-results.json. Values are formatted numbers only.
(() => {
  "use strict";
  const root = document.getElementById("evidence-root");
  if (!root) return;
  const f = (x, p = 3) => Number(x).toFixed(p);
  const esc = s => String(s).replace(/[&<>"]/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
  const table = (head, rows) =>
    `<div class="tbl"><table><thead><tr>${head.map((h, i) => `<th class="${i ? "n" : ""}">${esc(h)}</th>`).join("")}</tr></thead><tbody>${rows.map(r => `<tr>${r.map((c, i) => `<td class="${i ? "n" : ""}">${esc(c)}</td>`).join("")}</tr>`).join("")}</tbody></table></div>`;

  fetch("data/prism-results.json").then(r => { if (!r.ok) throw new Error(r.status); return r.json(); }).then(d => {
    const { n, d: dim, nq, k, seed } = d.params, perV = n * dim;
    const bpc = x => x / perV;
    const cas = d.cascade, byZ = z => cas.find(c => c.z === z);
    const z2 = byZ(2), z3 = byZ(3);
    const bars = [["Float32 scan", 32, 0], ["Flat 4-bit scan", 4, 0], ["Cascade z = 2", bpc(z2.bits_incr_mean), 1], ["Cascade z = 3", bpc(z3.bits_incr_mean), 1]];
    const bar = ([name, v, hl]) => `<div class="bar ${hl ? "hl" : ""}"><span>${esc(name)}</span><i><b style="width:${Math.max(1.5, v / 32 * 100)}%"></b></i><em>${f(v, v < 10 ? 2 : 0)}</em></div>`;

    root.innerHTML = `
      <p class="mono" style="margin:30px 0 0;color:var(--mid)">Bits read per coordinate per vector, per query</p>
      <div class="bars" role="img" aria-label="Bits read: float32 32, flat 4-bit 4, cascade z=2 ${f(bpc(z2.bits_incr_mean), 2)}, cascade z=3 ${f(bpc(z3.bits_incr_mean), 2)}">${bars.map(bar).join("")}</div>
      <p class="note-m">Setup: ${n.toLocaleString()} vectors, d = ${dim}, ${nq.toLocaleString()} queries, k = ${k}, seed ${seed}. ${d.synthetic ? "Synthetic data. " : ""}${esc(d.note)}. rustc: ${esc(d.rustc)}. Source checksum (FNV-1a, src/*.rs): ${esc(d.src_fnv1a64)}. CPU note: ${esc(d.cpu_note)}.</p>

      <h3 style="margin-top:56px">Error per bit, and no drift</h3>
      ${table(["Bits", "σ measured", "σ theory", "MSE original", "MSE shifted"], d.sigma_mse.map(s => [s.bits, f(s.sigma_measured, 4), f(s.sigma_theory, 4), f(s.mse_original, 4), f(s.mse_shifted, 4)]))}

      <h3 style="margin-top:40px">Search quality and cost</h3>
      ${table(["Method", "Recall@10", "Queries missing one", "Bits (new planes)", "Bits (prefix)", "Survivors per stage", "Reranks mean (p95)"],
        d.flat_fixed_rerank100.map(x => [`Flat ${x.bits}-bit, rerank 100`, f(x.recall), "–", x.bits, x.bits, "–", "100"]).concat(
        cas.map(c => [`Cascade z = ${c.z}`, f(c.recall, 4), (c.miss_rate * 100).toFixed(1) + "%", f(bpc(c.bits_incr_mean)), f(bpc(c.bits_prefix_mean)), c.survivors_mean_per_stage.map(Math.round).join(" / "), `${Math.round(c.reranks_mean)} (${c.reranks_p95})`])))}
      <p class="note-m">Stage 1 keeps about 399 vectors because the generator makes 50 clusters of about 400 and each query comes from one of them. That is the easy case. A different query distribution read 1.77 bits in an earlier prototype.</p>

      <h3 style="margin-top:40px">Time, single thread</h3>
      ${table(["Method", "p50 ms", "p95 ms"], d.wall_clock.map(w => [w.name.replace(/_/g, " "), f(w.p50_ms, 2), f(w.p95_ms, 2)]))}
      <p class="note-m">The cascade is slower than an exact float scan in this scalar implementation. Bits read did not predict time. No speed claim is made.</p>

      <h3 style="margin-top:40px">Concurrent writes</h3>
      ${table(["", "Queries", "p50 ms", "p95 ms", "p99 ms"], [["Writer off", d.concurrency.writer_off], ["Writer on", d.concurrency.writer_on]].map(([l, w]) => [l, w.queries, f(w.p50_ms, 2), f(w.p95_ms, 2), f(w.p99_ms, 2)]))}
      <p class="note-m">Writer: ${Math.round(d.concurrency.writes_vectors_per_s).toLocaleString()} vectors/s, ${Math.round(d.concurrency.commits_per_s)} commits/s. P99 ratio concurrent / serial: ${f(d.concurrency.jitter_ratio_p99, 2)}. A ratio below 1 means noise on a busy machine exceeds any writer effect. It is not evidence that writing helps. The snapshot invariants are asserted by the tests.</p>`;
    if (window.prismFx) window.prismFx.scan();
  }).catch(() => {
    root.innerHTML = `<p class="grey" style="margin-top:30px">The results file could not be loaded here. The same numbers are in <code>benchmarks/prism/results/results.json</code> and in the paper.</p>`;
  });
})();
