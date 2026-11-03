// Shared page behaviour: mega-menu, burger, scroll progress, reveals, copy buttons.
// Every link is a plain <a href>: navigation never depends on this file.
(() => {
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;

  // Mega-menu: hover and focus are handled by CSS; the arrow button toggles for touch and keyboard.
  const dds = $$('.gx-dd, li.has-dd');
  const setOpen = (dd, open) => {
    dd.classList.toggle('open', open);
    const b = $('.gx-arrow', dd);
    if (b) b.setAttribute('aria-expanded', String(open));
  };
  dds.forEach((dd) => {
    const b = $('.gx-arrow', dd);
    if (b) b.addEventListener('click', (e) => { e.stopPropagation(); setOpen(dd, !dd.classList.contains('open')); });
  });
  document.addEventListener('click', (e) => dds.forEach((dd) => { if (!dd.contains(e.target)) setOpen(dd, false); }));
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape') return;
    dds.forEach((dd) => { if (dd.classList.contains('open')) { setOpen(dd, false); $('.gx-arrow', dd)?.focus(); } });
  });

  const burger = $('.gx-burger'), gx = $('#gx');
  if (burger && gx) burger.addEventListener('click', () => {
    const open = !gx.classList.contains('open');
    gx.classList.toggle('open', open);
    burger.setAttribute('aria-expanded', String(open));
  });

  // Scroll progress (one passive listener, rAF-throttled, transform only).
  const bar = $('#gx-prog');
  if (bar) {
    let tick = false;
    const upd = () => {
      tick = false;
      const h = document.documentElement.scrollHeight - innerHeight;
      bar.style.transform = `scaleX(${h > 0 ? Math.min(1, scrollY / h) : 0})`;
    };
    addEventListener('scroll', () => { if (!tick) { tick = true; requestAnimationFrame(upd); } }, { passive: true });
    upd();
  }

  // Reveals.
  const rv = $$('.reveal-in');
  if (rv.length) {
    if (reduce || !('IntersectionObserver' in window)) rv.forEach((n) => n.classList.add('on'));
    else {
      const io = new IntersectionObserver((es) => es.forEach((en) => { if (en.isIntersecting) { en.target.classList.add('on'); io.unobserve(en.target); } }), { rootMargin: '0px 0px -8% 0px' });
      rv.forEach((n, i) => { n.style.transitionDelay = `${(i % 4) * 70}ms`; io.observe(n); });
    }
  }

  // Reeded-glass background on inner pages (WebGL; the CSS mesh is the fallback). Not on the paper sheet.
  if (document.body.classList.contains('gxp') && !document.body.classList.contains('pg-paper')) {
    if (false) import('./reeded.js').then((m) => { if (m.createReeded({ reduce })) document.body.classList.add('rd-on'); }).catch(() => {});
  }

  // Paper: table-of-contents highlight and "Save as PDF" (browser print dialog -> PDF).
  const toc = $$('.pp-toc a');
  if (toc.length && 'IntersectionObserver' in window) {
    const map = new Map(toc.map((a) => [a.getAttribute('href').slice(1), a]));
    const io = new IntersectionObserver((es) => es.forEach((en) => {
      if (en.isIntersecting) { toc.forEach((a) => a.classList.remove('on')); map.get(en.target.id)?.classList.add('on'); }
    }), { rootMargin: '-20% 0px -70% 0px' });
    map.forEach((_, id) => { const el = document.getElementById(id); if (el) io.observe(el); });
  }
  $$('[data-print]').forEach((b) => b.addEventListener('click', () => print()));

  // Copy buttons.
  $$('[data-copy]').forEach((b) => b.addEventListener('click', async () => {
    const t = document.querySelector(b.dataset.copy)?.innerText?.trim();
    if (!t) return;
    const old = b.textContent;
    try { await navigator.clipboard.writeText(t); b.textContent = 'Copied'; } catch { b.textContent = 'Press Ctrl+C'; }
    setTimeout(() => { b.textContent = old; }, 1600);
  }));
})();
