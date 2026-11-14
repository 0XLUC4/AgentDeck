---
name: agentdeck-web-perf
description: Measure and improve a web page's speed in the AgentDeck browser panel - Lighthouse audit, performance trace, LCP, CLS, INP, bundle and request weight. Use when a page is slow, before a release, or when asked about web performance or Core Web Vitals.
---

# Web performance in the AgentDeck browser panel

Measure first, change one thing, measure again. Use only the `browser` MCP tools.

## Measure

1. Use a production build (`build` then `preview`/`start`), not the dev server: dev numbers lie.
2. `lighthouse_audit` on the page (mobile and desktop). Note the performance score, LCP, CLS, TBT.
3. `performance_start_trace` with reload, then `performance_stop_trace`; read the insights with
   `performance_analyze_insight` (LCP breakdown, render-blocking, long tasks, layout shifts).
4. `list_network_requests`: total weight, biggest files, requests that block the first paint,
   missing compression or caching headers.

## Usual fixes, biggest gain first

- LCP: preload or prioritize the hero image (`fetchpriority="high"`), right size and modern format
  (AVIF/WebP), no lazy loading above the fold, server response under 200 ms.
- Render-blocking: defer non-critical JS, inline critical CSS, `font-display: swap`, preconnect to
  needed origins.
- JS weight: split by route, remove unused dependencies, import only what is used, check the bundle
  (`vite build --report`, `next build` output, source-map-explorer).
- CLS: width/height or `aspect-ratio` on images and embeds, space reserved for late content.
- INP / long tasks: break up work over 50 ms, debounce input handlers, move heavy work to a worker.

## Report

A before/after table (score, LCP, CLS, TBT, JS KB, requests), what you changed and why. Keep a change
only if the numbers improved.
