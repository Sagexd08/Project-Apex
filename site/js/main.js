import { createSpiral } from './spiral.js';
import { createDivider, createGlobe } from './ascii.js';
import { buildOdoLabels, createNumber, initHold, initMagnetic, initDrawer, typeLines } from './ui.js';

const { gsap, ScrollTrigger, Lenis } = window;
gsap.registerPlugin(ScrollTrigger);
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];

const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
const wide = matchMedia('(min-width: 861px)');
const sticky = wide.matches && !reduce;
const clamp01 = (v) => Math.min(1, Math.max(0, v));
const seg = (p, a, b) => clamp01((p - a) / (b - a));
const easeIO = (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);
if (!sticky) document.documentElement.classList.add('static');
wide.addEventListener('change', () => location.reload());
window.__ready = true;

let lenis = null;
if (!reduce && Lenis) {
  lenis = new Lenis({ lerp: 0.1, smoothWheel: true });
  lenis.on('scroll', ScrollTrigger.update);
  gsap.ticker.add((t) => lenis.raf(t * 1000));
  gsap.ticker.lagSmoothing(0);
}
ScrollTrigger.config({ ignoreMobileResize: true });

const stage = $('#top');
$$('a[href^="#"]').forEach((a) => a.addEventListener('click', (e) => {
  const id = a.getAttribute('href').slice(1);
  const el = id === 'top' ? document.body : document.getElementById(id);
  if (!el) return;
  e.preventDefault();
  let y;
  if (a.dataset.p && sticky) y = stage.offsetTop + Number(a.dataset.p) * (stage.offsetHeight - innerHeight);
  else if (a.dataset.p) y = $('#reveal').getBoundingClientRect().top + scrollY - 60;
  else y = id === 'top' ? 0 : el.getBoundingClientRect().top + scrollY - (id === 'faq' ? 80 : 0);
  if (lenis) lenis.scrollTo(y, { duration: 1.2, easing: easeIO });
  else window.scrollTo({ top: y, behavior: reduce ? 'auto' : 'smooth' });
}));

buildOdoLabels();
initHold();
initMagnetic();
initDrawer({ lenis, reduce });
const setProgress = gsap.quickSetter($('#progress'), 'scaleX');
ScrollTrigger.create({ start: 0, end: 'max', onUpdate: (s) => setProgress(s.progress) });

const copy = $('#copy'), reveal = $('#reveal'), roll = $('#roll'), ledger = $('#ledger');
const cue = $('.cue');
const groups = $$('.grp', roll);
const big = createNumber($('#bigNum'), groups[0].dataset.n);
let spiral = null, activeG = 0, rollMax = 0, bounds = [], lastP = 0;
groups[0].classList.add('act');

function setGroup(i) {
  if (i === activeG) return;
  activeG = i;
  const g = groups[i];
  big.set(g.dataset.n);
  $('#bigUnit').textContent = g.dataset.unit;
  $('#bigTag').textContent = g.dataset.tag;
  $('#bigCap').textContent = g.dataset.cap;
  groups.forEach((x, k) => x.classList.toggle('act', k === i));
}
function measure() {
  rollMax = Math.max(0, roll.scrollHeight - ledger.clientHeight);
  bounds = groups.map((g) => g.offsetTop);
}
const navProblem = $('.nav a[href="#problem"]');
function applyStage(p) {
  lastP = p;
  const d = easeIO(seg(p, 0, 0.3));
  const c = easeIO(seg(p, 0.24, 0.42));
  const r = seg(p, 0.44, 0.96);
  spiral?.dive(d);
  copy.style.transform = `translate3d(${(-d * 100).toFixed(2)}%,0,0)`;
  cue.style.opacity = String(1 - clamp01(d * 2.2));
  const R = Math.hypot(innerWidth, innerHeight) * 0.51;
  reveal.style.clipPath = `circle(${(c * R).toFixed(1)}px at 50% 50%)`;
  reveal.style.visibility = c > 0.005 ? 'visible' : 'hidden';
  spiral?.pause(c >= 0.995);
  roll.style.transform = `translate3d(0,${(-r * rollMax).toFixed(1)}px,0)`;
  const mid = r * rollMax + ledger.clientHeight * 0.4;
  let gi = 0;
  bounds.forEach((t, k) => { if (mid >= t - 30) gi = k; });
  setGroup(gi);
  navProblem?.classList.toggle('on', p > 0.4 && p < 0.99);
}
if (sticky) {
  ScrollTrigger.create({
    trigger: stage, start: 'top top', end: 'bottom bottom',
    onUpdate: (s) => applyStage(s.progress),
    onRefresh: (s) => { measure(); applyStage(s.progress); },
  });
}

const boot = async () => {
  const nav = $('#nav'), lines = $$('.hero-copy h1 .ln > span'), ins = $$('[data-in]');
  document.documentElement.classList.add('ready');
  gsap.set(lines, { yPercent: reduce ? 0 : 110 });
  $('.hero-copy h1').style.visibility = 'visible';
  if (!reduce) {
    const tl = gsap.timeline({ defaults: { ease: 'expo.out' } });
    tl.fromTo(nav, { yPercent: -150, autoAlpha: 1 }, { yPercent: 0, duration: 1.1 }, 0.2)
      .to(lines, { yPercent: 0, duration: 1.3, stagger: 0.11 }, 0.25)
      .fromTo(ins, { y: 22, opacity: 0 }, { y: 0, opacity: 1, duration: 1, stagger: 0.09 }, 0.45)
;
  } else { gsap.set(nav, { autoAlpha: 1 }); }

  createSpiral($('#gl'), { ticker: gsap.ticker, reduce }).then((sp) => {
    spiral = sp;
    if (!sp) return;
    const o = { v: 0 };
    gsap.to(o, { v: 1, duration: reduce ? 0 : 1.8, delay: 0.15, ease: 'power2.out', onUpdate: () => sp.show(o.v) });
    applyStage(lastP);
  }).catch(() => {});
};
Promise.race([document.fonts.ready, new Promise((r) => setTimeout(r, 1500))]).then(boot);

$$('.ascii-div').forEach((cv) => {
  cv.style.setProperty('--from', cv.dataset.from);
  const dv = createDivider(cv, cv.dataset.from, cv.dataset.to);
  dv.draw(0.5);
  ScrollTrigger.create({ trigger: cv, start: 'top bottom', end: 'bottom top', onUpdate: (s) => dv.draw(s.progress) });
});
createGlobe($('#globe'), gsap.ticker, reduce);

ScrollTrigger.batch('[data-rv]', {
  start: 'top 90%', once: true,
  onEnter: (els) => (reduce
    ? gsap.set(els, { opacity: 1 })
    : gsap.fromTo(els, { y: 48, opacity: 0 }, { y: 0, opacity: 1, duration: 1.15, ease: 'expo.out', stagger: 0.09, overwrite: true })),
});
if (!reduce) {
  gsap.fromTo('#ide', { y: 90, rotateX: 7, transformPerspective: 1400 }, {
    y: 0, rotateX: 0, ease: 'none',
    scrollTrigger: { trigger: '#ide', start: 'top bottom', end: 'top 35%', scrub: true },
  });
}
$$('.track li,.faq details,.prin article,.bench tbody tr').forEach((el, i) => el.style.setProperty('--i', i % 6));

$$('[data-count]').forEach((el) => {
  const to = Number(el.dataset.count);
  ScrollTrigger.create({
    trigger: el, start: 'top 92%', once: true,
    onEnter: () => {
      if (reduce) return;
      const o = { v: 0 };
      gsap.to(o, { v: to, duration: 1.8, ease: 'power3.out', onUpdate: () => { el.textContent = Math.round(o.v).toLocaleString('en-US'); } });
    },
  });
});
ScrollTrigger.create({
  trigger: '#ide', start: 'top 62%', once: true,
  onEnter: () => typeLines($('#term'), [
    { t: '$ bash scripts/verify.sh', cls: 'p', pause: 260 },
    { t: 'fmt ok · clippy -D warnings ok', fast: true },
    { t: 'tests: 141 passed · 0 failed · 2 ignored', cls: 'ok', fast: true },
    { t: 'production graph: 54 ok · 0 fail', cls: 'ok', fast: true },
    { t: 'digest 3cdaf426a42aef0a7e9133052d3d69df', fast: true },
    { t: 'gate 2: OPEN · independent review pending', cls: 'ac' },
  ], { reduce }),
});

const navLinks = $$('.nav a.nl').filter((a) => a.getAttribute('href') !== '#problem');
const active = new Map();
navLinks.forEach((a) => {
  const el = document.getElementById(a.getAttribute('href').slice(1));
  if (!el) return;
  ScrollTrigger.create({
    trigger: el, start: 'top 55%', end: 'bottom 55%',
    onToggle: (s) => { active.set(a, s.isActive); const on = [...navLinks].reverse().find((l) => active.get(l)); navLinks.forEach((l) => l.classList.toggle('on', l === on)); },
  });
});
if (!sticky) ScrollTrigger.create({ trigger: '#reveal', start: 'top 55%', end: 'bottom 55%', onToggle: (s) => navProblem?.classList.toggle('on', s.isActive) });

addEventListener('load', () => ScrollTrigger.refresh());

const chip = $('#chip');
if (sticky && chip && lenis) {
  const yAt = (p) => stage.offsetTop + p * (stage.offsetHeight - innerHeight);
  const HOLD_MS = 3000, HOLD_P = 0.3, OPEN_P = 0.48;
  let holding = false, t0 = 0, from = 0, raf = 0;
  const label = (t) => { if (chip.textContent !== t) chip.textContent = t; };
  const inHero = () => scrollY < yAt(0.02) + 4 && !document.documentElement.classList.contains('modal');
  addEventListener('pointermove', (e) => {
    const show = inHero() || holding;
    chip.classList.toggle('on', show && !e.target.closest('a,button,.nav'));
    chip.style.transform = `translate3d(${e.clientX + 16}px,${e.clientY + 16}px,0)`;
  }, { passive: true });
  const step = (now) => {
    if (!holding) return;
    const k = Math.min(1, (now - t0) / HOLD_MS);
    lenis.scrollTo(from + (yAt(HOLD_P) - from) * easeIO(k), { immediate: true });
    label(k >= 1 ? 'Release' : 'Keep holding');
    raf = requestAnimationFrame(step);
  };
  $('#top').addEventListener('pointerdown', (e) => {
    if (e.target.closest('a,button') || !inHero()) return;
    holding = true; t0 = performance.now(); from = scrollY; label('Keep holding');
    raf = requestAnimationFrame(step);
  });
  const end = () => {
    if (!holding) return;
    holding = false; cancelAnimationFrame(raf);
    const reached = lastP >= HOLD_P - 0.005;
    lenis.scrollTo(reached ? yAt(OPEN_P) : 0, { duration: reached ? 1.4 : 0.8, easing: easeIO });
    label('Click & hold');
  };
  addEventListener('pointerup', end); addEventListener('pointercancel', end);
}

const vm = $('#vizMem');
if (vm) {
  const cells = Array.from({ length: 240 }, () => document.createElement('i'));
  vm.append(...cells);
  if (reduce) cells.forEach((c) => c.classList.add('on'));
  else ScrollTrigger.create({
    trigger: vm, start: 'top 80%', once: true,
    onEnter: () => {
      const o = { v: 0 };
      gsap.to(o, { v: cells.length, duration: 2.2, ease: 'power2.inOut', onUpdate: () => { const n = Math.floor(o.v); for (let i = 0; i < n; i++) cells[i].classList.add('on'); } });
    },
  });
}

for (const anat of document.querySelectorAll('#anatomy, #manifesto')) {
  new IntersectionObserver(([e], io) => { if (e.isIntersecting) { anat.classList.add('in'); io.disconnect(); } }, { threshold: 0.25 }).observe(anat);
}
