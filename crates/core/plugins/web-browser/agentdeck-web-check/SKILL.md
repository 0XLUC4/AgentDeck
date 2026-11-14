---
name: agentdeck-web-check
description: Check a web page or web app for real in the AgentDeck browser panel (browser MCP tools) - load it, look at it, read console errors and failed requests, use it, fix, check again. Use after any change to a web UI, page, component, style, dev server or API called by a page, or when asked to test, check, show or debug a page.
---

# Web check in the AgentDeck browser panel

The user watches the AgentDeck browser panel next to your terminal. Only the `browser` MCP tools drive it
(`navigate_page`, `take_snapshot`, `take_screenshot`, `click`, `fill`, `list_console_messages`,
`list_network_requests`...). Never open a page any other way: no system browser, no other browser MCP
server, no Playwright or Puppeteer window, no `start <url>`.

## Loop

1. **Serve it.** Start the dev server with your own shell tool, in the background, without `--open`.
   Read its output for the URL and port; wait for "ready"/"compiled" before loading. Port taken: use
   the one the server picked, never kill a process you did not start. Static file: serve it over http
   (`npx serve`, `python -m http.server`), do not open `file://`.
2. **Load it.** `list_pages`, then `navigate_page` on the current page to the URL. Keep one page:
   `new_page` only when you need two at once, `close_page` the extra ones when done.
3. **Look.** `take_snapshot` for structure and element uids (cheap, use it first),
   `take_screenshot` to see the layout. Compare with what was asked: text, spacing, alignment, colors,
   hover/focus/disabled states, loading, empty and error states, long text, many items.
4. **Read the errors.** `list_console_messages` (errors and warnings: hydration mismatch, React key,
   undefined props, failed imports) and `list_network_requests` (4xx/5xx, failed, CORS, 404 assets).
   Open one with `get_network_request` to read its payload and response.
5. **Use it.** Do what a user would: `click`, `fill` / `fill_form`, `press_key` (Enter, Tab, Escape),
   `hover`, `drag`, `upload_file`, `handle_dialog` for alert/confirm, `wait_for` the expected text.
   Check the result on screen, not only that no error appeared.
6. **Fix and repeat.** Change the code, wait for hot reload (or `navigate_page` again), redo steps 3
   to 5 until the page is right with no new console error and no failed request.

## Find causes fast

- Wrong style: `get_css_styles` on the element: which rule wins, computed size, overflow, z-index.
- Wrong data: `get_network_request` on the API call; compare response with what is shown.
- JS state: `evaluate_script` to read DOM values, `localStorage`, element sizes
  (`el.getBoundingClientRect()`), or to call a function.
- Logic in a running page: `set_logpoints` instead of adding `console.log` and rebuilding.
- Animation or transition bug: `screencast_start` / `screencast_stop` to see it frame by frame.
- Stale page after a fix: hard reload (`navigate_page` with cache ignored), check the dev server
  rebuilt without errors.

## Also check quickly

- Keyboard: Tab reaches every control in order, focus is visible, Escape closes dialogs.
- Accessibility basics in the snapshot: buttons and links have names, images have alt, inputs have
  labels.
- Dark mode and a narrow window if the app supports them (use `agentdeck-mobile-check` for phones).

## Report

Say what you loaded, what you did on the page, what you saw (screenshot or snapshot facts), and the
console or network errors left, if any. Never claim a page works without having loaded it.

## Limits

- A page behind a login: ask the user to log in in the panel, then continue. Never type their password.
- The task says not to run or test (an orchestrator worker): skip this skill.
