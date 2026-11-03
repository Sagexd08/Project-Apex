/* Atmosphere + interactive cubes for index.html slab mode (module). Monochrome by design: everything outside the
   slab is black / white / grey. Decoration only (canvas aria-hidden, pointer-events none); hits come from a raycast on
   window pointer events, and the same facts are the #facts button list.
   Technique notes (see docs/ui/BUILD_NOTES.md): DOM carries content, WebGL only atmosphere; camera rides a
   CatmullRomCurve3 sampled by scroll progress with sway/roll from scroll velocity; side props are GPU-instanced line
   meshes recycled by a modulo in the vertex shader (no per-frame CPU work); speed streaks scale with velocity.
   Draw calls: 8 (boxes, trees, arches, rings, streaks, cube faces, cube edges, network). DPR <= 1.5; quality drops by itself. */
import * as THREE from "../vendor/three.module.min.js";

(function () {
  var W = window, root = document.documentElement, facts = W.slabFacts;
  if (!root.classList.contains("slab-on") || !facts || !facts.items.length) return;
  var canvas = document.createElement("canvas");
  canvas.className = "slab-cv"; canvas.setAttribute("aria-hidden", "true"); document.body.appendChild(canvas);
  var renderer;
  try { renderer = new THREE.WebGLRenderer({ canvas: canvas, antialias: true, alpha: false }); } catch (e) { canvas.remove(); return; }
  renderer.setClearColor(0x000000, 1);
  root.classList.add("slab-cubes");

  var FOV = 55, CZ = 12, MAXC = 6, RANGE = 110, TRAVEL = 900, N = facts.items.length;
  var scene = new THREE.Scene(), cam = new THREE.PerspectiveCamera(FOV, 1, 0.1, 160);
  var seed = 5; function rnd() { seed = (seed * 16807) % 2147483647; return seed / 2147483647; }
  var dpr = Math.min(devicePixelRatio || 1, 1), tierDown = false;

  var VS = "attribute vec3 iO;attribute vec3 iS;uniform float uZ,uR,uWrap,uA,uLen,uStreak;varying float vA;" +
    "void main(){vec3 p=position*iS;float z=uWrap>.5?mod(iO.z+uZ,uR)-(uR-8.):iO.z+uZ;" +
    "if(uStreak>.5){p=vec3(iO.xy,position.z*uLen);z=mod(iO.z+uZ*1.6,uR)-(uR-8.);}else p.xy+=iO.xy;p.z+=z;" +
    "vA=uA*(1.-smoothstep(uR*.5,uR-10.,-z))*(1.-smoothstep(-3.,1.5,z))*step(-(uR-8.),z);" +
    "gl_Position=projectionMatrix*modelViewMatrix*vec4(p,1.);}";
  var FS = "varying float vA;void main(){gl_FragColor=vec4(vec3(1.),vA);}";
  var GARDEN_VS = "attribute float aZ,aStatic,aRise,aAlpha;uniform float uZ,uR,uA,uT;varying float vA;" +
    "void main(){vec3 p=position;float pz=aStatic>.5?0.:mod(aZ+uZ,uR)-(uR-8.);" +
    "if(aRise>=0.)p.y+=mod(aRise+uT*1.6,16.)-4.;float z=pz+p.z;" +
    "vA=uA*aAlpha*(1.-smoothstep(uR*.5,uR-10.,-z))*(1.-smoothstep(-3.,1.5,z));" +
    "gl_Position=projectionMatrix*modelViewMatrix*vec4(p.xy,z,1.);}";
  var kinds = [];
  function kind(base, count, gen, o) {
    var g = new THREE.InstancedBufferGeometry(); g.setAttribute("position", new THREE.Float32BufferAttribute(base, 3));
    var io = new Float32Array(count * 3), is = new Float32Array(count * 3);
    for (var i = 0; i < count; i++) { var r = gen(i); io.set([r[0], r[1], r[2]], i * 3); is.set([r[3], r[4], r[5]], i * 3); }
    g.setAttribute("iO", new THREE.InstancedBufferAttribute(io, 3)); g.setAttribute("iS", new THREE.InstancedBufferAttribute(is, 3));
    g.instanceCount = count;
    var m = new THREE.ShaderMaterial({ vertexShader: VS, fragmentShader: FS, transparent: true, depthWrite: false,
      uniforms: { uZ: { value: 0 }, uR: { value: RANGE }, uWrap: { value: o.wrap ? 1 : 0 }, uA: { value: o.a }, uLen: { value: 1 }, uStreak: { value: o.streak ? 1 : 0 } } });
    var l = new THREE.LineSegments(g, m); l.frustumCulled = false; scene.add(l);
    kinds.push({ l: l, m: m, g: g, count: count, o: o }); return kinds[kinds.length - 1];
  }
  var E = [[0, 1], [2, 3], [4, 5], [6, 7], [0, 2], [1, 3], [4, 6], [5, 7], [0, 4], [1, 5], [2, 6], [3, 7]];
  function cc(i) { return [i & 1 ? 0.5 : -0.5, i & 2 ? 0.5 : -0.5, i & 4 ? 0.5 : -0.5]; }

  var GROUND = -4, PATH_HALF = 2.6, TAU = Math.PI * 2;
  var garden = { pos: [], z: [], stat: [], rise: [], alpha: [] };
  function addSegment(a, b, anchor, alpha, isStatic, rise) {
    garden.pos.push(a[0], a[1], a[2], b[0], b[1], b[2]);
    for (var v = 0; v < 2; v++) { garden.z.push(anchor); garden.stat.push(isStatic ? 1 : 0); garden.rise.push(rise === undefined ? -1 : rise); garden.alpha.push(alpha); }
  }
  function addBox(cx, cy, cz, w, h, d, anchor, alpha, rise) {
    E.forEach(function (e) {
      var p = cc(e[0]), q = cc(e[1]);
      addSegment([cx + p[0] * w, cy + p[1] * h, cz + p[2] * d], [cx + q[0] * w, cy + q[1] * h, cz + q[2] * d], anchor, alpha, false, rise);
    });
  }
  function addRing(cx, cy, cz, r, steps, plane, anchor, alpha) {
    for (var s = 0; s < steps; s++) {
      var a0 = s / steps * TAU, a1 = (s + 1) / steps * TAU;
      var u0 = Math.cos(a0) * r, v0 = Math.sin(a0) * r, u1 = Math.cos(a1) * r, v1 = Math.sin(a1) * r;
      var p = plane === "h" ? [cx + u0, cy, cz + v0] : plane === "x" ? [cx + u0, cy + v0, cz] : [cx, cy + u0, cz + v0];
      var q = plane === "h" ? [cx + u1, cy, cz + v1] : plane === "x" ? [cx + u1, cy + v1, cz] : [cx, cy + u1, cz + v1];
      addSegment(p, q, anchor, alpha, false);
    }
  }
  function addCone(cx, y0, cz, r, h, sides, anchor, alpha) {
    addRing(cx, y0, cz, r, sides, "h", anchor, alpha);
    for (var s = 0; s < sides; s += 2) {
      var a = s / sides * TAU;
      addSegment([cx + Math.cos(a) * r, y0, cz + Math.sin(a) * r], [cx, y0 + h, cz], anchor, alpha, false);
    }
  }
  function addPine(x, anchor, height) {
    addSegment([x, GROUND, 0], [x, GROUND + height * 0.3, 0], anchor, 0.5, false);
    [[0.2, 0.55, 0.5], [0.4, 0.8, 0.38], [0.62, 1, 0.26]].forEach(function (tier) {
      addCone(x, GROUND + height * tier[0], 0, height * tier[2] * 0.7, height * (tier[1] - tier[0]), 8, anchor, 0.55);
    });
  }
  function addOak(x, anchor, height) {
    var crownY = GROUND + height * 0.68, crownR = height * 0.3;
    addSegment([x, GROUND, 0], [x, crownY - crownR * 0.6, 0], anchor, 0.5, false);
    addRing(x, crownY, 0, crownR, 14, "h", anchor, 0.55);
    addRing(x, crownY, 0, crownR, 14, "x", anchor, 0.55);
    addRing(x, crownY, 0, crownR, 14, "z", anchor, 0.55);
  }
  function addLamp(x, anchor) {
    var top = GROUND + 7, toward = x > 0 ? -1 : 1;
    addSegment([x, GROUND, 0], [x, top, 0], anchor, 0.6, false);
    addSegment([x, top, 0], [x + toward * 1.1, top, 0], anchor, 0.6, false);
    addBox(x + toward * 1.1, top - 0.35, 0, 0.5, 0.7, 0.5, anchor, 0.7);
  }
  function addBench(x, anchor) {
    var s = x > 0 ? 1 : -1;
    addBox(x, GROUND + 0.9, 0, 0.7, 0.12, 2.4, anchor, 0.55);
    addSegment([x, GROUND, -1.1], [x, GROUND + 0.9, -1.1], anchor, 0.55, false);
    addSegment([x, GROUND, 1.1], [x, GROUND + 0.9, 1.1], anchor, 0.55, false);
    addBox(x + s * 0.35, GROUND + 1.6, 0, 0.1, 1.2, 2.4, anchor, 0.55);
  }
  function addFence(x, anchor, length) {
    var h = 1.6;
    addSegment([x, GROUND, -length / 2], [x, GROUND + h, -length / 2], anchor, 0.45, false);
    addSegment([x, GROUND + h * 0.3, -length / 2], [x, GROUND + h * 0.3, length / 2], anchor, 0.45, false);
    addSegment([x, GROUND + h * 0.8, -length / 2], [x, GROUND + h * 0.8, length / 2], anchor, 0.45, false);
  }
  var strip = RANGE / 18;
  addSegment([-PATH_HALF, GROUND, 4], [-PATH_HALF, GROUND, -RANGE * 1.4], 0, 0.8, true);
  addSegment([PATH_HALF, GROUND, 4], [PATH_HALF, GROUND, -RANGE * 1.4], 0, 0.8, true);
  addSegment([-PATH_HALF - 0.15, GROUND, 4], [-PATH_HALF - 0.15, GROUND, -RANGE * 1.4], 0, 0.4, true);
  addSegment([PATH_HALF + 0.15, GROUND, 4], [PATH_HALF + 0.15, GROUND, -RANGE * 1.4], 0, 0.4, true);
  var crossCount = 55;
  for (var cr = 0; cr < crossCount; cr++) {
    var crossZ = cr * RANGE / crossCount;
    addSegment([-PATH_HALF, GROUND, 0], [PATH_HALF, GROUND, 0], crossZ, 0.55, false);
    addSegment([-48, GROUND, 0], [-PATH_HALF - 1, GROUND, 0], crossZ, 0.2, false);
    addSegment([PATH_HALF + 1, GROUND, 0], [48, GROUND, 0], crossZ, 0.2, false);
  }
  for (var ti = 0; ti < 14; ti++) {
    var treeZ = ti * RANGE / 14;
    addPine(-7.5, treeZ + 1, 9 + (ti % 3) * 2); addPine(7.5, treeZ + 4, 9 + ((ti + 1) % 3) * 2);
    addOak(-12.5, treeZ + 6, 11 + (ti % 4)); addOak(12.5, treeZ + 2, 11 + ((ti + 2) % 4));
  }
  for (var hi = 0; hi < 18; hi++) {
    addBox(-5.2, GROUND + 0.8, 0, 0.9, 1.6, strip - 0.15, hi * strip, 0.6);
    addBox(5.2, GROUND + 0.8, 0, 0.9, 1.6, strip - 0.15, hi * strip, 0.6);
  }
  var coreVertexCount = garden.pos.length / 3;
  for (var fi = 0; fi < 22; fi++) { addFence(-9.2, fi * RANGE / 22, RANGE / 22); addFence(9.2, fi * RANGE / 22, RANGE / 22); }
  for (var li = 0; li < 8; li++) { addLamp(li % 2 ? 4.2 : -4.2, li * RANGE / 8); addLamp(li % 2 ? -4.2 : 4.2, li * RANGE / 8 + RANGE / 16); }
  for (var bi = 0; bi < 4; bi++) addBench(bi % 2 ? 3.4 : -3.4, bi * RANGE / 4 + 9);
  for (var ln = 0; ln < 36; ln++) {
    var lanternSide = rnd() < 0.5 ? -1 : 1;
    addBox(lanternSide * (3.5 + rnd() * 10), 0, (rnd() - 0.5) * 3, 0.5, 0.6, 0.5, rnd() * RANGE, 0.85, rnd() * 16);
  }
  var gardenGeometry = new THREE.BufferGeometry();
  gardenGeometry.setAttribute("position", new THREE.Float32BufferAttribute(garden.pos, 3));
  gardenGeometry.setAttribute("aZ", new THREE.Float32BufferAttribute(garden.z, 1));
  gardenGeometry.setAttribute("aStatic", new THREE.Float32BufferAttribute(garden.stat, 1));
  gardenGeometry.setAttribute("aRise", new THREE.Float32BufferAttribute(garden.rise, 1));
  gardenGeometry.setAttribute("aAlpha", new THREE.Float32BufferAttribute(garden.alpha, 1));
  var gardenMaterial = new THREE.ShaderMaterial({ vertexShader: GARDEN_VS, fragmentShader: FS, transparent: true, depthWrite: false,
    uniforms: { uZ: { value: 0 }, uR: { value: RANGE }, uA: { value: 0.9 }, uT: { value: 0 } } });
  var gardenLines = new THREE.LineSegments(gardenGeometry, gardenMaterial); gardenLines.frustumCulled = false; scene.add(gardenLines);

  var arch = [-0.5, 0, 0, -0.5, 1, 0, 0.5, 0, 0, 0.5, 1, 0, -0.5, 1, 0, 0.5, 1, 0, -0.5, 0.86, 0, 0.5, 0.86, 0, -0.5, 0, 0, -0.5, 0.0, -0.05];
  var line2 = [0, 0, 0, 0, 0, 1];
  var archKind = kind(arch, 12, function () { return [0, -4, -999, 30, 15, 1]; }, { wrap: false, a: 0.5 });
  var streaks = kind(line2, 140, function () { var a = rnd() * 6.2832, r = 3 + rnd() * 14; return [Math.cos(a) * r * 1.6, Math.sin(a) * r * 0.7, rnd() * RANGE, 1, 1, 1]; }, { wrap: true, a: 0, streak: true });

  /* arches at every chapter boundary (no wrap: each is flown through once) */
  function placeArches() {
    var rw = W.slabRunway, io = archKind.g.attributes.iO, c = 0;
    if (!rw || !rw.S || !rw.total) return;
    var maxS = Math.max(1, rw.max);
    for (var k = 0; k < rw.S.length && c < 12; k++, c++) {
      var pb = Math.min(1, Math.max(0, (rw.top0 + rw.S[k] - 0) / (maxS + innerHeight)));
      io.setXYZ(c, 0, -4, 99 - pb * TRAVEL);
    }
    for (; c < 12; c++) io.setXYZ(c, 0, -4, -999);
    io.needsUpdate = true; archKind.g.instanceCount = 12;
  }

  /* ---- camera path ---- */
  var pts = []; for (i = 0; i < 16; i++) pts.push(new THREE.Vector3(Math.sin(i * 1.3) * 0.9, Math.cos(i * 0.9) * 0.45, 0));
  var path = new THREE.CatmullRomCurve3(pts), tmp = new THREE.Vector3(), tan = new THREE.Vector3();

  /* ---- interactive cubes (monochrome) ---- */
  var inst = new THREE.InstancedMesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshBasicMaterial({ transparent: true, opacity: 0.3, depthWrite: false }), N);
  inst.frustumCulled = false; scene.add(inst);
  var CORN = []; for (i = 0; i < 8; i++) CORN.push(new THREE.Vector3().fromArray(cc(i)));
  var COPIES = [1, 1.03, 1.06], VPC = 24 * COPIES.length;
  var ep = new Float32Array(N * VPC * 3), ec = new Float32Array(N * VPC * 3), eg = new THREE.BufferGeometry();
  eg.setAttribute("position", new THREE.BufferAttribute(ep, 3)); eg.setAttribute("color", new THREE.BufferAttribute(ec, 3));
  var edges = new THREE.LineSegments(eg, new THREE.LineBasicMaterial({ vertexColors: true })); edges.frustumCulled = false; scene.add(edges);
  var np = new Float32Array(MAXC * 2 * 3), ng = new THREE.BufferGeometry();
  ng.setAttribute("position", new THREE.BufferAttribute(np, 3)); ng.setDrawRange(0, 0);
  var net = new THREE.LineSegments(ng, new THREE.LineBasicMaterial({ color: 0xffffff })); net.frustumCulled = false; scene.add(net);

  var cubes = [], collected = [], hover = -1, drag = null, pulse = [], hw, hh, slabFrac;
  for (i = 0; i < N; i++) cubes.push({
    p: new THREE.Vector3(), q: new THREE.Quaternion().setFromEuler(new THREE.Euler(rnd() * 3, rnd() * 3, rnd() * 3)),
    spin: new THREE.Vector3(0.1, 0.2, 0), base: 0.6 + rnd() * 0.45, sc: 1, vy: 0.18 + rnd() * 0.25, live: facts.items[i].tag === "live", glow: 0, held: false });
  function spawn(k, anywhere) {
    var cb = cubes[k], sd = k % 2 ? 1 : -1, lo = slabFrac + 0.05, hi = 0.95;
    cb.p.set(sd * hw * (lo + rnd() * Math.max(0.02, hi - lo)), anywhere ? (rnd() - 0.5) * 2 * hh : -hh - 1, -CZ + rnd() * 2 - 1);
  }
  function size() {
    var w = innerWidth, h = innerHeight;
    renderer.setPixelRatio(dpr); renderer.setSize(w, h, false);
    cam.aspect = w / h; cam.updateProjectionMatrix();
    hh = CZ * Math.tan(FOV * Math.PI / 360); hw = hh * cam.aspect;
    slabFrac = Math.min(0.9, Math.max(640, Math.min(1280, 0.64 * w)) / w);
    cubes.forEach(function (cb, k) { if (!cb.held) spawn(k, true); });
    placeArches();
  }
  addEventListener("resize", size); size();

  var m = new THREE.Matrix4(), one = new THREE.Vector3(), v = new THREE.Vector3(), col = new THREE.Color(), sv = new THREE.Vector3(), qd = new THREE.Quaternion();
  function updateCubes(dt) {
    for (var k = 0; k < N; k++) {
      var cb = cubes[k], held = collected.indexOf(k) >= 0, isDrag = drag && drag.i === k;
      if (!cb.held) { cb.p.y += cb.vy * dt; if (cb.p.y > hh + 1) spawn(k, false); }
      if (!isDrag) {
        sv.copy(cb.spin).multiplyScalar(dt); var ang = sv.length();
        if (ang > 0) cb.q.premultiply(qd.setFromAxisAngle(sv.normalize(), ang));
        cb.spin.multiplyScalar(Math.pow(0.35, dt));
        if (cb.spin.lengthSq() < 0.02) cb.spin.set(0.1 + (k % 3) * 0.05, 0.2 - (k % 2) * 0.4, 0);
      }
      cb.sc += (((k === hover || isDrag) ? 1.3 : 1) * (held ? 1.15 : 1) - cb.sc) * Math.min(1, dt * 10);
      cb.glow += (((k === hover) || pulse[k] > 0 || held ? 1 : 0) - cb.glow) * Math.min(1, dt * 8);
      if (pulse[k] > 0) pulse[k] -= dt;
      var s = cb.base * cb.sc, g = cb.glow, face = (cb.live ? 0.5 : 0.22) + g * 0.5, edge = 0.5 + g * 0.5;
      m.compose(cb.p, cb.q, one.set(s, s, s)); inst.setMatrixAt(k, m);
      inst.setColorAt(k, col.setRGB(face, face, face));
      for (var c = 0; c < COPIES.length; c++) {
        var sc = COPIES[c], br = c === 0 ? edge : g * (c === 1 ? 0.8 : 0.55); // extra copies only show on hover/select: thicker white line
        for (var e = 0; e < 12; e++) for (var j = 0; j < 2; j++) {
          v.copy(CORN[E[e][j]]).multiplyScalar(sc).applyMatrix4(m);
          var o = (((k * COPIES.length + c) * 12 + e) * 2 + j) * 3; ep[o] = v.x; ep[o + 1] = v.y; ep[o + 2] = v.z; ec[o] = ec[o + 1] = ec[o + 2] = br;
        }
      }
    }
    inst.instanceMatrix.needsUpdate = true; if (inst.instanceColor) inst.instanceColor.needsUpdate = true;
    eg.attributes.position.needsUpdate = true; eg.attributes.color.needsUpdate = true;
    var segs = 0, cl = collected.length;
    for (var a = 0; a < cl && cl > 1; a++) {
      var b = (a + 1) % cl; if (cl < 3 && b === 0) break;
      var pa = cubes[collected[a]].p, pb = cubes[collected[b]].p, o2 = segs * 6;
      np[o2] = pa.x; np[o2 + 1] = pa.y; np[o2 + 2] = pa.z; np[o2 + 3] = pb.x; np[o2 + 4] = pb.y; np[o2 + 5] = pb.z; segs++;
    }
    ng.setDrawRange(0, segs * 2); ng.attributes.position.needsUpdate = true;
  }

  /* ---- pointer: raycast on window events; the canvas never takes events ---- */
  var ray = new THREE.Raycaster(), ndc = new THREE.Vector2();
  function pick(e) { ndc.set(e.clientX / innerWidth * 2 - 1, -(e.clientY / innerHeight) * 2 + 1); ray.setFromCamera(ndc, cam); var h = ray.intersectObject(inst); return h.length ? h[0].instanceId : -1; }
  function blocked(e) { return e.target.closest && e.target.closest("a,button,input,summary,.nav,.slab-card,.slab-ch,.slab-sw"); }
  function setHover(k) { if (k !== hover) { hover = k; root.style.cursor = k >= 0 ? "pointer" : ""; } }
  addEventListener("pointermove", function (e) {
    if (drag) {
      var dx = e.clientX - drag.x, dy = e.clientY - drag.y; drag.x = e.clientX; drag.y = e.clientY;
      if (Math.abs(e.clientX - drag.x0) + Math.abs(e.clientY - drag.y0) > 5) drag.moved = true;
      var cb = cubes[drag.i]; cb.q.premultiply(qd.setFromEuler(new THREE.Euler(dy * 0.012, dx * 0.012, 0))); cb.spin.set(dy * 0.4, dx * 0.4, 0); return;
    }
    setHover(blocked(e) ? -1 : pick(e));
  }, { passive: true });
  addEventListener("pointerdown", function (e) {
    if (e.button !== 0 || blocked(e)) return;
    var k = pick(e); if (k < 0) return;
    e.preventDefault(); drag = { i: k, x: e.clientX, y: e.clientY, x0: e.clientX, y0: e.clientY, moved: false };
  });
  addEventListener("pointerup", function (e) {
    if (!drag) return; var d = drag; drag = null;
    if (!d.moved) { cubes[d.i].spin.set(6, 9, 0); facts.open(d.i, e.clientX + 16, e.clientY - 20); }
    setHover(pick(e));
  });
  addEventListener("slab:collect", function (e) {
    var k = e.detail, at = collected.indexOf(k);
    if (at >= 0) { collected.splice(at, 1); cubes[k].held = false; }
    else { if (collected.length >= MAXC) cubes[collected.shift()].held = false; collected.push(k); cubes[k].held = true; }
    label(k);
  });
  function label(k) { facts.setCollect(collected.indexOf(k) >= 0 ? "Release" : "Collect (" + collected.length + "/" + MAXC + ")"); }
  facts.onOpen = function (k) { pulse[k] = 1.6; cubes[k].spin.set(5, 7, 0); label(k); };

  /* ---- loop: scroll-driven camera + props; pauses when hidden or while the hero is on screen ---- */
  var last = performance.now(), lastS = 0, vel = 0, slow = 0, arched = false;
  function frame() {
    var now = performance.now();
    var dt = Math.min(0.05, (now - last) / 1000) || 0.016; last = now;
    var rw = W.slabRunway;
    if (document.hidden || !rw || rw.s < -innerHeight * 0.9) return;
    if (!arched && rw.S && rw.total) { placeArches(); arched = true; }
    if (!tierDown) { slow = dt * 1000 > 24 ? slow + 1 : Math.max(0, slow - 1); if (slow > 30) { tierDown = true; dpr = 1; size();
      gardenGeometry.setDrawRange(0, coreVertexCount); streaks.l.visible = false; } }
    var p = Math.min(1, Math.max(0, W.scrollY / Math.max(1, rw.max)));
    vel += (((W.scrollY - lastS) / dt) - vel) * 0.15; lastS = W.scrollY;
    var vn = Math.min(1, Math.abs(vel) / 2500);
    var uz = p * TRAVEL, stride = uz * 3.9;
    path.getPoint(p, tmp); path.getTangent(p, tan);
    cam.position.set(tmp.x + Math.sin(uz * 0.45) * (0.05 + vn * 0.12), tmp.y + Math.sin(stride) * (0.03 + vn * 0.1), 0);
    cam.rotation.set(Math.sin(stride + 1.2) * (0.004 + vn * 0.012), -tan.x * 0.12 + Math.sin(uz * 0.45) * 0.01, Math.max(-1, Math.min(1, vel / 4000)) * 0.04 + Math.cos(stride * 0.5) * vn * 0.01 + tan.y * 0.02);
    kinds.forEach(function (K) { K.m.uniforms.uZ.value = uz; });
    gardenMaterial.uniforms.uZ.value = uz; gardenMaterial.uniforms.uT.value = now / 1000;
    var fast = Math.max(0, (vn - 0.3) / 0.7);
    streaks.m.uniforms.uLen.value = 0.5 + fast * 16; streaks.m.uniforms.uA.value = fast * 0.5;
    streaks.l.visible = !tierDown && fast > 0.02;
    updateCubes(dt); renderer.render(scene, cam);
  }
  W.gsap.ticker.add(frame);
})();
