/* Editions landing, layer 1 (classic script). The page is complete without it (HTML + CSS).
   Always on: chapter index highlight, end bar.
   Gated on gsap + ScrollTrigger + Lenis, no reduced-motion, innerWidth >= 900, preserve-3d, no ?mode=flat:
   html.ed-3d and the CSS-3D slab (docs/ui/editions-replication-spec.md section 4). Text stays real DOM text.
   Known limits of the 3D mode: browser find-in-page will not scroll to matches (chapters are transformed);
   keyboard focus is handled by the focusin handler below, which scrolls the focused element on screen. */
(function () {
  "use strict";
  var D = document, root = D.documentElement, win = window;
  var chs = [].slice.call(D.querySelectorAll(".ed-ch"));
  var links = [].slice.call(D.querySelectorAll(".ed-idx a"));
  var end = D.querySelector(".ed-end");
  var closed = false;

  /* ---- index ---- */
  links.forEach(function (a) {
    var p = "";
    for (var j = -6; j <= 6; j++) p += '<path d="M0 22C30 22 70 ' + (22 + j * 2) + " 140 " + (22 + j * 5) + '"/>';
    a.insertAdjacentHTML("beforeend", '<svg class="ed-bundle" viewBox="0 0 140 44" aria-hidden="true">' + p + "</svg>");
  });
  function setCur(id) {
    links.forEach(function (a) {
      if (a.getAttribute("href") === "#" + id) a.setAttribute("aria-current", "true");
      else a.removeAttribute("aria-current");
    });
  }
  if ("IntersectionObserver" in win) {
    var io = new IntersectionObserver(function (es) {
      es.forEach(function (e) { if (e.isIntersecting && !root.classList.contains("ed-3d")) setCur(e.target.id); });
    }, { rootMargin: "-45% 0px -50% 0px" });
    chs.forEach(function (c) { io.observe(c); });
  }

  /* ---- end bar: real counts from this page ---- */
  var counts = { ch: chs.length, live: D.querySelectorAll(".ed-tag.live").length, wip: D.querySelectorAll(".ed-tag.wip").length };
  [].forEach.call(end.querySelectorAll("[data-n]"), function (b) { b.textContent = counts[b.getAttribute("data-n")]; });
  function onScroll() {
    var max = root.scrollHeight - win.innerHeight;
    var near = max > 0 && win.scrollY >= max - 0.12 * win.innerHeight;
    if (!near) closed = false;
    end.classList.toggle("on", near && !closed);
  }
  win.addEventListener("scroll", onScroll, { passive: true });
  end.addEventListener("click", function (e) {
    var act = e.target.getAttribute && e.target.getAttribute("data-act");
    if (act === "close") { closed = true; end.classList.remove("on"); }
    if (act === "again") { closed = true; end.classList.remove("on"); if (win.edScrollTo) win.edScrollTo(0); else win.scrollTo(0, 0); }
  });
  onScroll();

  /* ---- 3D gate ---- */
  var q = new URLSearchParams(location.search);
  var reduce = win.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (!(win.gsap && win.ScrollTrigger && win.Lenis) || reduce || win.innerWidth < 900 ||
      !(win.CSS && CSS.supports("transform-style", "preserve-3d")) || q.get("mode") === "flat") return;
  win.gsap.registerPlugin(win.ScrollTrigger);
  root.classList.add("ed-3d");

  /* knobs (spec section 1) */
  var A0 = 58, A1 = 20, D0 = 0.3, D1 = 1.5, PK = 0.75, YF = 0.5, RAMP = 0.6, LERP = 0.085, WHEEL = 0.9;
  var view = D.querySelector(".ed-view");
  var pop = win.Popup3D ? win.Popup3D.create({ chapters: chs, groups: [
    { selector: ".ed-title", depth: 80 },
    { selector: ".ed-lede,.ed-k", depth: 40 },
    { selector: ".ed-card", depth: 24, interactive: true },
    { selector: ".ed-cols p", depth: 24 }
  ] }) : null;
  var n = chs.length, H, P, total, L = [], S = [], yb = [], zb = [], ang = [], vis = [], clipv = [], played = [], tls = [];
  var rw = win.edRunway = { s: 0, u: 0, t: 0, a0: 0, P: 0, H: 0, yF: 0, k: 0 };
  var dirty = true, lastS = -1, curK = -1, rad = Math.PI / 180;

  function sm(a, b, x) { if (b <= a) return x >= b ? 1 : 0; var t = Math.min(1, Math.max(0, (x - a) / (b - a))); return t * t * (3 - 2 * t); }

  /* node clusters: static SVG, deterministic */
  function rng(seed) { return function () { seed |= 0; seed = seed + 0x6D2B79F5 | 0; var t = Math.imul(seed ^ seed >>> 15, 1 | seed); t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t; return ((t ^ t >>> 14) >>> 0) / 4294967296; }; }
  chs.forEach(function (ch, i) {
    var r = rng(11 + i * 7), cx = 120, cy = 130, s = "", m = 25 + Math.floor(r() * 10);
    for (var j = 0; j < m; j++) {
      var a = r() * 6.2832, d = 40 + r() * 90, x = cx + Math.cos(a) * d, y = cy + Math.sin(a) * d;
      s += '<line x1="' + cx + '" y1="' + cy + '" x2="' + x.toFixed(1) + '" y2="' + y.toFixed(1) + '" stroke="currentColor" stroke-width="1" opacity=".4"/><circle cx="' + x.toFixed(1) + '" cy="' + y.toFixed(1) + '" r="1.6" fill="currentColor" opacity=".7"/>';
    }
    for (j = 0; j < 7; j++) { var sz = 6 + r() * 4; s += '<rect x="' + (r() * 280).toFixed(0) + '" y="' + (r() * 230).toFixed(0) + '" width="' + sz.toFixed(0) + '" height="' + sz.toFixed(0) + '" fill="none" stroke="currentColor" opacity=".8"/>'; }
    var name = ch.getAttribute("data-name"), w = name.length * 8 + 20;
    s += '<rect x="' + (cx - 3) + '" y="' + (cy - 3) + '" width="6" height="6" fill="currentColor"/><rect class="ed-chip" x="' + (cx + 10) + '" y="' + (cy - 36) + '" width="' + w + '" height="22" rx="3"/><text class="ed-chip-t" x="' + (cx + 20) + '" y="' + (cy - 21) + '">' + name + "</text>";
    var nm = ch.getAttribute("data-name"), rl = "";
    [0, 0.34, 0.68, 0.98].forEach(function (f, q) {
      var v = ((n - i) * 1000 + 500 - q * 250);
      rl += '<span class="ed-rule l' + (q ? "" : " nm") + '" style="top:' + (f * 100) + '%" aria-hidden="true">' + (q ? v : nm + " " + v) + '</span><span class="ed-rule r' + (q ? "" : " nm") + '" style="top:' + (f * 100) + '%" aria-hidden="true">' + (q ? v - 125 : v + " " + nm) + "</span>";
    });
    ch.insertAdjacentHTML("beforeend", rl);
    ch.insertAdjacentHTML("beforeend", '<svg class="ed-node" viewBox="0 0 300 240" aria-hidden="true">' + s + "</svg>");
  });

  /* reveals (ScrollTrigger on numeric scroll positions: transformed chapters cannot be measured by rect) */
  chs.forEach(function (ch, i) {
    var tl = win.gsap.timeline({ paused: true, defaults: { ease: "expo.out" } });
    tl.from(ch.querySelector(".ed-title"), { y: 110, opacity: 0, scale: 0.94, transformOrigin: "0 100%", duration: 1.5 }, 0)
      .from(ch.querySelectorAll(".ed-lede,.ed-pill,.ed-k"), { y: 30, opacity: 0, stagger: 0.12, duration: 1.1 }, 0.25)
      .from(ch.querySelectorAll(".ed-card"), { y: 60, opacity: 0, stagger: 0.14, duration: 1.2 }, 0.5)
      .from(ch.querySelectorAll(".ed-cols p"), { y: 24, opacity: 0, stagger: 0.1, duration: 1 }, 0.8);
    tls[i] = tl;
  });
  function reveal(i) { if (!played[i]) { played[i] = true; tls[i].play(); } }
  function buildTriggers() {
    win.ScrollTrigger.getAll().forEach(function (t) { t.kill(); });
    chs.forEach(function (ch, i) {
      win.ScrollTrigger.create({ start: Math.max(0, S[i] - 0.5 * H), end: total, onToggle: function (st) { if (st.isActive) reveal(i); } });
    });
  }

  function measure() {
    H = win.innerHeight; P = PK * H;
    view.style.perspective = P + "px";
    var acc = 0;
    chs.forEach(function (c, i) { L[i] = c.offsetHeight; S[i] = acc; acc += L[i]; });
    total = acc; D.body.style.height = total + "px";
    if (pop) pop.measure();
    buildTriggers(); dirty = true;
  }

  function proj(y, z) { var oy = 0.2 * H; return oy + (y - oy) * P / (P - z); }

  function frame(s) {
    var max = Math.max(1, total - H);
    var t = Math.min(sm(0, RAMP * H, s), 1 - sm(max - RAMP * H, max, s));
    var yF = t * YF * H, u = s + yF, k = 0, i;
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
      var r = ang[i] * rad, topY = yb[i] - L[i] * Math.cos(r), topZ = zb[i] - L[i] * Math.sin(r);
      /* cull whole chapters only when fully off-screen or fully behind the camera */
      var v = !(topZ > 0.9 * P) && !(proj(topY, topZ) > H) && !(zb[i] < 0.9 * P && proj(yb[i], zb[i]) < 0);
      if (v !== vis[i]) { vis[i] = v; chs[i].style.visibility = v ? "visible" : "hidden"; }
      var cut = ang[i] < 1 ? 0 : Math.max(0, Math.round((zb[i] - 0.6 * P) / Math.sin(r)));
      if (cut !== clipv[i]) { clipv[i] = cut; chs[i].style.clipPath = cut ? "inset(-400px -400px " + cut + "px -400px)" : ""; }
      if (v) chs[i].style.transform = "translate3d(0," + (yb[i] - L[i]).toFixed(2) + "px," + zb[i].toFixed(2) + "px) rotateX(" + ang[i].toFixed(3) + "deg)";
      if (i <= k) reveal(i);
    }
    if (pop) pop.update({ u: u, starts: S, height: H, slab: t, visible: vis });
    if (k !== curK) { curK = k; setCur(chs[k].id); }
    rw.s = s; rw.u = u; rw.t = t; rw.a0 = a; rw.P = P; rw.H = H; rw.yF = yF; rw.k = k; rw.max = max;
  }

  var lenis = new win.Lenis({ lerp: LERP, wheelMultiplier: WHEEL });
  win.edScrollTo = function (y) { lenis.scrollTo(y); };
  lenis.on("scroll", win.ScrollTrigger.update);
  win.gsap.ticker.add(function (time) {
    lenis.raf(time * 1000);
    var s = lenis.animatedScroll;
    if (s !== lastS || dirty) { lastS = s; dirty = false; frame(s); }
  });
  win.gsap.ticker.lagSmoothing(0);

  function chapterOf(el) { var c = el.closest && el.closest(".ed-ch"); return c ? chs.indexOf(c) : -1; }
  D.addEventListener("click", function (e) {
    var a = e.target.closest && e.target.closest('a[href^="#"]');
    if (!a) return;
    var el = D.getElementById(a.getAttribute("href").slice(1)), i = el ? chapterOf(el) : -1;
    if (i < 0) return;
    e.preventDefault(); lenis.scrollTo(S[i]);
  });
  D.addEventListener("focusin", function (e) {
    var el = e.target, i = chapterOf(el), y = 0, node = el;
    if (i < 0) return;
    while (node && node !== chs[i]) { y += node.offsetTop; node = node.offsetParent; }
    lenis.scrollTo(S[i] + y - 0.35 * H, { immediate: true });
  });

  var rt;
  win.addEventListener("resize", function () { clearTimeout(rt); rt = setTimeout(measure, 150); });
  win.addEventListener("load", measure);
  if (D.fonts && D.fonts.ready) D.fonts.ready.then(measure);
  measure();
})();
