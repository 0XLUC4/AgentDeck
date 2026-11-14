---
name: agentdeck-web-check
description: Check a web page or web app for real in the AgentDeck browser panel with the browser MCP tools - serve it, load it, look, read console errors and DevTools issues, check failed requests, use the changed feature, run a quick accessibility audit, fix and check again. Use after any change to a web UI, page, component, style, dev server or an API a page calls, and when asked to test, check, show or debug a page, before saying the change works.
---

# Web check in the AgentDeck browser panel

**Never say a page works without having loaded and used it in the panel.**

The user watches the AgentDeck browser panel. Only the `browser` MCP tools drive it (in Claude Code:
`mcp__browser__*`). Never open a page any other way: no system browser, no other browser MCP server,
no Playwright or Puppeteer window, no `start <url>`.

## Loop

1. **Serve.** Start the dev server with your own shell tool, in the background, without `--open`.
   Read its output for the URL; wait for "ready"/"compiled". Port taken: use the one the server
   picked; never kill a process you did not start. Static files: serve over http
   (`python -m http.server`, `npx serve`), never `file://`.
2. **Load.** `list_pages` (gives the `pageId` every tool needs), then `navigate_page` on the current
   page. One page: `new_page` only when you need two at once, `close_page` the extras after.
3. **Wait and look.** `wait_for` text you expect (never a fixed sleep), then `take_snapshot`
   (structure and uids, cheap) and `take_screenshot` (layout). Compare with what was asked: text,
   spacing, alignment, colors, states (hover, focus, disabled, loading, empty, error), long text,
   many items.
4. **Read errors.** `list_console_messages` with `types: ["error", "warn", "issue"]` and
   `includePreservedMessages: true` (keeps errors from before a redirect); `includeStackTraces: true`
   to find the line. `issue` lists DevTools issues: missing labels, autocomplete, CSP, cookies,
   mixed content. `list_network_requests`: 4xx/5xx, failed, CORS, 404 assets
   (`resourceTypes: ["fetch", "xhr"]` for API calls); `get_network_request` for payload and response.
5. **Use the changed feature.** Fresh `take_snapshot` before each action (uids go stale after any DOM
   change). `click`, `fill` / `fill_form`, `press_key` (Enter, Tab, Escape), `hover`, `drag`,
   `upload_file`, `handle_dialog` for alert/confirm. Then check the result on screen and re-read
   console and network: many errors only appear on interaction.
6. **Accessibility gate.** `lighthouse_audit` with `mode: "snapshot"` (current state, no reload,
   ~2 s): buttons without a name, images without alt, inputs without a label, contrast, with
   selectors. The snapshot alone misses them: an image without alt is not in it at all.
7. **Fix and repeat.** Change the code, wait for hot reload or `navigate_page` with
   `type: "reload", ignoreCache: true`, and redo 3 to 6 until the page is right with no new error.
   Console and network lists keep everything since the last load: reload before reading them again.

## Find causes fast

| Problem | Tool |
|---|---|
| Wrong style | `get_css_styles` on the uid: winning rule, overridden ones, source line |
| Wrong data | `get_network_request` on the API call; compare the response with the screen |
| State, sizes | `evaluate_script` (`el.getBoundingClientRect()`, `localStorage`, a function call) |
| Logic in the running page | `set_logpoints` (`src/file.ts:42 'value', x`) instead of adding `console.log` and rebuilding |
| Error before the page settles | `navigate_page` with `initScript` to hook `window.onerror` early |
| Animation bug | `screencast_start` / `screencast_stop` |
| Stale page | hard reload as in step 7; check the dev server rebuilt without errors |

## Also check quickly

- Keyboard: Tab reaches every control in order, focus visible, Escape closes dialogs.
- Dark mode if supported: `emulate` with `colorScheme: "dark"`, then `"auto"`. Never pass
  `viewport` to `emulate`; phones: `agentdeck-mobile-check`.

## Report

What you loaded, what you did on the page, what you saw (screenshot or snapshot facts), console and
network errors left, accessibility findings. Measured facts apart from guesses about their cause.

## Limits

- Behind a login: ask the user to log in in the panel, then continue. Never type their password.
- The task says not to run or test (an orchestrator worker): skip this skill.
