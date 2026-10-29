const gsap = window.gsap;
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];

export function buildOdoLabels(root = document) {
  $$('.odo', root).forEach((el) => {
    const text = el.textContent;
    el.setAttribute('aria-label', text);
    el.textContent = '';
    [...text].forEach((c, i) => {
      const g = c === ' ' ? '&nbsp;' : c;
      const ch = document.createElement('span');
      ch.className = 'ch'; ch.setAttribute('aria-hidden', 'true'); ch.style.setProperty('--i', i);
      ch.innerHTML = `<span><i>${g}</i><i>${g}</i></span>`;
      el.appendChild(ch);
    });
  });
}

export function createNumber(el, initial) {
  let sig = '';
  const build = (str) => {
    el.textContent = '';
    [...str].forEach((c) => {
      const dg = document.createElement('span'); dg.className = 'dg';
      if (/\d/.test(c)) {
        const col = document.createElement('span');
        col.innerHTML = '0123456789'.split('').map((d) => `<i>${d}</i>`).join('');
        dg.appendChild(col); dg.dataset.d = '1';
      } else { dg.innerHTML = `<i>${c}</i>`; }
      el.appendChild(dg);
    });
  };
  const set = (str, instant = false) => {
    const s = [...str].map((c) => (/\d/.test(c) ? 'd' : c)).join('');
    if (s !== sig) { build(str); sig = s; instant = true; }
    el.setAttribute('aria-label', str);
    $$('.dg[data-d] > span', el).forEach((col, i) => {
      const digit = [...str].filter((c) => /\d/.test(c))[i];
      gsap.to(col, { yPercent: -10 * Number(digit), duration: instant ? 0 : 1.1, ease: 'expo.out', delay: instant ? 0 : i * 0.05, overwrite: true });
    });
  };
  set(initial, true);
  return { set };
}

export function initHold(root = document) {
  $$('.btn[data-hold]', root).forEach((btn) => {
    const prog = { v: 0 };
    let tween;
    const down = () => {
      btn.classList.add('holding');
      gsap.to(btn, { scale: 0.955, duration: 0.2, ease: 'power3.out' });
      tween?.kill();
      tween = gsap.to(prog, { v: 1, duration: 0.9, ease: 'power2.inOut', onUpdate: () => btn.style.setProperty('--hold', prog.v) });
    };
    const up = () => {
      tween?.kill();
      gsap.to(btn, { scale: 1, duration: 0.6, ease: 'elastic.out(1,.5)' });
      tween = gsap.to(prog, { v: 0, duration: 0.35, ease: 'power3.out', onUpdate: () => btn.style.setProperty('--hold', prog.v), onComplete: () => btn.classList.remove('holding') });
    };
    btn.addEventListener('pointerdown', down);
    ['pointerup', 'pointerleave', 'pointercancel'].forEach((e) => btn.addEventListener(e, up));
  });
}

export function initMagnetic(root = document) {
  if (!matchMedia('(hover:hover) and (pointer:fine)').matches) return;
  $$('.btn', root).forEach((b) => {
    const qx = gsap.quickTo(b, 'x', { duration: 0.5, ease: 'power3.out' });
    const qy = gsap.quickTo(b, 'y', { duration: 0.5, ease: 'power3.out' });
    b.addEventListener('pointermove', (e) => {
      const r = b.getBoundingClientRect();
      qx((e.clientX - r.left - r.width / 2) * 0.16); qy((e.clientY - r.top - r.height / 2) * 0.24);
    });
    b.addEventListener('pointerleave', () => { qx(0); qy(0); });
  });
}

export function initDrawer({ lenis, reduce }) {
  const btn = $('#w-btn'), drawer = $('#drawer'), scrim = $('#scrim'), ring = $('#w-btn circle');
  const closeBtn = $('#d-close'), tabs = $$('#d-tabs [role=tab]'), panes = $$('.pane');
  let isOpen = false, heldOpen = false, holdTween;
  const FOCUSABLE = 'button,[href],[tabindex]:not([tabindex="-1"])';

  const selectTab = (i, focus = false) => {
    tabs.forEach((t, k) => { t.setAttribute('aria-selected', k === i); t.tabIndex = k === i ? 0 : -1; });
    panes.forEach((p, k) => p.classList.toggle('on', k === i));
    if (focus) tabs[i].focus();
    if (!reduce) gsap.fromTo(panes[i].children, { y: 14, opacity: 0 }, { y: 0, opacity: 1, duration: 0.6, stagger: 0.06, ease: 'expo.out' });
  };
  tabs.forEach((t, i) => {
    t.addEventListener('click', () => selectTab(i));
    t.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowDown' || e.key === 'ArrowRight') { e.preventDefault(); selectTab((i + 1) % tabs.length, true); }
      if (e.key === 'ArrowUp' || e.key === 'ArrowLeft') { e.preventDefault(); selectTab((i - 1 + tabs.length) % tabs.length, true); }
    });
  });

  function open(burst) {
    if (isOpen) return;
    isOpen = true; lenis?.stop();
    btn.setAttribute('aria-expanded', 'true');
    scrim.style.visibility = 'visible'; drawer.style.visibility = 'visible';
    selectTab(0);
    if (reduce) { gsap.set([scrim, drawer], { opacity: 1, clipPath: 'none' }); }
    else {
      gsap.to(scrim, { opacity: 1, duration: 0.5, ease: 'power2.out' });
      gsap.fromTo(drawer, { clipPath: 'circle(0% at 100% 100%)' }, { clipPath: 'circle(150% at 100% 100%)', duration: burst ? 1.05 : 0.85, ease: 'expo.out' });
      gsap.fromTo([$('#d-tabs'), $('.d-body'), closeBtn], { y: 24, opacity: 0 }, { y: 0, opacity: 1, duration: 0.8, stagger: 0.08, delay: 0.2, ease: 'expo.out' });
    }
    setTimeout(() => closeBtn.focus(), 60);
  }
  function close() {
    if (!isOpen) return;
    isOpen = false; lenis?.start();
    btn.setAttribute('aria-expanded', 'false');
    const done = () => { drawer.style.visibility = 'hidden'; scrim.style.visibility = 'hidden'; btn.focus(); };
    if (reduce) return done();
    gsap.to(scrim, { opacity: 0, duration: 0.45 });
    gsap.to(drawer, { clipPath: 'circle(0% at 100% 100%)', duration: 0.6, ease: 'expo.inOut', onComplete: done });
  }

  const HOLD_MS = 700;
  btn.addEventListener('pointerdown', (e) => {
    if (e.button !== 0 || isOpen) return;
    heldOpen = false;
    gsap.to(btn, { scale: 0.9, duration: 0.25, ease: 'power3.out' });
    holdTween?.kill();
    holdTween = gsap.to(ring, { strokeDashoffset: 0, duration: HOLD_MS / 1000, ease: 'power1.in', onComplete: () => { heldOpen = true; open(true); } });
  });
  const release = () => {
    gsap.to(btn, { scale: 1, duration: 0.6, ease: 'elastic.out(1,.45)' });
    if (!heldOpen) { holdTween?.kill(); gsap.to(ring, { strokeDashoffset: 157, duration: 0.4, ease: 'power3.out' }); }
    else gsap.to(ring, { strokeDashoffset: 157, duration: 0.5, delay: 0.3 });
  };
  ['pointerup', 'pointerleave', 'pointercancel'].forEach((e) => btn.addEventListener(e, release));
  btn.addEventListener('click', () => { if (!heldOpen && !isOpen) open(false); heldOpen = false; });
  btn.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); open(false); } });

  closeBtn.addEventListener('click', close);
  scrim.addEventListener('click', close);
  addEventListener('keydown', (e) => {
    if (!isOpen) return;
    if (e.key === 'Escape') close();
    if (e.key === 'Tab') {
      const f = $$(FOCUSABLE, drawer).filter((n) => !n.disabled && n.offsetParent !== null);
      const first = f[0], last = f[f.length - 1];
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
    }
  });
  return { open, close };
}

export function createMinimap(el) {
  const W = 118, H = 68;
  const vp = document.createElement('div'); vp.className = 'vp';
  let docH = 1;
  const pick = '.hero-copy h1,.hero-copy .lead,.hero-copy .btn,.reveal h2,.ledger .row,.feat,.ide,.card,.track li,summary,.foot .word';
  function rebuild() {
    $$('i', el).forEach((n) => n.remove());
    docH = document.documentElement.scrollHeight;
    const frag = document.createDocumentFragment();
    $$(pick).forEach((n) => {
      const r = n.getBoundingClientRect(); const y = r.top + scrollY;
      if (r.height < 6 || r.width < 6) return;
      const i = document.createElement('i');
      const acc = n.matches('.btn');
      i.style.cssText = `left:${6 + Math.min(1, Math.max(0, r.left / innerWidth)) * (W - 14)}px;top:${(y / docH) * H}px;width:${Math.max(6, (Math.min(r.width, innerWidth) / innerWidth) * (W - 14))}px;height:${Math.max(1.5, (r.height / docH) * H * 0.55)}px;background:${acc ? '#a8b8a2' : '#a89c92'};opacity:${n.matches('.ide,.card') ? 0.35 : 0.8}`;
      frag.appendChild(i);
    });
    el.prepend(frag);
    el.appendChild(vp);
    move();
  }
  function move() {
    const vh = Math.max(8, (innerHeight / docH) * H);
    const top = Math.min(H - vh, (scrollY / docH) * H);
    vp.style.height = `${vh}px`; vp.style.transform = `translateY(${top}px)`;
  }
  return { rebuild, move };
}

export function typeLines(el, lines, { reduce }) {
  el.textContent = '';
  const cur = document.createElement('span'); cur.className = 'cur';
  const lineEls = lines.map((l) => { const d = document.createElement('div'); d.className = l.cls || ''; el.appendChild(d); return d; });
  if (reduce) { lines.forEach((l, i) => { lineEls[i].textContent = l.t; }); return; }
  let li = 0, ci = 0;
  const step = () => {
    if (li >= lines.length) return;
    const l = lines[li], d = lineEls[li];
    if (ci === 0) d.appendChild(cur);
    if (ci < l.t.length) {
      d.insertBefore(document.createTextNode(l.t[ci++]), cur);
      setTimeout(step, l.fast ? 5 : 22);
    } else { li++; ci = 0; setTimeout(step, l.pause ?? 140); }
  };
  step();
}
