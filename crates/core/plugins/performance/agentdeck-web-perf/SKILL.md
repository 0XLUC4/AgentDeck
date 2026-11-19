---
name: agentdeck-web-perf
description: Use to measure slow or janky pages, network weight and Core Web Vitals in the AgentDeck browser. Trace Largest Contentful Paint (LCP), INP and CLS before optimizing or releasing.
---

# Measure web performance in the AgentDeck browser

Use the panel's `browser` MCP tools against a production build (`preview` or `start`). Lighthouse audits accessibility, SEO and best practices. Collect speed metrics from traces.

## Baseline

At the 75th percentile, good/poor thresholds are LCP ≤2.5 s/>4 s, INP ≤200 ms/>500 ms, CLS ≤0.1/>0.25. First-load compressed budgets: JS <300 KB, CSS <100 KB, fonts <100 KB, third parties <200 KB, total <1.5 MB. TBT is a lab diagnostic.

1. Load: `navigate_page`, then `performance_start_trace` (reload and auto-stop; `ignoreCache: true` for cold runs). Repeat under identical conditions. Record LCP, CLS and available CrUX field data. Use `performance_analyze_insight` with insight set ID and name: LCPBreakdown, LCPDiscovery, DocumentLatency, RenderBlocking, NetworkDependencyTree, CLSCulprits or ThirdParties.
2. Interaction: trace with `reload: false, autoStop: false`, reproduce the slow click, input or keypress, then `performance_stop_trace`. Inspect input, processing and presentation delay and tasks >50 ms.
3. Network: filter `list_network_requests` by resourceTypes. Check size, chains, blocking resources and headers via `get_network_request`.
4. Phone: one `emulate` call sets both `cpuThrottlingRate: 4` and `networkConditions: "Slow 4G"`. Each call replaces settings. Use the panel's mobile preview for viewport; reset throttling with an empty settings call.

## Follow the trace

- LCP: identify the largest subpart. For TTFB inspect redirects, server caching, CDN and streaming. For discovery delay expose the image as HTML `<img src>`, use `fetchpriority="high"`, avoid lazy-loading; preload when HTML discovery is impossible. For transfer duration use responsive sizes, AVIF/WebP and CDN. For render delay inspect blocking CSS/JS, fonts and client fetches.
- INP: split long tasks, yield or move heavy work to a worker. Show feedback early, batch DOM reads before writes, virtualize long lists. For React, localize state and measure memoization or deferred updates.
- CLS: reserve dimensions for images, embeds, ads and late content; match fallback font metrics.
- Bytes: inspect bundle composition, split by route, trim heavy imports and defer third parties. Compress text, cache hashed assets as immutable and revalidate HTML.
- Navigation: test back/forward restoration and inspect reported blockers such as unload handlers. Evaluate speculation rules for subsequent navigations.
- Memory: use `take_heapsnapshot` with a filePath. Repeat the suspected action 5–10 times and compare snapshots for retained listeners or detached nodes.

Change one measured cause, repeat the same test and compare LCP/subpart, CLS, interaction duration, long tasks, JS/total KB and request count. Report field data separately from lab results and keep improvements that exceed run-to-run variation.
