// AgentDeck mobile audit: run with the browser tool `evaluate_script` (`sourcePath` = this file).
// Returns facts only: overflow and its culprits, tap targets (WCAG 2.2: fail under 24 px, warn under
// 44 px), input font under 16 px (iOS zooms on focus), text under 12 px, viewport meta problems,
// 100vh rules, fixed bars without safe-area insets.
() => {
  const width = document.documentElement.clientWidth;
  const name = (el) =>
    el.tagName.toLowerCase() +
    (el.id ? '#' + el.id : '') +
    (typeof el.className === 'string' && el.className.trim() ? '.' + el.className.trim().split(/\s+/).slice(0, 2).join('.') : '');
  const box = (el) => el.getBoundingClientRect();
  const shown = (el) => {
    const r = box(el);
    const s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none';
  };
  const size = (el) => Math.round(box(el).width) + 'x' + Math.round(box(el).height);
  const all = [...document.querySelectorAll('body *')].filter(shown);
  const first = (list, max = 15) => list.slice(0, max);

  // Overflow: the deepest elements sticking out (their parents are listed only if they stick out by themselves).
  const out = all.filter((el) => box(el).right > width + 1);
  const deepest = out.filter((el) => !out.some((other) => other !== el && el.contains(other)));

  // Tap targets: interactive elements, ignoring links inside running text.
  const targets = [...document.querySelectorAll('a[href], button, input:not([type=hidden]), select, textarea, summary, [role=button], [role=link], [onclick], [tabindex]:not([tabindex="-1"])')]
    .filter(shown)
    .filter((el) => !(el.tagName === 'A' && getComputedStyle(el).display === 'inline' && el.parentElement && /\S/.test(el.parentElement.textContent.replace(el.textContent, ''))));
  const smallest = (el) => Math.min(box(el).width, box(el).height);
  const tapFail = targets.filter((el) => smallest(el) < 24);
  const tapWarn = targets.filter((el) => smallest(el) >= 24 && smallest(el) < 44);

  const inputs = [...document.querySelectorAll('input:not([type=checkbox]):not([type=radio]):not([type=hidden]), select, textarea')].filter(shown);
  const textEls = all.filter((el) => [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim()));

  const meta = document.querySelector('meta[name=viewport]');
  const metaContent = meta ? meta.content.replace(/\s/g, '') : '';

  // CSS rules using 100vh (hidden behind mobile browser bars); cross-origin sheets are skipped.
  const vhRules = [];
  for (const sheet of document.styleSheets) {
    let rules;
    try { rules = sheet.cssRules; } catch { continue; }
    for (const rule of rules || []) {
      if (rule.style && /(^|[^\d.])100vh/.test(rule.cssText)) vhRules.push(rule.selectorText || rule.cssText.slice(0, 60));
    }
  }

  // Visible height without scrollbars (innerHeight includes a horizontal scrollbar).
  const height = document.documentElement.clientHeight;
  const fixedBars = all.filter((el) => {
    const s = getComputedStyle(el);
    return (s.position === 'fixed' || s.position === 'sticky') && (box(el).bottom >= height - 1 || box(el).top <= 1) && box(el).width >= width * 0.8;
  });
  const usesSafeArea = [...document.styleSheets].some((sheet) => {
    try { return [...sheet.cssRules].some((r) => r.cssText.includes('safe-area-inset')); } catch { return false; }
  });

  return {
    viewportWidth: width,
    userAgentMobile: /Mobi|iPhone|Android/.test(navigator.userAgent),
    horizontalOverflow: document.documentElement.scrollWidth > width,
    overflowCulprits: first(deepest.map((el) => name(el) + ' right=' + Math.round(box(el).right))),
    tapTargetsUnder24px: first(tapFail.map((el) => name(el) + ' ' + size(el))),
    tapTargetsUnder44px: first(tapWarn.map((el) => name(el) + ' ' + size(el))),
    inputsUnder16px: first(inputs.filter((el) => parseFloat(getComputedStyle(el).fontSize) < 16).map((el) => name(el) + ' ' + getComputedStyle(el).fontSize)),
    textUnder12px: first(textEls.filter((el) => parseFloat(getComputedStyle(el).fontSize) < 12).map((el) => name(el) + ' ' + getComputedStyle(el).fontSize)),
    viewportMeta: meta ? meta.content : 'MISSING',
    viewportMetaProblems: [
      !meta && 'no <meta name=viewport>',
      meta && !metaContent.includes('width=device-width') && 'no width=device-width',
      /user-scalable=(no|0)/.test(metaContent) && 'user-scalable=no blocks zoom (WCAG 1.4.4)',
      /maximum-scale=1(\.0)?(,|$)/.test(metaContent) && 'maximum-scale=1 blocks zoom',
    ].filter(Boolean),
    rulesWith100vh: first(vhRules),
    fixedBarsWithoutSafeArea: usesSafeArea ? [] : first(fixedBars.map(name), 5),
  };
}
