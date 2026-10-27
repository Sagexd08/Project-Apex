/**
 * Ledger: Project Apex's write path as one object. A tray holds eight upright
 * tablets, the write-ahead log, each carrying its sequence number as a dot code.
 * The tablet under the pointer lifts and its neighbours follow, staggered outwards.
 * A dashed plate, the retried write, comes down onto the lifted tablet and is gone
 * on contact: the same write, applied once. The slider is the stagger, in ms.
 */
const {
  Cam, facing, fit, proj, rings, prism, put, solid, seg, unproj,
  tdone, tset, tval, tween, disposer, mk, place, pointer, reflect, register,
} = HL;

const N = 8, PITCH = 14, TH = 7, DEPTH = 46, LIFT = 15, GHOST = 34, FALL = [1, 0.4, 0.14];
const X0 = -9, X1 = (N - 1) * PITCH + TH + 9, Y0 = -9, Y1 = DEPTH + 9;
const hOf = (i) => 20 + i * 2.4;

function mount({ stage, svg, read }, value) {
  const bag = disposer();
  let stag = value;

  const C = Cam(45, 0.5, 1.7);
  fit(C, [[X0, Y0, -5], [X1, Y1, -5], [X1, Y0, 0], [X0, Y1, 0], [X0, Y0, hOf(N - 1) + LIFT + GHOST], [X1, Y1, hOf(N - 1) + LIFT + 4]], 200, 166);
  const P = proj(C), front = facing(C);

  const g = mk("g", {}, svg);
  const [tr, tri] = rings(X0, Y0, X1, Y1, 6, 1.8);
  reflect(svg, g, P, front, tr, -5, 14);
  const tray = solid(g);
  put(tray, prism(P, front, tr, tri, -5, 0));
  tray.sil.classList.add("sil");

  // tablets, painted back to front: ascending x
  const tabs = [];
  for (let i = 0; i < N; i++) {
    const x = i * PITCH, [ring, inner] = rings(x, 0, x + TH, DEPTH, 2.4, 0.9);
    const s = solid(g);
    const code = [];
    for (let k = 0; k < 3; k++) code.push(mk("circle", { r: 1.15, class: "dot " + ((i >> k) & 1 ? "m" : "off") }, g));
    const slot = mk("path", { class: "nf lo" }, g);
    tabs.push({ x, ring, inner, s, code, slot, z: tween(0), last: NaN });
  }
  const ghostEl = mk("path", { class: "nf dash" }, g);
  const ghost = { tw: tween(GHOST), on: -1 };

  function draw(i, lift) {
    const t = tabs[i], h = hOf(i) + lift;
    if (t.last === lift) return;
    t.last = lift;
    put(t.s, prism(P, front, t.ring, t.inner, lift, h));
    const xr = t.x + TH;
    t.code.forEach((el, k) => place(el, P(xr, 12 + k * 5, lift + 6)));
    t.slot.setAttribute("d", seg(P(xr, 8, h - 6), P(xr, DEPTH - 8, h - 6)));
  }

  function drawGhost(now) {
    const a = ghost.on, gz = tval(ghost.tw, now);
    if (a < 0 || gz < 0.8) { ghostEl.setAttribute("d", ""); return; }
    const t = tabs[a], z0 = hOf(a) + tval(t.z, now) + gz;
    ghostEl.setAttribute("d", prism(P, front, t.ring, t.inner, z0, z0 + 3).sil);
  }

  const B = register(stage, (_dt, now) => {
    let moving = !tdone(ghost.tw, now);
    tabs.forEach((t, i) => { draw(i, tval(t.z, now)); if (!tdone(t.z, now)) moving = true; });
    drawGhost(now);
    return moving;
  });
  bag.add(B.unregister);

  // hit: ground plane at the tablets' mean rest height, never the lifted pose
  const MID = hOf(3.5);
  const hit = (p) => {
    const [wx, wy] = unproj(C, p[0], p[1], MID);
    if (wy < Y0 || wy > Y1) return -1;
    const i = Math.round((wx - TH / 2) / PITCH);
    return i < 0 || i >= N ? -1 : i;
  };

  const mark = (i) => tabs.forEach((t, k) => t.s.sil.classList.toggle("hi", k === i));
  let act = -1;
  mark(N - 1);
  read.textContent = "rest";
  function setActive(a) {
    if (a === act) return;
    const now = performance.now(), from = a >= 0 ? a : act;
    act = a;
    tabs.forEach((t, i) => {
      const d = Math.abs(i - from), f = a < 0 ? 0 : (FALL[d] ?? 0) * LIFT;
      tset(t.z, f, now, d * stag);
    });
    mark(a < 0 ? N - 1 : a);
    if (a >= 0) { ghost.on = a; ghost.tw = tween(GHOST); tset(ghost.tw, 0, now, 260); }
    else { ghost.on = -1; }
    read.textContent = a < 0 ? "rest" : "seq " + String(a + 1).padStart(2, "0");
    B.wake();
  }

  bag.add(pointer(stage, { move: (p) => setActive(hit(p)), leave: () => setActive(-1) }));
  bag.add(() => svg.replaceChildren());
  return { set: (v) => { stag = v; }, destroy: bag.dispose };
}

hairline({
  name: "ledger",
  means: "A log of eight tablets: the one you touch lifts, and a retried write lands on it and is applied once.",
  rules: [1, 2, 4, 8],
  range: [0, 40, 90],
  tour: [[140, 150], [205, 175], [265, 205], null],
  mount,
});
