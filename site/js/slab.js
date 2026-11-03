/* Slab layer for index.html (classic script, opt-in). Always: the facts card (works from the #facts list).
   Gated (reduced-motion, width < 900, no preserve-3d, ?mode=flat, or gsap/ScrollTrigger missing => normal page):
   the sections below the hero lie on the tilted, bending slab of docs/ui/editions-replication-spec.md section 4.
   Chapters keep their natural place in the document flow (so ScrollTrigger positions and anchors stay valid);
   only a per-chapter transform is added inside a perspective wrapper. Hero and its pin are never touched.
   Known limits: find-in-page does not scroll to matches; ScrollTrigger positions are measured with transforms cleared. */
(function () {
  "use strict";
  var D = document, root = D.documentElement, W = window;

  /* ---- facts card (always) ---- */
  var buttons = [].slice.call(D.querySelectorAll("[data-fact]"));
  var items = buttons.map(function (b) { return { tag: b.getAttribute("data-tag"), text: b.querySelector(".slab-t").textContent, el: b }; });
  var card = D.createElement("div");
  card.className = "slab-card"; card.hidden = true; card.setAttribute("role", "status");
  card.innerHTML = '<span class="slab-tag"></span><p></p><div class="slab-act"><button type="button" data-collect></button><button type="button" data-close>Close</button></div>';
  D.body.appendChild(card);
  var cur = -1;
  function place(x, y) {
    var w = card.offsetWidth || 320, h = card.offsetHeight || 160;
    card.style.left = Math.max(12, Math.min(W.innerWidth - w - 12, x)) + "px";
    card.style.top = Math.max(64, Math.min(W.innerHeight - h - 12, y)) + "px";
  }
  function close() { card.hidden = true; if (cur >= 0) items[cur].el.setAttribute("aria-expanded", "false"); cur = -1; }
  function open(i, x, y) {
    if (cur >= 0) items[cur].el.setAttribute("aria-expanded", "false");
    cur = i; var it = items[i];
    var tag = card.querySelector(".slab-tag"); tag.className = "slab-tag " + it.tag; tag.textContent = it.tag === "live" ? "LIVE" : "IN PROGRESS";
    card.querySelector("p").textContent = it.text; it.el.setAttribute("aria-expanded", "true"); card.hidden = false;
    if (x == null) { var r = it.el.getBoundingClientRect(); x = r.left; y = r.bottom + 8; if (y + 180 > W.innerHeight) y = r.top - 180; }
    place(x, y);
    if (W.slabFacts && W.slabFacts.onOpen) W.slabFacts.onOpen(i);
  }
  buttons.forEach(function (b, i) { b.setAttribute("aria-expanded", "false"); b.addEventListener("click", function () { var r = b.getBoundingClientRect(); open(i, r.left, r.bottom + 8); }); });
  card.addEventListener("click", function (e) {
    if (e.target.hasAttribute("data-close")) close();
    if (e.target.hasAttribute("data-collect")) W.dispatchEvent(new CustomEvent("slab:collect", { detail: cur }));
  });
  D.addEventListener("keydown", function (e) { if (e.key === "Escape") close(); });
  W.slabFacts = { items: items, open: open, close: close,
    setCollect: function (label) { card.querySelector("[data-collect]").textContent = label; } };

  /* ---- gate + switch: Classic (default) / 3D slab. ?mode=slab or ?mode=flat override and are remembered. ---- */
  var q = new URLSearchParams(location.search), KEY = "apex-slab", mode = q.get("mode"), saved = null;
  try { if (mode === "slab") localStorage.setItem(KEY, "1"); else if (mode === "flat") localStorage.setItem(KEY, "0"); saved = localStorage.getItem(KEY); } catch (e) { /* storage blocked: session default */ }
  var want = mode === "slab" || (mode !== "flat" && saved === "1");
  var why = W.matchMedia("(prefers-reduced-motion: reduce)").matches ? "reduced motion is on"
    : W.innerWidth < 900 ? "the window is narrower than 900px"
    : !(W.CSS && CSS.supports("transform-style", "preserve-3d")) ? "this browser lacks 3D transforms"
    : !(W.gsap && W.ScrollTrigger) ? "the animation libraries did not load" : "";
  var sw = D.createElement("button");
  sw.type = "button"; sw.className = "slab-sw"; sw.setAttribute("role", "switch"); sw.setAttribute("aria-label", "3D slab layout");
  sw.innerHTML = '<span class="a">Classic</span><span class="b">3D slab</span>';
  sw.setAttribute("aria-checked", want && !why ? "true" : "false");
  if (why) { sw.setAttribute("aria-disabled", "true"); sw.title = "3D slab is unavailable: " + why + "."; }
  else sw.title = "Switch between the classic page and the 3D slab layout (reloads the page)";
  D.body.appendChild(sw);
  sw.addEventListener("click", function () {
    if (why) return;
    try { localStorage.setItem(KEY, want ? "0" : "1"); } catch (e) { /* ignore */ }
    var u = new URL(location.href); u.searchParams.set("mode", want ? "flat" : "slab"); location.href = u.toString();
  });
  if (why || !want) return;
  var main = D.getElementById("main");
  var chs = ["product", "principles", "anatomy", "repo", "benchmarks", "fit", "manifesto", "open", "status", "facts", "cta"]
    .map(function (id) { return D.getElementById(id); }).concat([D.querySelector("main > .foot")]).filter(Boolean);
  if (!main || chs.length < 3) return;

  var world = D.createElement("div"); world.className = "slab-world";
  chs[0].parentNode.insertBefore(world, chs[0]);
  chs.forEach(function (c) { c.classList.add("slab-ch"); world.appendChild(c); });
  root.classList.add("slab-on");
  var pop = W.Popup3D ? W.Popup3D.create({ chapters: chs, groups: [
    { selector: "h2", depth: 80 },
    { selector: ".lede,.lead,.slab-lede", depth: 40 },
    { selector: "article,.card,.track,li[data-rv]", depth: 24, interactive: true }
  ] }) : null;

  var A0 = 58, A1 = 20, D0 = 0.3, D1 = 1.5, PK = 0.75, YF = 0.5, RAMP = 0.6, REVEAL_LEAD = 0.55, rad = Math.PI / 180;
  var n = chs.length, H, P, top0 = 0, total = 0, L = [], S = [], yb = [], zb = [], ang = [], vis = [], clipv = [];
  var rw = W.slabRunway = { s: 0, t: 0, k: 0, a0: 0, S: S, top0: 0, max: 1, v: 0 };
  var lastY = -1, dirty = true, curK = -1, on = true;

  function sm(a, b, x) { if (b <= a) return x >= b ? 1 : 0; var t = Math.min(1, Math.max(0, (x - a) / (b - a))); return t * t * (3 - 2 * t); }
  function proj(y, z) { var oy = 0.2 * H; return oy + (y - oy) * P / (P - z); }

  function measure() {
    H = W.innerHeight; P = PK * H;
    world.style.perspective = P + "px";
    var acc = 0;
    chs.forEach(function (c, i) { L[i] = c.offsetHeight; S[i] = acc; acc += L[i]; });
    total = acc;
    top0 = world.getBoundingClientRect().top + W.scrollY; rw.top0 = top0; rw.total = total;
    if (pop) pop.measure();
    dirty = true;
  }
  function clear() {
    on = false;
    if (pop) pop.suspend();
    chs.forEach(function (c, i) { c.style.transform = ""; c.style.visibility = ""; vis[i] = true; });
    world.style.perspective = "none";
  }

  function frame(y) {
    var s = y - top0, smax = Math.max(1, root.scrollHeight - H - top0);
    var t = Math.min(sm(0, RAMP * H, s), 1 - sm(smax - RAMP * H, smax, s));
    var yF = t * YF * H, u = s + yF, k = 0, i;
    world.style.perspectiveOrigin = "50% " + (s + 0.2 * H).toFixed(1) + "px";
    for (i = 0; i < n; i++) if (S[i] <= u) k = i;
    var a = t * A0, ca = Math.cos(a * rad), sa = Math.sin(a * rad), rem = S[k] + L[k] - u;
    yb[k] = yF + rem * ca; zb[k] = rem * sa; ang[k] = a;
    for (i = k - 1; i >= 0; i--) {
      var d = u - (S[i] + L[i]);
      ang[i] = t * (A0 - (A0 - A1) * sm(D0 * H, D1 * H, d));
      yb[i] = yb[i + 1] - L[i + 1] * Math.cos(ang[i + 1] * rad);
      zb[i] = zb[i + 1] - L[i + 1] * Math.sin(ang[i + 1] * rad);
    }
    for (i = k + 1; i < n; i++) { ang[i] = a; yb[i] = yb[i - 1] + L[i] * ca; zb[i] = zb[i - 1] + L[i] * sa; }
    for (i = 0; i < n; i++) {
      var r = ang[i] * rad, topY = yb[i] - L[i] * Math.cos(r), topZ = zb[i] - L[i] * Math.sin(r), nat = S[i] + L[i] - s;
      var v = !(topZ > 0.9 * P) && !(proj(topY, topZ) > H) && !(zb[i] < 0.9 * P && proj(yb[i], zb[i]) < 0);
      if (t === 0) v = true;
      if (v !== vis[i]) { vis[i] = v; chs[i].style.visibility = v ? "" : "hidden"; }
      var cut = t === 0 || ang[i] < 1 ? 0 : Math.max(0, Math.round((zb[i] - 0.6 * P) / Math.sin(r))); // keep geometry in front of the camera plane
      if (cut !== clipv[i]) { clipv[i] = cut; chs[i].style.clipPath = cut ? "inset(0 0 " + cut + "px 0)" : ""; }
      if (v) chs[i].style.transform = t === 0 ? "" : "translate3d(0," + (yb[i] - nat).toFixed(2) + "px," + zb[i].toFixed(2) + "px) rotateX(" + ang[i].toFixed(3) + "deg)";
    }
    if (pop) pop.update({ u: u, starts: S, height: H, slab: t, visible: vis });
    rw.s = s; rw.t = t; rw.k = k; rw.a0 = a; rw.max = root.scrollHeight - H;
    if (k !== curK) curK = k;
  }

  /* the slab shows content earlier than its natural scroll position: start once-only reveals a little sooner */
  function shiftReveals() {
    W.ScrollTrigger.getAll().forEach(function (st) {
      var el = st.trigger;
      if (st.vars && st.vars.once && el && el.closest && el.closest(".slab-ch")) st.start -= REVEAL_LEAD * H;
    });
  }
  W.ScrollTrigger.addEventListener("refreshInit", clear);
  W.ScrollTrigger.addEventListener("refresh", function () { on = true; measure(); shiftReveals(); });

  var hud = D.createElement("div"); hud.className = "slab-hud"; hud.setAttribute("aria-hidden", "true"); D.body.appendChild(hud);
  var setHud = W.gsap.quickSetter(hud, "textContent");
  W.gsap.ticker.add(function () {
    if (!on) return;
    var y = W.scrollY;
    if (y !== lastY || dirty) { lastY = y; dirty = false; frame(y); setHud(String(Math.round(Math.min(1, y / Math.max(1, rw.max)) * 100)).padStart(3, "0") + "%"); }
  });
  if (W.ResizeObserver) new ResizeObserver(function () { if (on) measure(); }).observe(world);
  W.addEventListener("load", function () { measure(); });
  measure();

  /* anchors: native layout positions are valid (transforms do not move layout), but Lenis may read rects; use the chapter offset */
  D.addEventListener("click", function (e) {
    var a = e.target.closest && e.target.closest('a[href^="#"]');
    if (!a || a.getAttribute("href").length < 2) return;
    var el = D.getElementById(a.getAttribute("href").slice(1));
    if (!el) return;
    var c = el.closest(".slab-ch"), i = chs.indexOf(c);
    if (i < 0) return;
    e.preventDefault(); e.stopImmediatePropagation();
    var y = top0 + S[i], node = el;
    if (el !== c) { y = top0 + S[i]; while (node && node !== c) { y += node.offsetTop; node = node.offsetParent; } }
    W.scrollTo({ top: Math.max(0, y - 20), behavior: "smooth" });
  }, true);
  D.addEventListener("focusin", function (e) {
    var el = e.target, c = el.closest && el.closest(".slab-ch");
    if (!c || e.target.closest(".nav")) return;
    var i = chs.indexOf(c), y = top0 + S[i], node = el;
    while (node && node !== c) { y += node.offsetTop; node = node.offsetParent; }
    W.scrollTo({ top: Math.max(0, y - 0.35 * H), behavior: "auto" });
  });
})();
