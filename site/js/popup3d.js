(function () {
  "use strict";
  var DEG = Math.PI / 180;
  var REST_ANGLE = -18, FLAT_ANGLE = -88, TILT_MAX = 8, LIFT_HOVER = 40, LIFT_OPEN = 70, OPEN_SCALE = 1.07;
  var RISE_START = 0.9, RISE_SPAN = 0.55;

  function clamp01(v) { return v < 0 ? 0 : v > 1 ? 1 : v; }
  function ease(t) { return 1 - Math.pow(1 - t, 3); }

  function axisAngle(pitchDeg, yawDeg) {
    var hx = pitchDeg * DEG / 2, hy = yawDeg * DEG / 2;
    var sx = Math.sin(hx), cx = Math.cos(hx), sy = Math.sin(hy), cy = Math.cos(hy);
    var w = cx * cy, x = sx * cy, y = cx * sy, z = sx * sy;
    var s = Math.sqrt(x * x + y * y + z * z);
    if (s < 1e-6) return "1 0 0 0deg";
    return (x / s).toFixed(4) + " " + (y / s).toFixed(4) + " " + (z / s).toFixed(4) + " " + (2 * Math.acos(Math.min(1, w)) / DEG).toFixed(2) + "deg";
  }

  function paint(item) {
    var rise = item.rise;
    var pitch = FLAT_ANGLE + (REST_ANGLE - FLAT_ANGLE) * ease(rise);
    pitch = pitch * item.slab + item.tiltX;
    var z = item.depth * ease(rise) * item.slab + item.lift;
    var key = pitch.toFixed(1) + "|" + item.tiltY.toFixed(1) + "|" + z.toFixed(0) + "|" + item.scale.toFixed(3);
    if (key === item.key) return;
    item.key = key;
    var style = item.el.style;
    style.translate = "0 0 " + z.toFixed(1) + "px";
    style.rotate = axisAngle(pitch, item.tiltY);
    style.scale = item.scale === 1 ? "" : item.scale.toFixed(3);
  }

  function climbTo3dAncestors(el, stop) {
    var node = el.parentElement;
    while (node && node !== stop) { node.classList.add("p3-keep"); node = node.parentElement; }
  }

  function offsetWithin(el, chapter) {
    var y = 0, node = el;
    while (node && node !== chapter) { y += node.offsetTop; node = node.offsetParent; }
    return y;
  }

  function create(options) {
    var gsap = window.gsap;
    var chapters = options.chapters, groups = options.groups, items = [], byChapter = chapters.map(function () { return []; });

    function hoverState(item, on) {
      item.hot = on;
      item.el.classList.toggle("p3-hot", on || item.open);
      gsap.to(item, { lift: on ? LIFT_HOVER : 0, duration: 0.35, ease: "power3.out", overwrite: "auto", onUpdate: function () { paint(item); } });
      if (!on) gsap.to(item, { tiltX: 0, tiltY: 0, duration: 0.5, ease: "power3.out", overwrite: false, onUpdate: function () { paint(item); } });
    }

    function toggleOpen(item) {
      item.open = !item.open;
      item.el.classList.toggle("p3-open", item.open);
      item.el.setAttribute("aria-expanded", String(item.open));
      item.el.classList.toggle("p3-hot", item.hot || item.open);
      gsap.to(item, { scale: item.open ? OPEN_SCALE : 1, lift: item.open ? LIFT_OPEN : (item.hot ? LIFT_HOVER : 0), duration: 0.5, ease: "back.out(1.6)", overwrite: "auto", onUpdate: function () { paint(item); } });
    }

    function wireInteraction(item) {
      var el = item.el;
      el.addEventListener("pointerenter", function () { hoverState(item, true); });
      el.addEventListener("pointerleave", function () { hoverState(item, false); });
      el.addEventListener("focus", function () { hoverState(item, true); });
      el.addEventListener("blur", function () { hoverState(item, false); });
      var tiltX = gsap.quickTo(item, "tiltX", { duration: 0.4, ease: "power3.out", onUpdate: function () { paint(item); } });
      var tiltY = gsap.quickTo(item, "tiltY", { duration: 0.4, ease: "power3.out", onUpdate: function () { paint(item); } });
      el.addEventListener("pointermove", function (e) {
        var box = el.getBoundingClientRect();
        if (!box.width || !box.height) return;
        var nx = (e.clientX - box.left) / box.width * 2 - 1, ny = (e.clientY - box.top) / box.height * 2 - 1;
        tiltX(Math.max(-1, Math.min(1, ny)) * TILT_MAX);
        tiltY(Math.max(-1, Math.min(1, -nx)) * TILT_MAX);
      }, { passive: true });
      el.setAttribute("tabindex", "0");
      el.setAttribute("role", "button");
      el.setAttribute("aria-expanded", "false");
      el.addEventListener("click", function (e) {
        if (e.target.closest && e.target.closest("a,button,summary,input,iframe")) return;
        toggleOpen(item);
      });
      el.addEventListener("keydown", function (e) {
        if (e.target !== el) return;
        if (e.key === "Enter" || e.key === " ") { e.preventDefault(); toggleOpen(item); }
      });
    }

    chapters.forEach(function (chapter, chapterIndex) {
      groups.forEach(function (group) {
        [].forEach.call(chapter.querySelectorAll(group.selector), function (el) {
          if (items.some(function (known) { return known.el === el; })) return;
          var item = { el: el, chapter: chapterIndex, depth: group.depth, interactive: !!group.interactive, offset: 0, rise: 0, slab: 1,
            tiltX: 0, tiltY: 0, lift: 0, scale: 1, open: false, hot: false, key: "", order: byChapter[chapterIndex].length };
          el.classList.add("p3", group.interactive ? "p3-card" : "p3-flat");
          climbTo3dAncestors(el, chapter);
          if (group.interactive) wireInteraction(item);
          items.push(item);
          byChapter[chapterIndex].push(item);
        });
      });
    });

    function measure() {
      items.forEach(function (item) { item.offset = offsetWithin(item.el, chapters[item.chapter]); });
    }

    function update(frameState) {
      var u = frameState.u, starts = frameState.starts, H = frameState.height, slab = frameState.slab, visible = frameState.visible;
      for (var c = 0; c < byChapter.length; c++) {
        if (visible && visible[c] === false) continue;
        var list = byChapter[c];
        for (var i = 0; i < list.length; i++) {
          var item = list[i];
          var y = starts[c] + item.offset;
          var rise = clamp01((u + RISE_START * H - y) / (RISE_SPAN * H));
          if (rise === item.rise && slab === item.slab) continue;
          item.rise = rise; item.slab = slab; paint(item);
        }
      }
    }

    function suspend() {
      items.forEach(function (item) {
        item.el.style.translate = ""; item.el.style.rotate = ""; item.el.style.scale = "";
        item.key = ""; item.rise = -1;
      });
    }

    function dispose() {
      items.forEach(function (item) {
        item.el.style.translate = ""; item.el.style.rotate = ""; item.el.style.scale = "";
        item.el.classList.remove("p3", "p3-card", "p3-flat", "p3-hot", "p3-open");
      });
    }

    measure();
    return { update: update, measure: measure, suspend: suspend, dispose: dispose, items: items };
  }

  window.Popup3D = { create: create };
})();
