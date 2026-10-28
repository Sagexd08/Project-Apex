// ASCII dither dividers (scroll-scrubbed) and the rotating ASCII globe. Both draw only when visible.
const GLYPHS = '01ABCDEF/\\<>[]{}()=+-*_.:#%&$@';
const CELL_W = 11, CELL_H = 15;

const hash = (x, y) => {
  let h = (x * 374761393 + y * 668265263) | 0;
  h = (h ^ (h >>> 13)) * 1274126177;
  return ((h ^ (h >>> 16)) >>> 0) / 4294967295;
};
const vnoise = (x) => {
  const i = Math.floor(x), f = x - i, s = f * f * (3 - 2 * f);
  return hash(i, 7) * (1 - s) + hash(i + 1, 7) * s;
};

/** One divider: canvas fills `from` colour; as p goes 0->1 the `to` colour pours in with a glyph-noise frontier. */
export function createDivider(canvas, from, to) {
  const ctx = canvas.getContext('2d', { alpha: false });
  let w = 0, h = 0, cols = 0, rows = 0, last = -1, dpr = 1;

  function size() {
    dpr = Math.min(window.devicePixelRatio || 1, 2);
    w = canvas.clientWidth; h = canvas.clientHeight;
    canvas.width = Math.round(w * dpr); canvas.height = Math.round(h * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    cols = Math.ceil(w / CELL_W); rows = Math.ceil(h / CELL_H);
    last = -1;
  }

  function draw(p) {
    if (Math.abs(p - last) < 0.003) return;
    last = p;
    ctx.fillStyle = from; ctx.fillRect(0, 0, w, h);
    ctx.font = `500 ${CELL_H - 3}px "Geist Mono", monospace`;
    ctx.textBaseline = 'middle'; ctx.textAlign = 'center';
    const flick = Math.floor(p * 60);
    const base = h * 0.5 + (p - 0.5) * h * 0.7; /* frontier stays inside the strip and drifts with scroll */
    for (let i = 0; i < cols; i++) {
      const front = base + (vnoise(i * 0.09) - 0.5) * 110 + (vnoise(i * 0.31 + 40) - 0.5) * 28;
      for (let j = 0; j < rows; j++) {
        const d = j * CELL_H + CELL_H / 2 - front; // >0: inside `to`
        const r = hash(i, j);
        const cx = i * CELL_W + CELL_W / 2, cy = j * CELL_H + CELL_H / 2;
        if (d > 0) {
          ctx.fillStyle = to; ctx.fillRect(i * CELL_W, j * CELL_H, CELL_W + 0.5, CELL_H + 0.5);
          if (d < 46 && r < (1 - d / 46) * 0.55) {
            ctx.fillStyle = from; ctx.fillText(GLYPHS[(Math.floor(r * 997) + flick) % GLYPHS.length], cx, cy);
          }
        } else if (d > -70) {
          const k = 1 + d / 70;
          if (d > -15 && r < 0.45) { ctx.fillStyle = to; ctx.fillRect(i * CELL_W, j * CELL_H, CELL_W + 0.5, CELL_H + 0.5); }
          else if (r < k * 0.5) { ctx.fillStyle = to; ctx.fillText(GLYPHS[(Math.floor(r * 811) + flick) % GLYPHS.length], cx, cy); }
        }
      }
    }
  }

  size();
  new ResizeObserver(size).observe(canvas);
  return { draw };
}

/** Rotating ASCII globe on a canvas; throttled to ~14fps, paused off-screen. */
export function createGlobe(canvas, ticker, reduce) {
  const ctx = canvas.getContext('2d');
  const RAMP = ' .·:-=+*#%@';
  let w = 0, h = 0, dpr = 1, cols = 0, rows = 0, visible = false, acc = 0, spin = 0;
  const cw = 10, ch = 14;

  function size() {
    dpr = Math.min(window.devicePixelRatio || 1, 2);
    w = canvas.clientWidth; h = canvas.clientHeight;
    canvas.width = Math.round(w * dpr); canvas.height = Math.round(h * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    cols = Math.floor(w / cw); rows = Math.floor(h / ch);
    draw();
  }
  // crude continents: sum of lat/lon waves above a threshold
  const land = (lon, lat) => Math.sin(lon * 2 + 1.3) * Math.cos(lat * 3) + Math.sin(lon * 5 - lat * 2) * 0.45 + Math.cos(lon * 3 + lat * 4) * 0.35;

  function draw() {
    ctx.clearRect(0, 0, w, h);
    ctx.font = `${ch - 2}px "Geist Mono", monospace`;
    ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
    const R = Math.min(cols * cw * 0.5, rows * ch * 0.5) * 0.96;
    const cx = cols * cw / 2, cy = rows * ch / 2;
    for (let j = 0; j < rows; j++) {
      for (let i = 0; i < cols; i++) {
        const x = (i * cw + cw / 2 - cx) / R, y = (j * ch + ch / 2 - cy) / R;
        const rr = x * x + y * y;
        if (rr > 1) continue;
        const z = Math.sqrt(1 - rr);
        const lat = Math.asin(-y), lon = Math.atan2(x, z) + spin;
        const l = land(lon, lat);
        if (l < 0.12) { if (rr > 0.94) { ctx.fillStyle = 'rgba(239,230,211,.18)'; ctx.fillText('·', i * cw + cw / 2, j * ch + ch / 2); } continue; }
        const b = Math.min(1, z * 0.85 + (l - 0.12) * 0.25);
        const g = RAMP[Math.min(RAMP.length - 1, Math.floor(b * RAMP.length))];
        ctx.fillStyle = `rgba(239,230,211,${(0.12 + b * 0.5).toFixed(2)})`;
        ctx.fillText(g, i * cw + cw / 2, j * ch + ch / 2);
      }
    }
  }

  new ResizeObserver(size).observe(canvas);
  new IntersectionObserver((e) => { visible = e[0].isIntersecting; }).observe(canvas);
  if (!reduce) {
    ticker.add((_t, dt) => {
      if (!visible) return;
      acc += dt;
      if (acc < 70) return; // ~14fps
      spin += acc * 0.00026; acc = 0; draw();
    });
  }
  size();
}
