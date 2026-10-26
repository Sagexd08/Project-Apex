// Prism page: progress bar, nav highlight, copy button, and three small labs.
// The labs run a MODEL (Gaussian similarity tail, error formula from the paper). They are not measurements.
(() => {
  "use strict";
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));
  const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;

  // ---- reading progress + nav highlight
  const bar = $("#progress");
  const links = $$(".nav a.nl");
  const secs = links.map(a => $(a.getAttribute("href"))).filter(Boolean);
  const onScroll = () => {
    const h = document.documentElement;
    bar.style.transform = `scaleX(${Math.min(1, h.scrollTop / Math.max(1, h.scrollHeight - h.clientHeight))})`;
    let cur = null;
    for (const s of secs) if (s.getBoundingClientRect().top < innerHeight * 0.4) cur = s.id;
    links.forEach(a => a.classList.toggle("on", a.getAttribute("href") === "#" + cur));
  };
  addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  // ---- copy abstract
  const cp = $("#copy-abs");
  if (cp) cp.addEventListener("click", async () => {
    const txt = $("#abs-text").innerText.trim();
    try { await navigator.clipboard.writeText(txt); cp.querySelector(".odo").textContent = "Copied"; }
    catch { const r = document.createRange(); r.selectNodeContents($("#abs-text")); const s = getSelection(); s.removeAllRanges(); s.addRange(r); cp.querySelector(".odo").textContent = "Selected"; }
    setTimeout(() => (cp.querySelector(".odo").textContent = "Copy abstract"), 1800);
  });

  // ---- seeded rng + gaussian
  function rng(seed) { let a = seed >>> 0; return () => { a = (a + 0x6d2b79f5) >>> 0; let t = a; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; }
  function gauss(r) { let u = 0; while (!u) u = r(); const v = r(); return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v); }

  // ---- lab 1: angle estimator. A random hyperplane meets the plane of q and x in a random line through the origin;
  // it separates them iff the line's direction (mod pi) lies between the two vectors' directions.
  const geo = $("#geo");
  if (geo) {
    const th = $("#theta"), dd = $("#dim");
    const out = { p: $("#o-p"), obs: $("#o-obs"), sd: $("#o-sd"), sc: $("#o-sc"), tv: $("#theta-v") };
    let seed = 11; const nLines = 64;
    const drawGeo = () => {
      const t = +th.value, rad = t * Math.PI / 180, d = +dd.value, R = 130, C = 160;
      const r = rng(seed); let sep = 0, lines = "";
      for (let i = 0; i < nLines; i++) {
        const a = r() * Math.PI, between = a < rad ? 1 : 0;
        sep += between;
        const x1 = C - R * Math.cos(a), y1 = C + R * Math.sin(a), x2 = C + R * Math.cos(a), y2 = C - R * Math.sin(a);
        lines += `<line x1="${x1.toFixed(1)}" y1="${y1.toFixed(1)}" x2="${x2.toFixed(1)}" y2="${y2.toFixed(1)}" stroke="${between ? "#ff9100" : "currentColor"}" stroke-opacity="${between ? 0.9 : 0.18}" stroke-width="${between ? 1.4 : 1}"/>`;
      }
      const xx = C + R * Math.cos(rad), xy = C - R * Math.sin(rad);
      const arc = `M ${C + 40} ${C} A 40 40 0 0 0 ${(C + 40 * Math.cos(rad)).toFixed(1)} ${(C - 40 * Math.sin(rad)).toFixed(1)}`;
      geo.innerHTML = `<circle cx="${C}" cy="${C}" r="${R}" fill="none" stroke="currentColor" stroke-opacity=".35"/>${lines}
        <line x1="${C}" y1="${C}" x2="${C + R}" y2="${C}" stroke="#ff9100" stroke-width="2.4"/>
        <line x1="${C}" y1="${C}" x2="${xx.toFixed(1)}" y2="${xy.toFixed(1)}" stroke="currentColor" stroke-width="2.4"/>
        <path d="${arc}" fill="none" stroke="currentColor"/>
        <text x="${C + R - 4}" y="${C + 18}" text-anchor="end">q</text>
        <text x="${(xx + (xx > C ? 6 : -6)).toFixed(1)}" y="${(xy - 6).toFixed(1)}" text-anchor="${xx > C ? "start" : "end"}">x</text>`;
      const p = rad / Math.PI, sd = Math.PI * Math.sqrt(p * (1 - p) / d);
      out.tv.textContent = t + "°";
      out.p.textContent = p.toFixed(3);
      out.obs.textContent = (sep / nLines).toFixed(3);
      out.sd.textContent = (sd * 180 / Math.PI).toFixed(2) + "°";
      out.sc.textContent = (Math.sin(rad) * sd).toFixed(3);
    };
    th.addEventListener("input", drawGeo); dd.addEventListener("change", drawGeo);
    $("#resample").addEventListener("click", () => { seed = (seed * 7919 + 13) >>> 0; drawGeo(); });
    drawGeo();
  }

  // ---- lab 2: cascade model (errors are modelled as perfectly correlated across stages)
  const cv = $("#cascade");
  if (cv) {
    const N = 4000, K = 10, C = 3, P = 256, SD_S = 0.25;
    const ctx = cv.getContext("2d");
    const zEl = $("#z"), zv = $("#z-v"), segBtns = $$("#sched button"), viewBtns = $$("#stage-view button");
    const ro = { s1: $("#r-s1"), s2: $("#r-s2"), s3: $("#r-s3"), bits: $("#r-bits"), miss: $("#r-miss"), win: $("#r-win") };
    const SCHEDS = { "1-2-4": [1, 2, 4], "1-4": [1, 4], "2-4": [2, 4], "4": [4] };
    let sched = "1-2-4", seed = 5, view = 99;
    const sigma = b => C / (Math.sqrt(3 * P) * Math.pow(2, b));

    function run(r, z, bits, keep) {
      const s = new Float32Array(N), u = new Float32Array(N);
      for (let i = 0; i < N; i++) { s[i] = gauss(r) * SD_S; u[i] = gauss(r); }
      const order = Array.from(s.keys()).sort((a, b) => s[b] - s[a]);
      const top = new Set(order.slice(0, K)), t = s[order[K - 1]];
      let cand = Array.from(s.keys()), inc = 0, pre = 0, prev = 0; const stages = [];
      for (const b of bits) {
        inc += cand.length * (b - prev); pre += cand.length * b; prev = b;
        const sg = sigma(b), est = cand.map(i => s[i] + sg * u[i]);
        const tau = est.slice().sort((a, c) => c - a)[Math.min(K, est.length) - 1];
        const nxt = []; for (let j = 0; j < cand.length; j++) if (est[j] + z * sg >= tau - z * sg) nxt.push(cand[j]);
        stages.push({ after: nxt, tau, sg }); cand = nxt;
      }
      const left = new Set(cand); let miss = 0; top.forEach(i => { if (!left.has(i)) miss = 1; });
      return keep ? { s, top, t, stages, miss, inc: inc / N, pre: pre / N } : { miss, inc: inc / N };
    }

    function draw() {
      const z = +zEl.value, bits = SCHEDS[sched]; zv.textContent = z.toFixed(1);
      const W = cv.width, H = cv.height, m = run(rng(seed), z, bits, true);
      ctx.fillStyle = "#232323"; ctx.fillRect(0, 0, W, H);
      const lo = -0.9, hi = 0.9, X = v => 30 + (v - lo) / (hi - lo) * (W - 60);
      const si = Math.min(view, m.stages.length - 1), stage = m.stages[si], alive = new Set(stage.after);
      const rr = rng(99);
      const wlo = stage.tau - 2 * z * stage.sg;
      ctx.fillStyle = "rgba(255,145,0,.10)"; ctx.fillRect(X(wlo), 20, X(stage.tau) - X(wlo), H - 60);
      ctx.strokeStyle = "rgba(241,238,231,.5)"; ctx.setLineDash([4, 4]); ctx.beginPath(); ctx.moveTo(X(m.t), 20); ctx.lineTo(X(m.t), H - 40); ctx.stroke(); ctx.setLineDash([]);
      for (let i = 0; i < N; i++) {
        const y = 30 + rr() * (H - 90), x = X(m.s[i]);
        if (m.top.has(i)) { ctx.fillStyle = alive.has(i) ? "#ff9100" : "#ff3b30"; ctx.fillRect(x - 3, y - 3, 6, 6); }
        else { ctx.fillStyle = alive.has(i) ? "rgba(241,238,231,.85)" : "rgba(241,238,231,.12)"; ctx.fillRect(x - 1, y - 1, 2, 2); }
      }
      ctx.fillStyle = "#cbcbcb"; ctx.font = "11px 'Geist Mono',monospace";
      ctx.fillText("true similarity to the query →", 30, H - 14); ctx.fillText("dashed: true 10th best", W - 150, H - 14);
      const ss = m.stages.map(x => x.after.length);
      ro.s1.textContent = ss[0].toLocaleString(); ro.s2.textContent = ss[1] != null ? ss[1].toLocaleString() : "–"; ro.s3.textContent = ss[2] != null ? ss[2].toLocaleString() : "–";
      ro.bits.textContent = m.inc.toFixed(2) + " / " + m.pre.toFixed(2);
      ro.win.textContent = (2 * z * stage.sg).toFixed(3);
      const rm = rng(1234); let ms = 0; const T = 100; for (let i = 0; i < T; i++) ms += run(rm, z, bits, false).miss;
      ro.miss.textContent = (100 * ms / T).toFixed(0) + "%";
    }
    zEl.addEventListener("input", draw);
    segBtns.forEach(b => b.addEventListener("click", () => { sched = b.dataset.s; view = 99; segBtns.forEach(x => x.setAttribute("aria-pressed", x === b)); draw(); }));
    viewBtns.forEach(b => b.addEventListener("click", () => { view = +b.dataset.v; viewBtns.forEach(x => x.setAttribute("aria-pressed", x === b)); draw(); }));
    $("#reseed").addEventListener("click", () => { seed = (seed * 48271 + 7) >>> 0; draw(); });
    draw();
  }

  // ---- lab 3: snapshots
  const tl = $("#tl");
  if (tl) {
    const MAX = 14; let lsn = 6, pinA = 3, pinB = 6, auto = null;
    const cells = (upto, pin, fresh) => Array.from({ length: MAX }, (_, i) => {
      const n = i + 1;
      return `<span class="${["cell", n <= upto ? "" : "off", pin === n ? "pin" : "", fresh === n ? "new" : ""].join(" ")}">${n}</span>`;
    }).join("");
    const render = fresh => {
      tl.innerHTML = `
        <div class="row"><span>Writer</span><div class="cells">${cells(lsn, 0, fresh)}</div></div>
        <div class="row"><span>Reader A</span><div class="cells">${cells(pinA, pinA)}</div></div>
        <div class="row"><span>Reader B</span><div class="cells">${cells(pinB, pinB)}</div></div>`;
      $("#tl-lsn").textContent = lsn; $("#tl-a").textContent = pinA; $("#tl-b").textContent = pinB;
    };
    const stop = () => { if (auto) { clearInterval(auto); auto = null; $("#tl-play").querySelector(".odo").textContent = "Play writer"; } };
    const commit = () => { if (lsn < MAX) { lsn++; render(lsn); } else stop(); };
    $("#tl-commit").addEventListener("click", commit);
    $("#tl-repin").addEventListener("click", () => { pinB = lsn; render(); });
    $("#tl-reset").addEventListener("click", () => { stop(); lsn = 6; pinA = 3; pinB = 6; render(); });
    $("#tl-play").addEventListener("click", () => { if (auto) return stop(); if (reduce) return commit(); $("#tl-play").querySelector(".odo").textContent = "Pause writer"; auto = setInterval(commit, 700); });
    render();
  }

  // ================= enhancement layer: reveal, counters, tilt, equation stage, bit rain, plane peel =================
  const html = document.documentElement; html.classList.add("fx");
  const esc = s => s.replace(/[&<>]/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c]));
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

  function count(el) {
    if (!el || reduce || el.dataset.done) return;
    el.dataset.done = 1;
    const to = parseFloat(el.dataset.count), dec = +el.dataset.dec || 0, suf = el.dataset.suf || "", t0 = performance.now(), D = 1500;
    const step = now => {
      const p = clamp((now - t0) / D, 0, 1), e = 1 - Math.pow(1 - p, 3);
      el.textContent = (to * e).toFixed(dec) + suf;
      if (p < 1) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }

  const RV = ".pp h2,.abs-box,.mf article,.vision>div,.lab,.eq,.code,.paper details,.kpi,.tbl,.bars,.note-m,.lim ul";
  const io = "IntersectionObserver" in window ? new IntersectionObserver(es => es.forEach(e => {
    if (!e.isIntersecting) return;
    const t = e.target; t.classList.add("rv-in"); t.classList.remove("rv-pre"); io.unobserve(t);
    if (t.matches(".kpi")) count(t.querySelector("b"));
  }), { rootMargin: "0px 0px -8% 0px", threshold: 0.08 }) : null;

  function tilt(el) {
    if (reduce || !matchMedia("(hover: hover)").matches) return;
    el.addEventListener("pointermove", e => {
      const r = el.getBoundingClientRect(), x = (e.clientX - r.left) / r.width - 0.5, y = (e.clientY - r.top) / r.height - 0.5;
      el.style.transform = `perspective(900px) rotateX(${(-y * 7).toFixed(2)}deg) rotateY(${(x * 9).toFixed(2)}deg)`;
    });
    el.addEventListener("pointerleave", () => { el.style.transform = ""; });
  }

  function scan() {
    $$(RV).forEach(el => {
      if (el.dataset.rv) return; el.dataset.rv = 1;
      if (el.matches(".eq")) { const pre = $("pre", el); if (pre) pre.innerHTML = pre.textContent.split("\n").map((l, i) => `<span class="ln" style="--i:${i}">${esc(l) || " "}</span>`).join(""); }
      if (el.matches(".eq,.kpi")) tilt(el);
      if (reduce || !io) return;
      el.classList.add("rv", "rv-pre");
      el.style.setProperty("--d", Array.prototype.indexOf.call(el.parentNode.children, el) % 5);
      io.observe(el);
    });
  }
  window.prismFx = { scan };
  scan();
  setTimeout(() => $$(".rv-pre").forEach(el => { el.classList.add("rv-in"); el.classList.remove("rv-pre"); $$("b[data-count]", el).forEach(count); }), 6000);

  // equation stage: letters rise in one by one, auto-advance, dots to choose
  const stage = $("#eqstage");
  if (stage) {
    const qs = $$(".eqq", stage), dots = $("#eqdots"); let cur = 0, timer = null;
    const go = i => { cur = i; qs.forEach((q, j) => q.classList.toggle("on", j === i)); $$("button", dots).forEach((b, j) => b.setAttribute("aria-selected", j === i)); };
    const restart = () => { clearInterval(timer); if (!reduce) timer = setInterval(() => go((cur + 1) % qs.length), 3800); };
    qs.forEach((q, i) => {
      const t = document.createElement("span"); t.className = "eqtxt"; t.setAttribute("aria-label", q.dataset.eq);
      Array.from(q.dataset.eq).forEach((c, j) => { const s = document.createElement("span"); s.className = "ch"; s.style.setProperty("--i", j); s.setAttribute("aria-hidden", "true"); s.textContent = c; t.appendChild(s); });
      q.prepend(t);
      const b = document.createElement("button"); b.type = "button"; b.setAttribute("role", "tab"); b.setAttribute("aria-label", "Equation " + (i + 1));
      b.addEventListener("click", () => { go(i); restart(); }); dots.appendChild(b);
    });
    go(0); restart();
  }

  // hero: bit rain + plane peel + parallax variable, one loop that sleeps when the hero is off screen
  const hero = $("#top"), rain = $("#rain"), stackEl = $(".stack");
  const setPeel = v => stackEl && stackEl.style.setProperty("--peel", v.toFixed(3));
  if (hero) {
    const cx = rain ? rain.getContext("2d") : null, FS = 16;
    let W = 0, H = 0, cols = [], vis = true, last = 0;
    const size = () => {
      if (!rain) return;
      const r = rain.getBoundingClientRect(), dpr = Math.min(2, devicePixelRatio || 1);
      rain.width = r.width * dpr; rain.height = r.height * dpr; cx.setTransform(dpr, 0, 0, dpr, 0, 0); W = r.width; H = r.height;
      cols = []; for (let x = 0; x < W; x += FS * 2.2) cols.push({ x, y: Math.random() * H, v: 30 + Math.random() * 70 });
    };
    const paint = () => {
      if (!cx) return;
      cx.clearRect(0, 0, W, H); cx.font = "14px 'Geist Mono',monospace";
      for (const c of cols) for (let k = 0; k < 9; k++) {
        cx.fillStyle = k === 0 ? "rgba(255,145,0,.95)" : `rgba(241,238,231,${(0.42 * (1 - k / 9)).toFixed(3)})`;
        cx.fillText(Math.random() < 0.5 ? "0" : "1", c.x, c.y - k * FS);
      }
    };
    size(); addEventListener("resize", size);
    if (reduce) { paint(); setPeel(0.5); }
    else {
      if (io) new IntersectionObserver(e => { vis = e[0].isIntersecting; }).observe(hero);
      const tick = t => {
        requestAnimationFrame(tick);
        if (!vis || document.hidden) return;
        const prog = clamp(scrollY / Math.max(1, hero.offsetHeight), 0, 1);
        setPeel(0.4 + 0.35 * Math.sin(t / 1400) + 1.5 * prog);
        if (t - last < 66) return; const dt = (t - last) / 1000; last = t;
        for (const c of cols) { c.y += c.v * dt; if (c.y - 9 * FS > H) { c.y = 0; c.v = 30 + Math.random() * 70; } }
        paint();
      };
      requestAnimationFrame(tick);
    }
  }
  addEventListener("scroll", () => html.style.setProperty("--sy", scrollY), { passive: true });
})();
