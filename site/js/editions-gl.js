/* Editions landing, layer 2: background field only (module). Needs html.ed-3d + WebGL; otherwise the
   CSS --deep field stays. draw calls: 3 merged LineSegments layers (arcs/diagonals/vortex, buildings, rings) + one Points (dust); monochrome. Reads window.edRunway; renders only when it changes; paused when hidden. */
import * as THREE from "../vendor/three.module.min.js";

(function () {
  var root = document.documentElement, canvas = document.querySelector(".ed-gl");
  if (!canvas || !root.classList.contains("ed-3d") || new URLSearchParams(location.search).get("mode") === "nogl") return;
  var renderer;
  try { renderer = new THREE.WebGLRenderer({ canvas: canvas, antialias: true, alpha: false }); } catch (e) { return; }
  renderer.setClearColor(0x000000, 1); // field outside the slab is monochrome

  var scene = new THREE.Scene(), cam = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
  cam.position.set(0, 0, 14);
  var group = new THREE.Group(); scene.add(group);

  var seed = 7; function rnd() { seed = (seed * 16807) % 2147483647; return seed / 2147483647; }
  var layers = [{ pos: [], col: [] }, { pos: [], col: [] }, { pos: [], col: [] }], pos, col, MID = [1, 1, 1];
  function use(i) { pos = layers[i].pos; col = layers[i].col; }
  use(0);
  function seg(a, b, k) { pos.push(a[0], a[1], a[2], b[0], b[1], b[2]); for (var i = 0; i < 2; i++) col.push(MID[0] * k, MID[1] * k, MID[2] * k); }
  function ring(c, r, rx, ry, k, n) { // circle in a plane rotated by rx, ry
    var pts = []; for (var i = 0; i <= n; i++) { var a = i / n * 6.2832, x = Math.cos(a) * r, y = Math.sin(a) * r, z = 0;
      var y2 = y * Math.cos(rx) - z * Math.sin(rx), z2 = y * Math.sin(rx) + z * Math.cos(rx);
      var x3 = x * Math.cos(ry) + z2 * Math.sin(ry), z3 = -x * Math.sin(ry) + z2 * Math.cos(ry);
      pts.push([c[0] + x3, c[1] + y2, c[2] + z3]); }
    for (i = 0; i < n; i++) seg(pts[i], pts[i + 1], k);
  }
  function box(c, w, h, d, k) {
    var v = []; for (var i = 0; i < 8; i++) v.push([c[0] + (i & 1 ? w : -w) / 2, c[1] + (i & 2 ? h : -h) / 2, c[2] + (i & 4 ? d : -d) / 2]);
    [[0, 1], [2, 3], [4, 5], [6, 7], [0, 2], [1, 3], [4, 6], [5, 7], [0, 4], [1, 5], [2, 6], [3, 7]].forEach(function (e) { seg(v[e[0]], v[e[1]], k); });
  }
  // layer 0 (far): faint arcs, diagonals, vortex grids
  for (var i = 0; i < 12; i++) { var R = 12 + rnd() * 18, cx = (rnd() - 0.5) * 44, cy = -10 + (rnd() - 0.5) * 36, a0 = rnd() * 6, sp = 0.8 + rnd(), prev = null;
    for (var j = 0; j <= 40; j++) { var a = a0 + sp * j / 40, p = [cx + Math.cos(a) * R, cy + Math.sin(a) * R, -8]; if (prev) seg(prev, p, 0.12); prev = p; } }
  for (i = 0; i < 16; i++) { var x0 = (rnd() - 0.5) * 44, y0 = -10 + (rnd() - 0.5) * 36; seg([x0, y0, -8], [x0 + 12 + rnd() * 10, y0 + 8 + rnd() * 8, -8], 0.1); }
  function vortex(vc, s) {
    for (var q = 0; q < 10; q++) ring([vc[0], vc[1] - q * 0.35 * s, vc[2]], (0.4 + q * 0.28) * s, 1.45, 0, 0.3, 32);
    for (q = 0; q < 14; q++) { var aa = q / 14 * 6.2832; seg([vc[0] + Math.cos(aa) * 0.4 * s, vc[1], vc[2]], [vc[0] + Math.cos(aa) * 3 * s, vc[1] - 3.2 * s, vc[2] + Math.sin(aa) * 0.3 * s], 0.28); }
  }
  vortex([9.5, -2, -6], 1); vortex([-9.5, -12, -6], 1.1); vortex([10, -22, -6], 0.9);
  // layer 1 (mid): wireframe isometric buildings
  use(1);
  for (i = 0; i < 46; i++) { var side = i % 2 ? 1 : -1; box([side * (5.8 + rnd() * 6.5), -25 + (rnd() - 0.5) * 62, -3 - rnd() * 2], 0.8 + rnd() * 1.4, 1 + rnd() * 3.5, 0.8 + rnd() * 1.4, 0.33); }
  // layer 2 (near): interlocked torus rings
  use(2);
  function tori(c, r) { for (var q = 0; q < 3; q++) ring([c[0] + q * r * 0.45, c[1], c[2]], r, 1.2, q * 1.05, 0.38, 48); }
  tori([-9.5, 0, -1], 1.5); tori([8.2, -14, -1], 1.3); tori([-8, -30, -1], 1.6); tori([8.8, -45, -1], 1.4); tori([-8.6, -60, -1], 1.2);
  var lines = layers.map(function (L) {
    var lg = new THREE.BufferGeometry();
    lg.setAttribute("position", new THREE.Float32BufferAttribute(L.pos, 3)); lg.setAttribute("color", new THREE.Float32BufferAttribute(L.col, 3));
    var o = new THREE.LineSegments(lg, new THREE.LineBasicMaterial({ vertexColors: true })); group.add(o); return o;
  });
  var TRAVEL = [20, 50, 70]; // parallax travel per layer over the whole scroll (edRunway.s / edRunway.max)

  var dp = []; for (i = 0; i < 260; i++) dp.push((rnd() - 0.5) * 44, (rnd() - 0.5) * 34, -8 + rnd() * 6);
  var dg = new THREE.BufferGeometry(); dg.setAttribute("position", new THREE.Float32BufferAttribute(dp, 3));
  group.add(new THREE.Points(dg, new THREE.PointsMaterial({ color: 0xffffff, size: 0.05, transparent: true, opacity: 0.5 })));

  function size() {
    var w = innerWidth, h = innerHeight;
    renderer.setPixelRatio(Math.min(devicePixelRatio || 1, 1.5)); renderer.setSize(w, h, false);
    cam.aspect = w / h; cam.updateProjectionMatrix(); dirty = true;
  }
  var dirty = true, last = "";
  addEventListener("resize", size); size();
  root.classList.add("ed-gl-on");
  (function loop() {
    requestAnimationFrame(loop);
    var r = window.edRunway;
    if (document.hidden || !r) return;
    var key = r.s.toFixed(1) + "|" + r.t.toFixed(3);
    if (!dirty && key === last) return;
    last = key; dirty = false;
    lines.forEach(function (l, q) { l.position.y = (r.s / Math.max(1, r.max)) * TRAVEL[q]; });
    group.rotation.x = r.t * 0.12; group.rotation.y = Math.sin(r.s * 0.0008) * 0.08;
    renderer.render(scene, cam);
  })();
})();
