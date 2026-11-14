---
name: agentdeck-web-perf
description: Measure and improve a web page's speed in the AgentDeck browser panel - Lighthouse audit, performance trace, LCP, CLS, INP, TBT, bundle size, request weight, caching, memory leaks. Use when a page is slow, heavy, janky, before a release, or when asked about web performance or Core Web Vitals.
---

# Web performance in the AgentDeck browser panel

Measure first, change one thing, measure again. Use only the `browser` MCP tools.

## Targets ("good" by Google)

| Metric | Good | What it means |
|---|---|---|
| LCP | under 2.5 s | biggest element shown |
| INP | under 200 ms | reaction to a click or key |
| CLS | under 0.1 | layout jumps |
| TBT | under 200 ms | main thread blocked (lab proxy for INP) |
| JS | under ~170 KB gzip on first load | parse and run cost on phones |

## Measure

1. Use a production build (`build` then `preview`/`start`), not the dev server: dev numbers lie.
2. `lighthouse_audit` on the page, mobile and desktop. Note score, LCP, CLS, TBT and the top
   opportunities. Run twice; numbers vary.
3. `performance_start_trace` with reload, then `performance_stop_trace`; read the insights with
   `performance_analyze_insight` (LCP breakdown, render-blocking, long tasks, layout shifts, third
   parties). Trace an interaction too (start, click, stop) for INP.
4. `list_network_requests`: total weight, biggest files, requests that block the first paint, waterfall
   chains (A loads B loads C), missing compression (`content-encoding`), missing cache headers.
5. Test on a slow profile: `emulate` with CPU throttling (4x) and slow 4G, like a mid-range phone.

## Usual fixes, biggest gain first

- **LCP**: hero image preloaded or `fetchpriority="high"`, right size (`srcset`/`sizes`), AVIF/WebP,
  never lazy above the fold; server response under 200 ms (cache, CDN, static or streamed render);
  no client-side fetch before the main content shows.
- **Render-blocking**: `defer`/`async` scripts, critical CSS inlined, fonts `font-display: swap`,
  preloaded and subset (woff2), `preconnect` to needed origins only.
- **JS weight**: split by route (`import()`), remove unused dependencies, replace heavy ones
  (moment → date-fns/Temporal, lodash → native), tree-shake (named ESM imports), check the bundle
  (`vite build` + `rollup-plugin-visualizer`, `next build` output, `source-map-explorer`).
- **Third parties**: tag managers, chat widgets, analytics: load after interaction or idle.
- **CLS**: width/height or `aspect-ratio` on images and embeds, space reserved for ads and late
  content, no content inserted above existing content, fonts with matching fallback metrics.
- **INP / long tasks**: break work over 50 ms (`scheduler.yield()` / `setTimeout`), debounce input,
  virtualize long lists, move heavy work to a Web Worker, avoid layout thrashing (read then write).
- **React**: avoid re-rendering big trees (state lower, `memo` only where measured), stable keys,
  server components / SSR for static parts.
- **Caching**: hashed static files `Cache-Control: public, max-age=31536000, immutable`; HTML short or
  `no-cache`; Brotli or gzip on text.
- **Memory leaks** (page slows over time): `take_heapsnapshot` before and after repeating an action
  a few times; growing detached nodes or listeners mean a missing cleanup.

## Report

A before/after table (score, LCP, CLS, TBT, INP, JS KB, total KB, requests), what you changed and why.
Keep a change only if the numbers improved. Say what lab numbers cannot show (real users, real
devices).
