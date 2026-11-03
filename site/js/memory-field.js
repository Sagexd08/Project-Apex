// Abstract "memory" hero scene: stored vectors drifting as clusters, a query pulse that recalls its
// nearest neighbours, and an afterglow that fades (the trace a recalled memory leaves).
// Canvas 2D, transparent, additive. Same surface as the old spiral: { dive, pause, show }.
const N = 760;
const K = 9;
const CLUSTERS = 7;
const PULSE_MS = 3400;

const rng = (seed) => () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296);
const gauss = (r) => Math.sqrt(-2 * Math.log(r() || 1e-9)) * Math.cos(6.2831853 * r());

export function createMemoryField(host, { ticker = null, reduce = false, centerX = 0.72 } = {}) {
  let canvas = host.querySelector('canvas');
  if (!canvas) { canvas = document.createElement('canvas'); canvas.setAttribute('aria-hidden', 'true'); canvas.className = 'mf-canvas'; host.prepend(canvas); }
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;

  const r = rng(20261030);
  const centres = Array.from({ length: CLUSTERS }, () => [gauss(r) * 0.62, gauss(r) * 0.42, gauss(r) * 0.5]);
  const pts = Array.from({ length: N }, (_, i) => {
    const c = i < N * 0.86 ? centres[i % CLUSTERS] : [0, 0, 0];
    const s = i < N * 0.86 ? 0.17 : 0.9;
    return { x: c[0] + gauss(r) * s, y: c[1] + gauss(r) * s * 0.8, z: c[2] + gauss(r) * s, heat: 0, tw: r() * 6.28 };
  });

  const S = { w: 0, h: 0, dpr: 1, t: 0, zoom: 1, cx: centerX, show: reduce ? 1 : 0, paused: false, on: true, px: 0, py: 0, nextPulse: 600, pulse: null };
  const proj = new Array(N);

  const size = () => {
    const w = host.clientWidth, h = host.clientHeight;
    if (!w || !h) return;
    S.dpr = Math.min(devicePixelRatio || 1, 2);
    S.w = w; S.h = h;
    canvas.width = Math.round(w * S.dpr); canvas.height = Math.round(h * S.dpr);
    canvas.style.width = '100%'; canvas.style.height = '100%';
    draw();
  };

  const project = () => {
    const a = S.t * 0.00006 + S.px * 0.25, ca = Math.cos(a), sa = Math.sin(a);
    const b = 0.25 + S.py * 0.12, cb = Math.cos(b), sb = Math.sin(b);
    const scale = Math.min(S.w, S.h) * 0.62 * S.zoom;
    for (let i = 0; i < N; i++) {
      const p = pts[i];
      const x1 = p.x * ca + p.z * sa, z1 = -p.x * sa + p.z * ca;
      const y2 = p.y * cb - z1 * sb, z2 = p.y * sb + z1 * cb;
      const f = 1 / (1.9 - z2 * 0.7);
      proj[i] = { x: S.w * S.cx + x1 * scale * f * 1.7, y: S.h * 0.5 + y2 * scale * f * 1.7, z: z2, f };
    }
  };

  const startPulse = (near) => {
    let o = near;
    if (o == null) o = (Math.random() * N) | 0;
    const q = proj[o];
    const d = [];
    for (let i = 0; i < N; i++) { const dx = proj[i].x - q.x, dy = proj[i].y - q.y; d.push([dx * dx + dy * dy, i]); }
    d.sort((a, b) => a[0] - b[0]);
    S.pulse = { o, t0: S.t, hits: d.slice(1, K + 1).map((e) => ({ i: e[1], r: Math.sqrt(e[0]) })) };
  };

  const draw = () => {
    if (!S.w) return;
    ctx.setTransform(S.dpr, 0, 0, S.dpr, 0, 0);
    ctx.clearRect(0, 0, S.w, S.h);
    ctx.globalCompositeOperation = 'lighter';
    project();
    const sh = S.show, base = Math.min(S.w, S.h);

    // faint nested planes: four stacked bit planes behind the cloud
    ctx.lineWidth = 1;
    for (let k = 0; k < 4; k++) {
      const y = S.h * (0.5 + (k - 1.5) * 0.17), w = base * (0.9 - k * 0.04);
      const cx = S.w * S.cx;
      ctx.strokeStyle = `rgba(160,205,255,${0.06 * sh})`;
      ctx.beginPath(); ctx.moveTo(cx - w, y + 24); ctx.lineTo(cx - w * 0.55, y - 30); ctx.lineTo(cx + w, y - 30); ctx.lineTo(cx + w * 0.55, y + 24); ctx.closePath(); ctx.stroke();
    }

    const pl = S.pulse, age = pl ? S.t - pl.t0 : 0;
    if (pl) {
      const q = proj[pl.o], rad = (age / 1400) * base * 0.55;
      const fade = Math.max(0, 1 - age / 2600);
      ctx.strokeStyle = `rgba(255,154,122,${0.55 * fade * sh})`; ctx.lineWidth = 1.5;
      ctx.beginPath(); ctx.arc(q.x, q.y, Math.max(1, rad), 0, 6.2832); ctx.stroke();
      ctx.strokeStyle = `rgba(255,154,122,${0.25 * fade * sh})`;
      ctx.beginPath(); ctx.arc(q.x, q.y, Math.max(1, rad * 0.62), 0, 6.2832); ctx.stroke();
      for (const h of pl.hits) {
        if (rad >= h.r) {
          pts[h.i].heat = 1;
          const p = proj[h.i], reach = Math.min(1, (rad - h.r) / 60);
          ctx.strokeStyle = `rgba(255,170,140,${0.5 * fade * reach * sh})`; ctx.lineWidth = 1;
          ctx.beginPath(); ctx.moveTo(q.x, q.y); ctx.lineTo(q.x + (p.x - q.x) * reach, q.y + (p.y - q.y) * reach); ctx.stroke();
        }
      }
    }

    for (let i = 0; i < N; i++) {
      const p = proj[i], m = pts[i];
      const depth = (p.z + 1.2) / 2.4, tw = 0.75 + 0.25 * Math.sin(S.t * 0.0017 + m.tw);
      const size = (0.9 + depth * 1.7) * p.f * 1.6 * (1 + m.heat * 1.6);
      if (m.heat > 0.02) {
        ctx.fillStyle = `rgba(255,${130 + (1 - m.heat) * 90 | 0},${120 + (1 - m.heat) * 110 | 0},${(0.25 + m.heat * 0.7) * sh})`;
        ctx.beginPath(); ctx.arc(p.x, p.y, size * 2.2, 0, 6.2832); ctx.fill();
      }
      ctx.fillStyle = `rgba(${190 + depth * 60 | 0},${215 + depth * 40 | 0},255,${(0.22 + depth * 0.55) * tw * sh})`;
      ctx.beginPath(); ctx.arc(p.x, p.y, size, 0, 6.2832); ctx.fill();
    }
    ctx.globalCompositeOperation = 'source-over';
  };

  let last = 0;
  const step = (time) => {
    const dt = last ? Math.min(64, time - last) : 16; last = time;
    if (S.paused || !S.on || document.hidden) return;
    S.t += dt;
    for (const p of pts) p.heat *= Math.pow(0.9993, dt);
    if (!reduce) {
      S.nextPulse -= dt;
      if (S.nextPulse <= 0 && S.w) { project(); startPulse(); S.nextPulse = PULSE_MS; }
      if (S.pulse && S.t - S.pulse.t0 > 3200) S.pulse = null;
    }
    draw();
  };

  if (reduce) { size(); project(); startPulse(centres.length); S.pulse.t0 = -3000; S.t = 0; pts.slice(0, 40).forEach((p, i) => { p.heat = i % 7 === 0 ? 0.8 : 0; }); draw(); }
  if (ticker) ticker.add((time) => step(time * 1000)); else if (!reduce) { const loop = (t) => { step(t); requestAnimationFrame(loop); }; requestAnimationFrame(loop); }

  new ResizeObserver(size).observe(host);
  new IntersectionObserver((e) => { S.on = e[0].isIntersecting; }).observe(host);
  host.addEventListener('pointermove', (e) => {
    const b = host.getBoundingClientRect();
    S.px = ((e.clientX - b.left) / b.width - 0.5) * 2; S.py = ((e.clientY - b.top) / b.height - 0.5) * 2;
  }, { passive: true });
  host.addEventListener('pointerdown', (e) => {
    if (reduce || !S.w) return;
    const b = host.getBoundingClientRect(), x = e.clientX - b.left, y = e.clientY - b.top;
    let best = 0, bd = 1e12;
    for (let i = 0; i < N; i++) { const dx = proj[i].x - x, dy = proj[i].y - y, d = dx * dx + dy * dy; if (d < bd) { bd = d; best = i; } }
    startPulse(best);
  }, { passive: true });
  size();

  return {
    dive(p) { S.zoom = 1 + p * 1.55; S.cx = centerX - p * 0.22; },
    pause(v) { S.paused = v; },
    show(v) { S.show = v; if (reduce) draw(); },
    uniforms: {},
  };
}
