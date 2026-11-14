---
name: agentdeck-mobile-check
description: Check a web page at phone width in the AgentDeck browser panel's mobile preview - measured overflow, tap targets (WCAG 2.2), input zoom, text size, viewport meta, 100vh and safe areas, menus, forms, slow network - with a ready audit script. Use for responsive or mobile work, a mobile app's web preview (Expo web, Capacitor, PWA), or when asked whether a page works on phones.
---

# Mobile check in the AgentDeck browser panel

Only the `browser` MCP tools (the panel the user watches).

## Phone width: the panel's mobile preview

The panel has a mobile preview: 390 x 844 viewport, mobile user agent, touch. **Never call
`resize_page`, or `emulate` with a `viewport`**: they resize the native page, which then leaves the
panel. In a Claude agent such a call is blocked and switches the mobile preview on (width under 600)
or off (wider) instead; other agents ask the user to click the phone button in the panel bar.
Other widths (360, 430, tablet 768, landscape) are not available: say they were not checked.

## Steps

1. `navigate_page` to the URL, mobile preview on (Claude: `resize_page` 390 x 844 switches it).
2. **Audit**: `evaluate_script` with `sourcePath` = `scripts/mobile-audit.js` in this skill's folder
   (absolute path; e.g. `~/.claude/skills/agentdeck-mobile-check/scripts/mobile-audit.js` expanded,
   or `~/.agents/skills/...`), `waitForStableDom: false`. It returns:
   - `userAgentMobile`, `viewportWidth`: confirm the preview is on (about 390, true).
   - `horizontalOverflow` and `overflowCulprits` (deepest elements sticking out).
   - `tapTargetsUnder24px` (fail, WCAG 2.2 AA) and `tapTargetsUnder44px` (warn, Apple and AAA).
   - `inputsUnder16px` (iOS zooms the page on focus), `textUnder12px`.
   - `viewportMetaProblems`, `rulesWith100vh`, `fixedBarsWithoutSafeArea`.
3. **Look**: `take_screenshot` (`fullPage: true` for the whole page) and compare with the audit.
4. **Use it**: fresh `take_snapshot` before each action (uids go stale after any DOM change); open
   the menu, dialogs, dropdowns; `fill` a field; check the submit stays reachable and fixed headers
   or footers do not cover content. Hover-only features need a tap equivalent.
5. **Accessibility**: `lighthouse_audit` with `device: "mobile"`, `mode: "snapshot"`.
6. **Slow network**: `emulate` with `networkConditions: "Slow 4G"` and `cpuThrottlingRate: 4` in one
   call, no `viewport`; reload; loading states shown, page usable before everything arrives. Each
   `emulate` call replaces all its settings and may drop the mobile user agent: rerun the audit and
   check `userAgentMobile`; `emulate` with nothing to reset at the end.
7. Fix, reload, rerun the audit until clean. Turn the preview off when done (Claude: `resize_page`
   1280 x 800).

## Usual fixes

| Finding | Fix |
|---|---|
| Overflow | the culprit's fixed `width` → `max-width: 100%`; `min-width: 0` on flex/grid children; tables and code in their own `overflow-x: auto` box; long words and URLs `overflow-wrap: anywhere` |
| Small tap target | padding to 44 px or `min-height`/`min-width: 44px`; at least 24 px with spacing |
| Input zoom | `font-size: 16px` (or more) on inputs; never `user-scalable=no` or `maximum-scale=1` |
| 100vh | `100dvh` (or `100svh` for a stable minimum) |
| Notch, home bar | `viewport-fit=cover` plus `padding: env(safe-area-inset-*)` on fixed bars |
| Missing viewport meta | `<meta name="viewport" content="width=device-width, initial-scale=1">` |
| Component layout | container queries (`@container`) where the component, not the page, decides |

## Report

What was checked at 390 px, audit results before and after, screenshots, what was fixed, and what
was not tested: other widths, landscape, a real device, iOS Safari, an on-screen keyboard.
