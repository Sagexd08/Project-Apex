// Prism lab pages: dropdown, mobile menu, contents scrollspy, reading progress. No dependencies.
(() => {
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];

  const dd = $('.lb-dd'), menu = $('.lb-menu'), nav = $('.lb-nav'), burger = $('.lb-burger');
  const setMenu = (open) => {
    if (!dd || !menu) return;
    dd.setAttribute('aria-expanded', String(open));
    menu.classList.toggle('open', open);
  };
  if (dd) {
    dd.addEventListener('click', () => setMenu(dd.getAttribute('aria-expanded') !== 'true'));
    document.addEventListener('click', (e) => { if (!e.target.closest('.lb-nav li')) setMenu(false); });
    document.addEventListener('keydown', (e) => { if (e.key === 'Escape') { setMenu(false); dd.focus(); } });
  }
  if (burger && nav) {
    burger.addEventListener('click', () => {
      const open = !nav.classList.contains('open');
      nav.classList.toggle('open', open);
      burger.setAttribute('aria-expanded', String(open));
      if (open) setMenu(true);
    });
  }

  // Contents: highlight the section nearest the top.
  const links = $$('.toc a');
  const secs = links.map((a) => $(a.getAttribute('href'))).filter(Boolean);
  if ('IntersectionObserver' in window && secs.length) {
    const io = new IntersectionObserver((entries) => {
      entries.forEach((en) => {
        if (!en.isIntersecting) return;
        links.forEach((a) => a.classList.toggle('on', a.getAttribute('href') === '#' + en.target.id));
      });
    }, { rootMargin: '-20% 0px -70% 0px' });
    secs.forEach((s) => io.observe(s));
  }

  // Reading progress.
  const bar = $('.lb-prog');
  let ticking = false;
  const paint = () => {
    const h = document.documentElement;
    const max = h.scrollHeight - h.clientHeight;
    if (bar) bar.style.transform = 'scaleX(' + (max > 0 ? Math.min(1, h.scrollTop / max) : 0) + ')';
    ticking = false;
  };
  addEventListener('scroll', () => { if (!ticking) { ticking = true; requestAnimationFrame(paint); } }, { passive: true });
  paint();
})();
