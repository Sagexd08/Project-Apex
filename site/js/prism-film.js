(() => {
  "use strict";
  const cv = document.getElementById("film");
  if (!cv) return;
  const $ = id => document.getElementById(id);
  const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const W = 1280, H = 720, ctx = cv.getContext("2d");
  cv.width = W; cv.height = H;
  const C = { bg: "#3b0f1c", off: "#efe6d3", acc: "#a8b8a2", mid: "#d8d0be", line: "#6a3441", dim: "#a89c92" };
  const clamp = (v, a = 0, b = 1) => Math.min(b, Math.max(a, v));
  const seg = (t, a, b) => clamp((t - a) / (b - a));
  const ease = p => 1 - Math.pow(1 - p, 3);
  const lerp = (a, b, p) => a + (b - a) * p;
  const rngf = s => () => { s = (s + 0x6d2b79f5) >>> 0; let t = s; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
  const gauss = r => Math.sqrt(-2 * Math.log(r() || 1e-9)) * Math.cos(2 * Math.PI * r());

  function T(s, x, y, size, o = {}) {
    ctx.save(); ctx.globalAlpha *= o.a == null ? 1 : o.a; ctx.fillStyle = o.c || C.off; ctx.textAlign = o.al || "left";
    ctx.font = `${o.w || 700} ${size}px 'Geist Mono',ui-monospace,monospace`; ctx.fillText(s, x, y); ctx.restore();
  }
  const typed = (s, p) => s.slice(0, Math.floor(s.length * p));
  function L(x1, y1, x2, y2, col, w = 2, a = 1) { ctx.save(); ctx.globalAlpha *= a; ctx.strokeStyle = col; ctx.lineWidth = w; ctx.beginPath(); ctx.moveTo(x1, y1); ctx.lineTo(x2, y2); ctx.stroke(); ctx.restore(); }

  const CX = 380, CY = 390, R = 230, TH = 55 * Math.PI / 180;
  function circle(a = 1) { ctx.save(); ctx.globalAlpha *= a; ctx.strokeStyle = C.line; ctx.lineWidth = 2; ctx.beginPath(); ctx.arc(CX, CY, R, 0, 7); ctx.stroke(); ctx.restore(); }
  function vecs(th, qa = 1, xa = 1) {
    L(CX, CY, CX + R * qa, CY, C.acc, 6);
    L(CX, CY, CX + R * Math.cos(th) * xa, CY - R * Math.sin(th) * xa, C.off, 6);
    if (th > 0.02) { ctx.save(); ctx.strokeStyle = C.mid; ctx.lineWidth = 2; ctx.beginPath(); ctx.arc(CX, CY, 70, -th, 0); ctx.stroke(); ctx.restore(); }
  }
  function s1(t) {
    circle(seg(t, 0, .1));
    const th = TH * ease(seg(t, .25, .6));
    vecs(th, ease(seg(t, .05, .25)), ease(seg(t, .2, .35)));
    T("q", CX + R + 22, CY + 10, 34, { c: C.acc, a: seg(t, .15, .25) }); T("x", CX + R * Math.cos(th) + 18, CY - R * Math.sin(th) - 8, 34, { a: seg(t, .3, .4) });
    T("θ", CX + 86, CY - 20, 34, { c: C.mid, a: seg(t, .45, .6) });
    T(typed("cos θ = ⟨q, x⟩", seg(t, .55, .8)), 700, 340, 58);
    T("bigger inner product", 700, 430, 30, { c: C.mid, w: 500, a: seg(t, .78, .9) }); T("= smaller angle", 700, 470, 30, { c: C.acc, a: seg(t, .84, .95) });
  }

  const NL = 44;
  function s2(t) {
    circle(); vecs(TH); const show = Math.floor(NL * ease(seg(t, .08, .72)));
    let h = 0;
    for (let i = 0; i < show; i++) {
      const a = ((i * 0.6180339887) % 1) * Math.PI, sep = a < TH; if (sep) h++;
      L(CX - R * Math.cos(a), CY + R * Math.sin(a), CX + R * Math.cos(a), CY - R * Math.sin(a), sep ? C.acc : C.mid, sep ? 3 : 1, sep ? 1 : .35);
    }
    T("random hyperplanes: " + show, 700, 250, 30, { c: C.mid, w: 500 });
    T("separate q from x: " + h, 700, 295, 30, { c: C.acc });
    T("P[disagree] = θ / π", 700, 400, 50, { a: seg(t, .6, .8) });
    T("θ̂ = π · H / d", 700, 480, 58, { a: seg(t, .75, .92) });
    T("H = disagreeing bits, d = bits", 700, 525, 24, { c: C.mid, w: 500, a: seg(t, .85, 1) });
  }

  const r3 = rngf(5), N3 = 48;
  const spiky = Array.from({ length: N3 }, (_, i) => (i % 9 === 3 ? 1 : .08 + r3() * .1) * (r3() < .5 ? -1 : 1));
  const gs = Array.from({ length: N3 }, () => gauss(r3));
  const nrm = a => { const n = Math.hypot(...a); return a.map(v => v / n); };
  const SP = nrm(spiky), GA = nrm(gs);
  function s3(t) {
    const p = ease(seg(t, .28, .72)), base = 430, sc = 560;
    for (let i = 0; i < N3; i++) {
      const v = lerp(SP[i], GA[i], p), x = 64 + i * 24, hgt = v * sc;
      ctx.fillStyle = p > .5 ? C.off : C.acc; ctx.globalAlpha = .9; ctx.fillRect(x, hgt >= 0 ? base - hgt : base, 17, Math.abs(hgt)); ctx.globalAlpha = 1;
    }
    L(54, base, 1226, base, C.line, 2);
    T(p < .5 ? "one vector, a few large coordinates" : "same vector after rotation: every coordinate small", 64, 120, 30, { c: C.mid, w: 500 });
    T("x → R x", 64, 190, 54, { a: seg(t, .15, .3) });
    T("coordinate ≈ N(0, 1/d)", 64, 640, 50, { a: seg(t, .7, .9) });
    T("same length, nothing to train", 700, 640, 26, { c: C.acc, a: seg(t, .85, 1) });
  }

  function s4(t) {
    const x0 = 140, x1 = 1140, y = 330, hh = 110, v = .685, idx = 10;
    ctx.strokeStyle = C.line; ctx.lineWidth = 2; ctx.strokeRect(x0, y, x1 - x0, hh);
    const levels = [1, 2, 3, 4], bits = ["1", "10", "101", "1010"], names = ["1 bit · 2 cells", "2 bits · 4 cells", "3 bits · 8 cells", "4 bits · 16 cells"];
    let cur = -1;
    levels.forEach((k, i) => {
      const a = ease(seg(t, .08 + i * .19, .08 + i * .19 + .12)); if (a <= 0) return; cur = i;
      const n = 2 ** k;
      for (let j = 1; j < n; j++) L(x0 + (x1 - x0) * j / n, y, x0 + (x1 - x0) * j / n, y + hh, C.mid, j % (n / 2) === 0 ? 3 : 1, a * (j % (n / 2) === 0 ? .9 : .5));
    });
    if (cur >= 0) {
      const n = 2 ** levels[cur], j = Math.floor(v * n), a = ease(seg(t, .08 + cur * .19 + .05, .08 + cur * .19 + .15));
      ctx.fillStyle = C.acc; ctx.globalAlpha = .35 * a; ctx.fillRect(x0 + (x1 - x0) * j / n, y, (x1 - x0) / n, hh); ctx.globalAlpha = 1;
      T(names[cur], 140, 250, 30, { c: C.mid, w: 500 });
      T("code: " + bits[cur], 140, 560, 76, { c: C.acc });
    }
    const mx = x0 + (x1 - x0) * v; L(mx, y - 40, mx, y + hh, C.off, 4); T("coordinate", mx, y - 52, 24, { al: "center", c: C.off, w: 500 });
    T("top b bits of the 4-bit code are a valid b-bit code", 140, 140, 32, { c: C.off });
    T("idx_b = idx_4 >> (4 − b)", 700, 640, 40, { a: seg(t, .78, .95), al: "left" });
    void idx;
  }

  function s5(t) {
    const sig = [.0541, .0271, .0135, .0068], base = 560, sc = 6200;
    sig.forEach((s, i) => {
      const p = ease(seg(t, .08 + i * .17, .08 + i * .17 + .16)), x = 200 + i * 250, hgt = s * sc * p;
      ctx.fillStyle = i === 3 ? C.acc : C.off; ctx.globalAlpha = .92; ctx.fillRect(x, base - hgt, 150, hgt); ctx.globalAlpha = 1;
      T(`${i + 1} bit${i ? "s" : ""}`, x + 75, base + 44, 28, { al: "center", c: C.mid, w: 500 });
      T(s.toFixed(4), x + 75, base - hgt - 16, 32, { al: "center", a: p });
      if (i > 0) T("÷ 2", x - 52, base - sig[i - 1] * sc + 40, 36, { al: "center", c: C.acc, a: p });
    });
    L(160, base, 1160, base, C.line, 2);
    T("score error σ per bit width", 160, 110, 30, { c: C.mid, w: 500 });
    T("σ_b ≈ C / (√(3P) · 2^b)", 160, 175, 52, { a: seg(t, .7, .88) });
    T("measured: 0.061, 0.027, –, 0.007", 160, 660, 26, { c: C.mid, w: 500, a: seg(t, .9, 1) });
  }

  const r6 = rngf(9), dots = [];
  for (let i = 0; i < 5; i++) dots.push({ s: .8 + r6() * .15, top: 1 });
  for (let i = 0; i < 110; i++) dots.push({ s: clamp(.18 + gauss(r6) * .24, -.2, .77), top: 0 });
  function s6(t) {
    const X = s => 120 + (s + .2) / 1.2 * 1040, y0 = 330, tau = .78, w = .24;
    L(120, y0 + 120, 1160, y0 + 120, C.line, 2);
    ctx.fillStyle = C.acc; ctx.globalAlpha = .13 * seg(t, .3, .45); ctx.fillRect(X(tau - w), 190, X(tau) - X(tau - w), 300); ctx.globalAlpha = 1;
    L(X(tau), 170, X(tau), 500, C.acc, 3, seg(t, .22, .34)); T("τ = 5th best estimate", X(tau), 150, 24, { al: "center", c: C.acc, a: seg(t, .22, .34) });
    dots.forEach((d, i) => {
      const fall = d.top ? 0 : seg(t, .5 + (i % 40) / 40 * .3, .62 + (i % 40) / 40 * .3), jx = ((i * 37) % 11) / 11;
      const y = 230 + jx * 230 + fall * 40;
      ctx.fillStyle = d.top ? C.acc : C.off; ctx.globalAlpha = (d.top ? 1 : .85) * (1 - fall * (d.s < tau - w ? 1 : 0)) * ease(seg(t, 0, .15));
      ctx.beginPath(); ctx.arc(X(d.s), y, d.top ? 9 : 5, 0, 7); ctx.fill(); ctx.globalAlpha = 1;
    });
    T("true neighbours", X(.88), 540, 24, { al: "center", c: C.acc, w: 500 });
    T("window = 2zσ", X(tau - w / 2), 590, 24, { al: "center", c: C.mid, w: 500, a: seg(t, .3, .45) });
    T("dropped: they cannot reach the top", X(.25), 640, 26, { al: "center", c: C.mid, w: 500, a: seg(t, .8, .95) });
    T("keep c  ⇔  ŝ + zσ ≥ τ − zσ", 120, 100, 50, { a: seg(t, .05, .25) });
  }

  function s7(t) {
    const rows = [["1 bit, all vectors", 20000], ["2 bits, survivors", 399], ["4 bits, survivors", 356], ["exact rerank", 107], ["top k", 10]];
    rows.forEach(([lab, n], i) => {
      const p = ease(seg(t, .08 + i * .15, .08 + i * .15 + .17)), w = Math.max(14, Math.log10(n) / Math.log10(20000) * 760) * p, y = 150 + i * 92;
      ctx.fillStyle = i === 4 ? C.acc : C.off; ctx.globalAlpha = .9 - i * .08; ctx.fillRect(110, y, w, 56); ctx.globalAlpha = 1;
      T(n.toLocaleString(), 110 + w + 18, y + 40, 38, { a: p }); T(lab, 110, y - 10, 24, { c: C.mid, w: 500, a: p });
    });
    T("measured, z = 2, 20,000 vectors", 110, 100, 28, { c: C.mid, w: 500 });
    T("1.06 bits read, not 4", 800, 560, 54, { c: C.acc, a: seg(t, .78, .94) });
    T("log scale", 1090, 150, 22, { c: C.dim, w: 500 });
  }

  function s8(t) {
    const n = Math.floor(1 + 11 * ease(seg(t, .08, .6))), pa = 4, pb = 9, X = i => 150 + i * 80;
    const row = (label, y, up, pin) => {
      T(label, 70, y + 36, 24, { c: C.mid, w: 500, al: "right" });
      for (let i = 0; i < 12; i++) {
        const vis = i < up; ctx.fillStyle = pin === i + 1 ? C.acc : C.off; ctx.globalAlpha = vis ? .95 : .12; ctx.fillRect(X(i), y, 66, 52); ctx.globalAlpha = 1;
        T(String(i + 1), X(i) + 33, y + 35, 26, { al: "center", c: C.bg, a: vis ? 1 : 0 });
      }
    };
    row("writer", 190, n, 0);
    row("reader A", 300, Math.min(n, pa) * seg(t, .15, .2) > 0 ? pa : 0, pa);
    row("reader B", 410, n >= pb ? pb : 0, pb);
    T("V(ℓ) = { r : lsn(r) ≤ ℓ }", 150, 590, 54, { a: seg(t, .6, .8) });
    T("same pin, same answer. No torn reads.", 150, 640, 28, { c: C.acc, a: seg(t, .8, .95) });
    T("commits keep arriving; each reader keeps its own view", 150, 130, 28, { c: C.mid, w: 500 });
  }

  function s9(t) {
    const lines = [["1.06 bits per coordinate at z = 3, vs 4 flat", C.off], ["exact top 10 on 1000 of 1000 queries", C.off], ["0.61 vs 0.55 ms in one run, reversed in a re-run: inconclusive", C.acc], ["a live writer adds 21% to median P99", C.acc], ["next: real embeddings, SIMD, sharper bounds", C.mid]];
    T("what we measured", 110, 130, 30, { c: C.mid, w: 500 });
    lines.forEach(([s, c], i) => T(typed(s, seg(t, .08 + i * .16, .08 + i * .16 + .15)), 110, 230 + i * 84, 42, { c }));
    T("synthetic data · one machine · open source", 110, 660, 24, { c: C.dim, w: 500, a: seg(t, .85, 1) });
  }

  const SCENES = [
    { d: 8, f: s1, name: "Angle", cap: "Ranking by inner product is ranking by angle." },
    { d: 11, f: s2, name: "One bit", cap: "A random hyperplane separates two vectors with probability θ over π, so counting disagreeing bits estimates the angle." },
    { d: 10, f: s3, name: "Rotation", cap: "A random rotation makes every vector look alike, so the quantizer needs no training and cannot drift." },
    { d: 11, f: s4, name: "Planes", cap: "Cut each coordinate into nested cells. The top b bits of a 4-bit code are a valid b-bit code." },
    { d: 9, f: s5, name: "Error", cap: "Each extra bit halves the score error, and the error does not depend on the data." },
    { d: 11, f: s6, name: "Gate", cap: "Drop only the vectors whose best possible score cannot reach the top k." },
    { d: 10, f: s7, name: "Funnel", cap: "Measured on synthetic data: 20,000 vectors narrow to 10 while reading about one bit each." },
    { d: 9, f: s8, name: "Snapshot", cap: "Readers pin a log position, so a reader never waits for a writer and never sees half a write." },
    { d: 8, f: s9, name: "Honest", cap: "What was measured, and what has not been." },
  ];
  const starts = []; let total = 0; SCENES.forEach(s => { starts.push(total); total += s.d; });

  let time = 0, playing = false, last = 0, speed = 1, started = false;
  const bar = $("film-seek"), lab = $("film-time"), cap = $("film-cap"), ttl = $("film-title"), btn = $("film-play"), chips = $("film-chips");
  bar.max = String(Math.round(total * 100));
  SCENES.forEach((s, i) => { const b = document.createElement("button"); b.type = "button"; b.textContent = s.name; b.addEventListener("click", () => { time = starts[i]; render(); }); chips.appendChild(b); });
  const fmt = s => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;

  function render() {
    let i = SCENES.length - 1; while (starts[i] > time) i--;
    const sc = SCENES[i], t = clamp((time - starts[i]) / sc.d);
    ctx.fillStyle = C.bg; ctx.fillRect(0, 0, W, H);
    ctx.globalAlpha = Math.min(seg(t, 0, .04) || (t > 0 ? 1 : 0), 1 - seg(t, .96, 1)) || 0; if (t > 0.04 && t < .96) ctx.globalAlpha = 1;
    sc.f(t); ctx.globalAlpha = 1;
    bar.value = String(Math.round(time * 100)); lab.textContent = `${fmt(time)} / ${fmt(total)}`;
    cap.textContent = sc.cap; ttl.textContent = `${i + 1} / ${SCENES.length}  ${sc.name}`;
    Array.from(chips.children).forEach((b, j) => b.setAttribute("aria-current", j === i));
  }
  function tick(now) {
    if (playing) { time += Math.min(.1, (now - last) / 1000) * speed; if (time >= total) { time = total - .001; set(false); } render(); }
    last = now; requestAnimationFrame(tick);
  }
  function set(p) { playing = p; btn.textContent = p ? "Pause" : (time >= total - .01 ? "Replay" : "Play"); }
  btn.addEventListener("click", () => { if (time >= total - .01) time = 0; set(!playing); render(); });
  bar.addEventListener("input", () => { time = +bar.value / 100; set(false); render(); });
  $("film-speed").addEventListener("change", e => { speed = +e.target.value; });
  addEventListener("keydown", e => { if (e.target.closest("input,select,textarea")) return; if (e.key === "k" && cv.dataset.vis === "1") btn.click(); });
  if ("IntersectionObserver" in window) new IntersectionObserver(es => {
    const v = es[0].isIntersecting; cv.dataset.vis = v ? "1" : "0";
    if (v && !started && !reduce) { started = true; set(true); } else if (!v && playing) set(false);
  }, { threshold: .45 }).observe(cv);
  time = 0; render(); set(false); requestAnimationFrame(tick);
})();
