(function () {
  "use strict";
  var $ = function (id) { return document.getElementById(id); };
  var DIM = 64;
  var STOP = {};
  "the and for are was why did our with that this from have has not but you your can will its into than then them they what when how who all any use using used".split(" ").forEach(function (w) { STOP[w] = 1; });

  function fnv(s) { var h = 2166136261; for (var i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 16777619); } return h >>> 0; }
  function embed(text) {
    var v = new Float32Array(DIM), words = text.toLowerCase().split(/[^a-z0-9]+/);
    words.forEach(function (w) {
      if (w.length < 3 || STOP[w]) return;
      var h = fnv(w); v[h % DIM] += (h & 256) ? 1 : -1;
      if (w.length > 5) { var s = fnv(w.slice(0, 5)); v[s % DIM] += ((s & 256) ? 1 : -1) * 0.5; }
    });
    var n = 0; for (var i = 0; i < DIM; i++) n += v[i] * v[i]; n = Math.sqrt(n) || 1;
    for (var j = 0; j < DIM; j++) v[j] /= n;
    return v;
  }
  function cos(a, b) { var s = 0; for (var i = 0; i < DIM; i++) s += a[i] * b[i]; return s; }

  function Bat() { this.ver = 0; this.tenants = {}; }
  Bat.prototype.T = function (n) { return this.tenants[n] || (this.tenants[n] = { nodes: {}, edges: [] }); };
  Bat.prototype.put = function (tn, node) {
    var T = this.T(tn), ver = ++this.ver, h = T.nodes[node.key] || (T.nodes[node.key] = []);
    h.push({ ver: ver, node: { key: node.key, title: node.title, body: node.body, kind: node.kind || "note", status: node.status || "asserted" }, vec: embed(node.title + " " + node.body) });
    return { seq: ver, version: ver };
  };
  Bat.prototype.link = function (tn, e) { var ver = ++this.ver; this.T(tn).edges.push({ rel: e.rel, from: e.from, to: e.to, ver: ver }); return { seq: ver, version: ver }; };
  Bat.prototype.supersede = function (tn, oldKey, node) {
    var T = this.T(tn), h = T.nodes[oldKey];
    if (!h) return { error: "NOT_FOUND" };
    var cur = h[h.length - 1], ver = ++this.ver;
    h.push({ ver: ver, node: Object.assign({}, cur.node, { status: "superseded" }), vec: cur.vec });
    var nh = T.nodes[node.key] || (T.nodes[node.key] = []);
    nh.push({ ver: ver, node: { key: node.key, title: node.title, body: node.body, kind: node.kind || "decision", status: "asserted" }, vec: embed(node.title + " " + node.body) });
    T.edges.push({ rel: "SUPERSEDES", from: node.key, to: oldKey, ver: ver });
    return { seq: ver, version: ver };
  };
  Bat.prototype.forget = function (tn, key) {
    var h = this.T(tn).nodes[key]; if (!h) return { error: "NOT_FOUND" };
    var cur = h[h.length - 1], ver = ++this.ver;
    h.push({ ver: ver, node: Object.assign({}, cur.node, { status: "forgotten" }), vec: cur.vec, gone: true });
    return { seq: ver, version: ver };
  };
  Bat.prototype.view = function (tn, asOf) {
    var T = this.T(tn), lim = asOf == null ? Infinity : asOf, nodes = {}, k;
    for (k in T.nodes) {
      var h = T.nodes[k], pick = null;
      for (var i = 0; i < h.length; i++) if (h[i].ver <= lim) pick = h[i];
      if (pick && !pick.gone) nodes[k] = pick;
    }
    return { nodes: nodes, edges: T.edges.filter(function (e) { return e.ver <= lim && nodes[e.from] && nodes[e.to]; }) };
  };
  Bat.prototype.recall = function (tn, q, o) {
    var t0 = performance.now(), V = this.view(tn, o.as_of), qv = embed(q), scored = [], k, trace = [];
    for (k in V.nodes) scored.push({ key: k, score: cos(qv, V.nodes[k].vec), n: V.nodes[k].node });
    scored.sort(function (a, b) { return b.score - a.score; });
    var seeds = scored.filter(function (s) { return s.score > 0.08 && s.n.status !== "superseded"; }).slice(0, 3);
    var out = {}, order = [];
    function add(key, why, score) { if (out[key] || order.length >= o.max_nodes) return; out[key] = { key: key, title: V.nodes[key].node.title, kind: V.nodes[key].node.kind, status: V.nodes[key].node.status, score: score, why: why }; order.push(key); }
    seeds.forEach(function (s) { add(s.key, "seed", s.score); trace.push("seed " + s.key + " (cosine " + s.score.toFixed(2) + ")"); });
    var frontier = seeds.map(function (s) { return s.key; });
    for (var hop = 1; hop <= o.max_hops; hop++) {
      var next = [];
      V.edges.forEach(function (e) {
        [[e.from, e.to], [e.to, e.from]].forEach(function (p) {
          if (frontier.indexOf(p[0]) >= 0 && !out[p[1]]) {
            var sc = scored.filter(function (s) { return s.key === p[1]; })[0];
            add(p[1], "hop " + hop + " via " + e.rel, sc ? sc.score : 0);
            if (out[p[1]]) { next.push(p[1]); trace.push("hop " + hop + ": " + p[0] + " -" + e.rel + "- " + p[1]); }
          }
        });
      });
      frontier = next;
    }
    var nodes = order.map(function (x) { return out[x]; });
    return { nodes: nodes, trace: trace.length ? trace : ["no memory matched the question"], as_of: o.as_of == null ? this.ver : o.as_of, ms: performance.now() - t0 };
  };

  var bat = new Bat(), S = { tenant: "team-a", calls: [], sel: null, lastQ: "why did we choose group commit?", busy: false, hl: {} };

  var NODES = [
    { key: "adr-1", title: "Use group commit for the WAL", body: "Batch fsyncs across concurrent writers to raise write throughput without weakening durability.", kind: "decision" },
    { key: "adr-2", title: "Acked writes must survive kill -9", body: "A write is acknowledged only after its record is flushed to the log.", kind: "requirement" },
    { key: "adr-3", title: "Restart must not replay the whole log", body: "Checkpoints write a memory-mapped vector file so restart maps it instead of parsing every record.", kind: "decision" },
    { key: "adr-4", title: "Pin readers to a log version", body: "A reader names a version and sees the same answer every time, with no lock and no half-written record.", kind: "decision" }
  ];
  var EDGES = [["SUPPORTS", "adr-2", "adr-1"], ["DEPENDS_ON", "adr-3", "adr-2"], ["RELATES_TO", "adr-4", "adr-1"]];
  var Q = "why did we choose group commit?";

  function el(tag, cls, text) { var e = document.createElement(tag); if (cls) e.className = cls; if (text !== undefined) e.textContent = text; return e; }
  function wait(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }
  function jsonStr(v) { return JSON.stringify(v, null, 2); }

  function record(name, req, res, tag) {
    var c = { id: S.calls.length + 1, name: name, req: req, res: res, tag: tag || "", tenant: S.tenant, t: res && res.ms != null ? res.ms : 0 };
    S.calls.push(c); drawCall(c); drawGraph(); drawSlider(); stats(); return c;
  }
  function say(who, text) {
    var d = el("div", "pgx-say"); d.append(el("div", "pgx-who", who), el("p", "", text)); feed(d);
  }
  function feed(n) { var f = $("pgx-feed"), e = $("pgx-empty"); if (e) e.remove(); f.append(n); n.scrollIntoView({ block: "nearest", behavior: matchMedia("(prefers-reduced-motion:reduce)").matches ? "auto" : "smooth" }); }
  function drawCall(c) {
    var d = el("div", "pgx-call"); d.tabIndex = 0; d.dataset.id = c.id;
    var h = el("div", "pgx-who"); h.append(document.createTextNode((c.tag ? c.tag + " · " : "") + c.tenant + " · " + c.name));
    h.append(el("span", "pgx-pill", c.t ? c.t.toFixed(2) + " ms" : "ok"));
    d.append(h, el("div", "pgx-mono", c.name + "(" + JSON.stringify(c.req).slice(0, 120) + ")"));
    if (c.res && c.res.nodes) {
      var ul = el("ul", "pgx-hits");
      if (!c.res.nodes.length) ul.append(el("li", "pgx-none", "0 memories returned"));
      c.res.nodes.forEach(function (n) {
        var li = el("li"), top = el("div", "pgx-hit-top");
        top.append(el("b", n.status === "superseded" ? "sup" : "", n.title), el("span", "pgx-dim", n.key + " · " + n.why));
        var bar = el("div", "pgx-bar"), fill = el("i"); fill.style.width = Math.max(3, Math.min(100, n.score * 100)) + "%"; bar.append(fill);
        li.append(top, bar, el("span", "pgx-score", "score " + n.score.toFixed(2)));
        ul.append(li);
      });
      d.append(ul);
      d.append(el("div", "pgx-trace", c.res.trace.join("  ·  ")));
    } else if (c.res && c.res.version != null) d.append(el("div", "pgx-dim", "written at version " + c.res.version));
    function pick() { select(c.id); }
    d.addEventListener("click", pick); d.addEventListener("keydown", function (e) { if (e.key === "Enter") pick(); });
    feed(d); select(c.id, true);
  }
  function select(id, quiet) {
    S.sel = id; var c = S.calls[id - 1]; if (!c) return;
    document.querySelectorAll(".pgx-call").forEach(function (n) { n.classList.toggle("sel", +n.dataset.id === id); });
    var box = $("pgx-insp"); box.textContent = "";
    box.append(el("div", "pgx-who", "request"), pre({ tool: c.name, tenant: c.tenant, args: c.req }));
    box.append(el("div", "pgx-who", "response"), pre(c.res));
    S.hl = {}; if (c.res && c.res.nodes) c.res.nodes.forEach(function (n) { S.hl[n.key] = 1; });
    drawGraph();
  }
  function pre(v) { var p = el("pre"); p.textContent = jsonStr(v); return p; }

  function layout(keys) {
    var n = keys.length, R = 118, cx = 190, cy = 150, pos = {};
    keys.forEach(function (k, i) { var a = -Math.PI / 2 + i * 2 * Math.PI / Math.max(n, 1); pos[k] = [cx + R * Math.cos(a), cy + R * Math.sin(a)]; });
    return pos;
  }
  function drawGraph() {
    var svg = $("pgx-graph"), NS = "http://www.w3.org/2000/svg", V = bat.view(S.tenant, S.slider == null ? null : S.slider);
    svg.textContent = "";
    var keys = Object.keys(V.nodes).sort();
    if (!keys.length) { var t = document.createElementNS(NS, "text"); t.setAttribute("x", 190); t.setAttribute("y", 150); t.setAttribute("text-anchor", "middle"); t.setAttribute("class", "pgx-svgdim"); t.textContent = "this tenant has no memory yet"; svg.append(t); return; }
    var pos = layout(keys);
    V.edges.forEach(function (e) {
      var a = pos[e.from], b = pos[e.to], l = document.createElementNS(NS, "line");
      l.setAttribute("x1", a[0]); l.setAttribute("y1", a[1]); l.setAttribute("x2", b[0]); l.setAttribute("y2", b[1]);
      l.setAttribute("class", "pgx-edge" + (e.rel === "SUPERSEDES" ? " sup" : "")); svg.append(l);
      var m = document.createElementNS(NS, "text"); m.setAttribute("x", (a[0] + b[0]) / 2); m.setAttribute("y", (a[1] + b[1]) / 2); m.setAttribute("text-anchor", "middle"); m.setAttribute("class", "pgx-edgel"); m.textContent = e.rel; svg.append(m);
    });
    keys.forEach(function (k) {
      var p = pos[k], n = V.nodes[k].node, g = document.createElementNS(NS, "g");
      var c = document.createElementNS(NS, "circle"); c.setAttribute("cx", p[0]); c.setAttribute("cy", p[1]); c.setAttribute("r", 14);
      c.setAttribute("class", "pgx-node" + (n.status === "superseded" ? " sup" : "") + (S.hl[k] ? " hl" : "") + (S.node === k ? " pick" : "")); g.append(c);
      var t = document.createElementNS(NS, "text"); t.setAttribute("x", p[0]); t.setAttribute("y", p[1] + 28); t.setAttribute("text-anchor", "middle"); t.setAttribute("class", "pgx-nodel"); t.textContent = k; g.append(t);
      g.setAttribute("tabindex", "0"); g.setAttribute("role", "button"); g.style.cursor = "pointer";
      (function (key) { var f = function () { pickNode(key); }; g.addEventListener("click", f); g.addEventListener("keydown", function (e) { if (e.key === "Enter") f(); }); })(k);
      var ti = document.createElementNS(NS, "title"); ti.textContent = n.title + " (" + n.status + ")"; g.append(ti);
      svg.append(g);
    });
  }
  function drawSlider() {
    var s = $("pgx-ver"), max = bat.ver; s.max = String(max); s.disabled = max < 2;
    if (S.slider == null) s.value = String(max);
    $("pgx-verl").textContent = (S.slider == null ? "now · version " + max : "viewing version " + S.slider + " of " + max);
  }

  function run(name, fn, req, tag) { var res = fn(); return record(name, req, res, tag); }
  var api = {
    remember: function (n, tag) { return run("remember", function () { return bat.put(S.tenant, n); }, n, tag); },
    link: function (r, f, t, tag) { return run("link", function () { return bat.link(S.tenant, { rel: r, from: f, to: t }); }, { rel: r, from: f, to: t }, tag); },
    recall: function (q, asOf, tag) { var o = { max_hops: 2, max_nodes: 8 }; if (asOf != null) o.as_of = asOf; var req = Object.assign({ q: q }, o); return run("recall", function () { return bat.recall(S.tenant, q, o); }, req, tag); },
    forget: function (k, tag) { return run("forget", function () { return bat.forget(S.tenant, k); }, { key: k }, tag); },
    supersede: function (oldK, n, tag) { return run("supersede", function () { return bat.supersede(S.tenant, oldK, n); }, { old: oldK, node: n }, tag); }
  };

  var STEPS = [
    { t: "Remember", d: "Write four engineering decisions and how they relate.", go: function () { say("agent", "I will write down what the team decided, so I can recall it later."); return wait(300).then(function () { NODES.forEach(function (n) { api.remember(n, "step 1"); }); EDGES.forEach(function (e) { api.link(e[0], e[1], e[2], "step 1"); }); S.v1 = bat.ver; }); } },
    { t: "Recall", d: "Ask a question; get the memories and how they connect.", go: function () { say("agent", "A teammate asks why we use group commit. I recall instead of guessing."); return wait(300).then(function () { api.recall(Q, null, "step 2"); }); } },
    { t: "Change a decision", d: "Replace one decision. The old one is kept and marked.", go: function () { say("agent", "The decision changed. I supersede it; the old memory is kept, never overwritten."); return wait(300).then(function () { if (S.v1 == null) S.v1 = bat.ver; api.supersede("adr-1", { key: "adr-5", title: "Use per-core WAL shards", body: "Shard the log per core to remove the single fsync queue.", kind: "decision" }, "step 3"); }); } },
    { t: "Time travel", d: "Ask the same question now and as of the earlier version.", go: function () { say("agent", "What do I recall now, and what did I believe before the change?"); return wait(300).then(function () { api.recall(Q, null, "now"); var v = S.v1 != null ? S.v1 : Math.max(0, bat.ver - 2); api.recall(Q, v, "as of v" + v); }); } },
    { t: "Isolation", d: "Another tenant asks the same question. It must see nothing.", go: function () { say("agent", "Team B asks the same question with its own key. It must see none of this."); return wait(300).then(function () { var keep = S.tenant; S.tenant = "team-b"; var c = api.recall(Q, null, "step 5"); S.tenant = keep; var n = c.res.nodes.length; say("result", n === 0 ? "Team B received 0 memories. Isolation held." : "Team B received " + n + " memories. That would be a leak."); drawGraph(); }); } }
  ];

  function buildSteps() {
    var ol = $("pgx-steps");
    STEPS.forEach(function (s, i) {
      var li = el("li"), b = el("button", "pgx-step"); b.type = "button";
      b.append(el("b", "", String(i + 1)), el("span", "", s.t), el("small", "", s.d));
      b.addEventListener("click", function () { if (S.busy) return; S.busy = true; Promise.resolve(s.go()).then(function () { S.busy = false; }); });
      li.append(b); ol.append(li);
    });
  }
  function resetAll() { bat = new Bat(); S.calls = []; S.sel = null; S.slider = null; S.hl = {}; S.node = null; S.v1 = null; $("pgx-feed").textContent = ""; var e = el("div", "pgx-empty", "Press a step on the left, or run the whole story."); e.id = "pgx-empty"; $("pgx-feed").append(e); $("pgx-insp").textContent = "Select a call, or click a node in the graph."; drawGraph(); drawSlider(); stats(); }


  function pickNode(k) {
    S.node = k; var V = bat.view(S.tenant, S.slider), n = V.nodes[k]; drawGraph();
    var box = $("pgx-insp"); box.textContent = "";
    if (!n) { box.append(el("p", "pgx-dim", k + " is not visible at this version.")); return; }
    var hist = bat.T(S.tenant).nodes[k].map(function (h) { return "v" + h.ver + " " + h.node.status; }).join("  ->  ");
    box.append(el("div", "pgx-who", "memory " + k), pre({ title: n.node.title, kind: n.node.kind, status: n.node.status, body: n.node.body, history: hist }));
    var row = el("div", "pgx-run");
    var sup = el("button", "pgx-btn ghost", "Supersede"); sup.type = "button";
    sup.addEventListener("click", function () { $("pgx-title").value = "Update to: " + n.node.title; $("pgx-body").value = ""; $("pgx-sup").value = k; $("pgx-body").focus(); say("you", "Fill in the replacement for " + k + " and press Save. The old memory is kept and marked superseded."); });
    var fg = el("button", "pgx-btn ghost", "Forget"); fg.type = "button";
    fg.addEventListener("click", function () { api.forget(k, "you"); S.node = null; });
    row.append(sup, fg); box.append(row);
  }
  function stats() {
    var V = bat.view(S.tenant, null), live = 0, k;
    for (k in V.nodes) if (V.nodes[k].node.status !== "superseded") live++;
    $("pgx-stats").textContent = S.calls.length + " calls  |  " + live + " live memories  |  log version " + bat.ver + "  |  tenant " + S.tenant;
  }
  function codeFor(c) {
    return "# Bat DB memory API (illustrative)\nimport batdb\ndb = batdb.connect(tenant=\"" + c.tenant + "\")\nresult = db." + c.name + "(" + JSON.stringify(c.req, null, 2) + ")\nprint(result)\n";
  }
  function copyLast() {
    var c = S.calls[S.sel ? S.sel - 1 : S.calls.length - 1]; if (!c) return;
    var t = codeFor(c);
    (navigator.clipboard ? navigator.clipboard.writeText(t) : Promise.reject()).then(function () { say("you", "Copied the call as Python."); }, function () { say("you", "Copy blocked by the browser; the call is in the inspector."); });
  }

  /* 40 second guided demo: absolute timeline, stoppable */
  var DEMO_MS = 40000, timers = [], clock = null, t0 = 0, playing = false;
  function cap(t) { $("pgx-cap").textContent = t; }
  function mark(i) { document.querySelectorAll(".pgx-step").forEach(function (b, j) { b.classList.toggle("now", j === i); }); }
  function demoPlan() {
    var P = [];
    function at(ms, f) { P.push([ms, f]); }
    at(0, function () { resetAll(); cap("An agent that forgets repeats its mistakes. Watch it remember."); mark(-1); });
    NODES.forEach(function (n, i) { at(3000 + i * 1500, function () { cap(i === 0 ? "Step 1. Remember: each decision is written once and gets a log version." : "Remember: " + n.title); mark(0); api.remember(n, "demo"); }); });
    EDGES.forEach(function (e, i) { at(9200 + i * 700, function () { cap("Link the decisions so a question can walk from one to the next."); api.link(e[0], e[1], e[2], "demo"); if (i === 2) S.v1 = bat.ver; }); });
    at(12500, function () { mark(1); cap("Step 2. Recall: ask in plain words. The nearest memory is a seed; its links pull in the rest."); $("pgx-q").value = Q; api.recall(Q, null, "demo"); });
    at(18500, function () { mark(2); cap("Step 3. The team changes its mind. Supersede keeps the old decision and marks it."); api.supersede("adr-1", { key: "adr-5", title: "Use per-core WAL shards", body: "Shard the log per core to remove the single fsync queue.", kind: "decision" }, "demo"); });
    at(22500, function () { cap("Same question, now. The newest decision leads; the old one is struck through."); api.recall(Q, null, "demo"); });
    at(26000, function () { mark(3); cap("Step 4. Time travel: drag the log back to what we believed before the change."); S.slider = S.v1; drawGraph(); drawSlider(); });
    at(28000, function () { cap("Pinned to version " + S.v1 + ": the old answer comes back unchanged."); api.recall(Q, S.v1, "as of v" + S.v1); });
    at(32000, function () { S.slider = null; drawGraph(); drawSlider(); });
    at(33000, function () { mark(4); cap("Step 5. Isolation: another tenant asks the same question with its own key."); var k = S.tenant; S.tenant = "team-b"; var c = api.recall(Q, null, "demo"); S.tenant = k; say("result", c.res.nodes.length === 0 ? "Team B received 0 memories. Isolation held." : "LEAK: team B received memories."); });
    at(37500, function () { cap("Remember. Recall. Revise. Rewind. Isolate. That is the whole API. Now try it yourself."); mark(-1); });
    at(DEMO_MS, function () { endDemo(); });
    return P;
  }
  function startDemo() {
    stopDemo(true); playing = true; t0 = performance.now(); S.busy = true;
    $("pgx-play").textContent = "Stop demo"; $("pgx-prog").style.width = "0";
    demoPlan().forEach(function (e) { timers.push(setTimeout(e[1], e[0])); });
    clock = setInterval(function () { var t = Math.min(DEMO_MS, performance.now() - t0); $("pgx-prog").style.width = (t / DEMO_MS * 100) + "%"; $("pgx-time").textContent = Math.ceil((DEMO_MS - t) / 1000) + "s"; }, 120);
  }
  function stopDemo(silent) { timers.forEach(clearTimeout); timers = []; clearInterval(clock); playing = false; S.busy = false; $("pgx-play").textContent = "Play 40s demo"; if (!silent) { $("pgx-time").textContent = ""; mark(-1); } }
  function endDemo() { stopDemo(); $("pgx-prog").style.width = "100%"; $("pgx-time").textContent = "done"; }

  function save() {
    var t = $("pgx-title").value.trim(), b = $("pgx-body").value.trim(), old = $("pgx-sup").value;
    if (!t) { $("pgx-title").focus(); return; }
    var key = "mem-" + (bat.ver + 1), node = { key: key, title: t, body: b || t, kind: "note" };
    if (old) { api.supersede(old, node, "you"); $("pgx-sup").value = ""; } else api.remember(node, "you");
    S.node = key; $("pgx-title").value = ""; $("pgx-body").value = "";
  }

  function init() {
    if (!$("pgx-feed")) return;
    buildSteps();
    $("pgx-play").addEventListener("click", function () { if (playing) { stopDemo(); cap("Demo stopped. Everything is still live: click around."); } else startDemo(); });
    $("pgx-all").addEventListener("click", function () { if (S.busy) return; resetAll(); S.busy = true; var p = Promise.resolve(); STEPS.forEach(function (st) { p = p.then(function () { return st.go(); }).then(function () { return wait(500); }); }); p.then(function () { S.busy = false; }); });
    $("pgx-save").addEventListener("click", save);
    $("pgx-copy").addEventListener("click", copyLast);
    document.querySelectorAll(".pgx-chip").forEach(function (c) { c.addEventListener("click", function () { $("pgx-q").value = c.textContent; $("pgx-ask").requestSubmit(); }); });
    document.addEventListener("keydown", function (e) { if (e.key === "/" && !/INPUT|TEXTAREA/.test(document.activeElement.tagName)) { e.preventDefault(); $("pgx-q").focus(); } });
    $("pgx-reset").addEventListener("click", function () { if (!S.busy) resetAll(); });
    document.querySelectorAll("[name=pgx-tenant]").forEach(function (r) { r.addEventListener("change", function () { S.tenant = r.value; S.slider = null; S.hl = {}; drawGraph(); drawSlider(); }); });
    $("pgx-ask").addEventListener("submit", function (e) { e.preventDefault(); var q = $("pgx-q").value.trim(); if (!q) return; S.lastQ = q; api.recall(q, S.slider, "ask"); });
    $("pgx-ver").addEventListener("input", function () { var v = +$("pgx-ver").value; S.slider = v >= bat.ver ? null : v; drawGraph(); drawSlider(); });
    resetAll();
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init); else init();
})();
