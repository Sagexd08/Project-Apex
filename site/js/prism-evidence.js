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
      <p class="note-m">Status of everything below: <b>Measured</b> (synthetic data, one machine, not reproduced elsewhere). Bits read is not total memory: raw float32 vectors are kept for the exact rerank, so 4-bit planes add 1/8 to float32 storage and do not replace it. The z gate is a heuristic; only the deterministic bound is certified.</p>
      <p class="note-m">Setup: ${n.toLocaleString()} vectors, d = ${dim}, ${nq.toLocaleString()} queries, k = ${k}, seed ${seed}. ${d.synthetic ? "Synthetic data. " : ""}${esc(d.note)}. rustc: ${esc(d.rustc)}. Source checksum (FNV-1a, src/*.rs): ${esc(d.src_fnv1a64)}. CPU note: ${esc(d.cpu_note)}.</p>

      <h3 style="margin-top:56px">Error per bit, and no drift</h3>
      ${table(["Bits", "σ measured", "σ theory", "MSE original", "MSE shifted"], d.sigma_mse.map(s => [s.bits, f(s.sigma_measured, 4), f(s.sigma_theory, 4), f(s.mse_original, 4), f(s.mse_shifted, 4)]))}

      <h3 style="margin-top:40px">Search quality and cost</h3>
      ${table(["Method", "Recall@10", "Queries missing one", "Bits (new planes)", "Bits (prefix)", "Survivors per stage", "Reranks mean (p95)"],
        d.flat_fixed_rerank100.map(x => [`Flat ${x.bits}-bit, rerank 100`, f(x.recall), "–", x.bits, x.bits, "–", "100"]).concat(
        cas.map(c => [`Cascade z = ${c.z}`, f(c.recall, 4), (c.miss_rate * 100).toFixed(1) + "%", f(bpc(c.bits_incr_mean)), f(bpc(c.bits_prefix_mean)), c.survivors_mean_per_stage.map(Math.round).join(" / "), `${Math.round(c.reranks_mean)} (${c.reranks_p95})`])))}
      <p class="note-m">Stage 1 keeps about 399 vectors because the generator makes 50 clusters of about 400 and each query comes from one of them. That is the easy case. Out-of-distribution queries read 1.40 bits; see below.</p>

      <h3 style="margin-top:40px">Time, single thread</h3>
      ${table(["Method", "p50 ms", "p95 ms"], d.wall_clock.map(w => [w.name.replace(/_/g, " "), f(w.p50_ms, 2), f(w.p95_ms, 2)]))}
      <p class="note-m">This first run used a scalar kernel and an unoptimised exact scan. The lookup-table results below are the fair comparison. No speed claim is made.</p>

      <h3 style="margin-top:40px">Concurrent writes</h3>
      ${table(["", "Queries", "p50 ms", "p95 ms", "p99 ms"], [["Writer off", d.concurrency.writer_off], ["Writer on", d.concurrency.writer_on]].map(([l, w]) => [l, w.queries, f(w.p50_ms, 2), f(w.p95_ms, 2), f(w.p99_ms, 2)]))}
      <p class="note-m">Writer: ${Math.round(d.concurrency.writes_vectors_per_s).toLocaleString()} vectors/s, ${Math.round(d.concurrency.commits_per_s)} commits/s. P99 ratio concurrent / serial: ${f(d.concurrency.jitter_ratio_p99, 2)}. A ratio below 1 means noise on a busy machine exceeds any writer effect. It is not evidence that writing helps. The snapshot invariants are asserted by the tests.</p>`;

    const x = d.extra;
    if (x) {
      const nm = k => k.replace(/^[a-e]_/, "").replace(/_/g, " ");
      const ds = Object.entries(x.datasets), gs = Object.entries(x.gates), ed = Object.values(x.error_dist);
      const mb = (v, nn) => f(v / (nn * dim), 2);
      const rg = (arr, g) => { const v = arr.map(g); return f(Math.min(...v), 2) + " to " + f(Math.max(...v), 2); };
      root.insertAdjacentHTML("beforeend", `
      <h2 style="margin-top:90px;font-size:var(--t-h1)">After the fixes</h2>
      <p class="grey">A lookup-table kernel, harder data, error-tail measurements, per-vector gates and repeated concurrency trials. Source checksum ${esc(d.src_fnv1a64)}.</p>
      <h3 style="margin-top:34px">Kernel timing, single thread (n = ${n.toLocaleString()})</h3>
      ${table(["Path", "p50 ms", "p95 ms"], x.kernel_timing.map(t => [t.name.replace(/_/g, " "), f(t.p50_ms, 2), f(t.p95_ms, 2)]))}
      <p class="note-m">The lookup-table cascade is about 7× faster than the scalar cascade and still not faster than an optimised exact scan.</p>
      <h3 style="margin-top:40px">Harder data, cascade at z = 2 (z = 4 bits for contrast)</h3>
      ${table(["Data set", "n", "Recall@10", "Missing one", "Bits z=2", "Bits z=4", "p50 ms"], ds.map(([k, v]) => { const a = v.cascade_lut.find(r => r.z === 2), b = v.cascade_lut.find(r => r.z === 4); return [nm(k), v.n.toLocaleString(), f(a.recall, 4), (a.miss_rate * 100).toFixed(1) + "%", mb(a.bits_incr_mean, v.n), mb(b.bits_incr_mean, v.n), f(a.time_p50_ms, 2)]; }))}
      <p class="note-m">Bits are per coordinate per vector. Time is over the first 200 queries. The 1.77 bits of the early prototype was not reproduced.</p>
      <h3 style="margin-top:40px">Error tails across the four 20,000-vector sets (20 million pairs each)</h3>
      ${table(["Bits", "Excess kurtosis", "99.9% of |e|/σ", "Max |e|/σ"], [0, 1, 2].map(i => [ed[0][i].bits, rg(ed, e => e[i].excess_kurtosis), rg(ed, e => e[i].q99_9_abs_over_sigma), rg(ed, e => e[i].max_abs_over_sigma)]))}
      <p class="note-m">A bell curve has kurtosis 0 and a 99.9% value of 3.29. The shape is close but the extremes reach 5 to 7σ.</p>
      <h3 style="margin-top:40px">Gates compared: bits per coordinate per vector (recall 1.000 on all)</h3>
      ${table(["Data set", "Global z=3", "Global z=4", "Per-vector δ=0.1%", "Deterministic", "Per-vector violations"], gs.map(([k, g]) => { const r = p => g.rows.find(q => q.gate.startsWith(p)), nn = x.datasets[k].n; return [nm(k), mb(r("global_z3").bits_incr_mean, nn), mb(r("global_z4").bits_incr_mean, nn), mb(r("per_vector").bits_incr_mean, nn), mb(r("deterministic").bits_incr_mean, nn), (100 * Math.max(...g.per_vector_bound.map(q => q.per_vector_violation_rate))).toFixed(3) + "% max"]; }))}
      <p class="note-m">The deterministic bound never misses and reads almost every bit. The per-vector bound costs about as much as z = 4.</p>
      <h3 style="margin-top:40px">Concurrent writer, seven alternating trials</h3>
      ${table(["Trial", "p99 off ms", "p99 on ms", "Ratio", "q/s off", "q/s on"], x.concurrency_repeats.trials.map((t, i) => [i + 1, f(t.p99_off_ms, 2), f(t.p99_on_ms, 2), f(t.p99_ratio, 2), Math.round(t.qps_off), Math.round(t.qps_on)]))}
      <p class="note-m">Median P99 ratio ${f(x.concurrency_repeats.median_p99_ratio, 2)} (min ${f(x.concurrency_repeats.min_p99_ratio, 2)}, max ${f(x.concurrency_repeats.max_p99_ratio, 2)}). Median throughput ${Math.round(x.concurrency_repeats.median_qps_off)} off, ${Math.round(x.concurrency_repeats.median_qps_on)} on. ${x.concurrency_repeats.readers} readers, ${x.concurrency_repeats.secs_per_condition} s per condition, writer about 1,900 vectors/s. One non-idle machine.</p>`);
    }
    if (window.prismFx) window.prismFx.scan();
  }).catch(() => {
    root.innerHTML = `<p class="grey" style="margin-top:30px">The results file could not be loaded here. The same numbers are in <code>benchmarks/prism/results/results.json</code> and in the paper.</p>`;
  });
})();
