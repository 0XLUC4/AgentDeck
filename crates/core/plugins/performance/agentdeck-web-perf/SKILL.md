---
name: agentdeck-web-perf
description: Measure and improve web page speed in the AgentDeck browser panel - performance traces with LCP subparts, INP from real interactions, CLS culprits, field data when available, network weight, render-blocking, caching, bfcache, memory leaks, before/after numbers. Use when a page is slow, heavy or janky, before a release, or when asked about web performance or Core Web Vitals, before changing code for speed.
---

# Web performance in the AgentDeck browser panel

Measure, change one thing, measure again. Only the `browser` MCP tools (the panel the user sees).
`lighthouse_audit` does **not** measure performance (it covers accessibility, SEO and best
practices): speed numbers come from traces.

## Targets (Core Web Vitals, 75th percentile)

| Metric | Good | Poor |
|---|---|---|
| LCP (largest element shown) | ≤ 2.5 s | > 4 s |
| INP (reaction to clicks and keys) | ≤ 200 ms | > 500 ms |
| CLS (layout jumps) | ≤ 0.1 | > 0.25 |

Budgets on first load (compressed): JS < 300 KB, CSS < 100 KB, fonts < 100 KB, third parties
< 200 KB, total < 1.5 MB. FID is gone; TBT is only a lab hint, not a target.

## Measure

1. **Production build** (`build` then `preview`/`start`), never the dev server: dev numbers lie.
2. **Load trace**: `navigate_page` to the URL, then `performance_start_trace` (reload and auto stop by
   default; `ignoreCache: true` for a cold load). It returns LCP, CLS, field data (CrUX) when the
   site has some, and the insight list. Open the relevant ones with `performance_analyze_insight`
   (insight set id + name): `LCPBreakdown`, `LCPDiscovery`, `DocumentLatency`, `RenderBlocking`,
   `NetworkDependencyTree`, `CLSCulprits`, `ThirdParties`. Run twice; numbers vary.
3. **Interaction trace (INP)**: `performance_start_trace` with `reload: false, autoStop: false`, do
   the slow interaction (`click`, `fill`, `press_key`), then `performance_stop_trace`. Look at long
   tasks (> 50 ms) and their input delay, processing time and presentation delay.
4. **Network**: `list_network_requests` (`resourceTypes` to filter): total weight, biggest files,
   render-blocking CSS/JS, chains (A loads B loads C), missing compression or cache headers
   (`get_network_request` for headers).
5. **Mid-range phone**: one `emulate` call with `cpuThrottlingRate: 4` and
   `networkConditions: "Slow 4G"` (each call replaces all settings), trace again, then `emulate` with
   neither to reset. Never pass `viewport` (it moves the page out of the panel; use the panel's
   mobile preview).

## LCP: fix the biggest subpart

Healthy split: TTFB ~40 %, resource load delay < 10 %, resource load duration ~40 %, render delay
< 10 %.

- **TTFB**: server cache, CDN, static or streamed rendering, fewer redirects.
- **Load delay**: the LCP image must be discoverable in the HTML as a real `<img src>` (not
  `data-src`, not a CSS background set by JS), `fetchpriority="high"`, never `loading="lazy"`;
  `<link rel="preload">` only when it cannot be in the HTML.
- **Load duration**: right size (`srcset`/`sizes`), AVIF/WebP, CDN.
- **Render delay**: render-blocking CSS/JS (`defer`, critical CSS inline), fonts
  (`font-display: swap`, preloaded woff2 subset), no client-side fetch before the main content.

## INP, CLS and the rest

- **INP**: break work over 50 ms (`await scheduler.yield()`, fallback `setTimeout`), show feedback
  before heavy work, debounce input, virtualize long lists, smaller DOM, no layout thrashing
  (read then write), heavy work in a Web Worker. React: state lower, `memo` where measured,
  `useTransition` for non-urgent updates.
- **CLS**: `width`/`height` or `aspect-ratio` on images and embeds, space reserved for late content
  and ads, nothing inserted above existing content, fallback fonts with matching metrics
  (`size-adjust`).
- **JS weight**: split by route (`import()`), drop or replace heavy dependencies, named ESM imports,
  inspect the bundle (`vite build` + `rollup-plugin-visualizer`, `next build` output,
  `source-map-explorer`). Third parties after interaction or idle.
- **Caching**: hashed assets `Cache-Control: public, max-age=31536000, immutable`; HTML short or
  `no-cache`; Brotli or gzip on text.
- **Back/forward cache**: navigate away and back (`navigate_page` back); blockers are
  `Cache-Control: no-store` on HTML and `unload` handlers.
- **Next pages**: multi-page sites gain near-instant navigations from
  `<script type="speculationrules">` (prefetch or prerender).
- **Memory leak** (slower over time): `take_heapsnapshot` (a `filePath` is required), repeat the
  action 5-10 times, snapshot again; growing detached nodes or listeners mean a missing cleanup.

## Report

A before/after table: LCP (and its biggest subpart), CLS, INP or the slowest interaction, long tasks,
JS KB, total KB, requests, field data if any. What changed and why. Keep a change only if the
numbers improved. Lab numbers are not real users on real devices: say so.
