---
name: agentdeck-web-check
description: Check a web page or web app for real in the AgentDeck browser panel (browser MCP tools) - load it, look at it, read console errors and failed requests, fix, check again. Use after any change to a web UI, page, dev server, API called by a page, or when asked to test, check, show or debug a page.
---

# Web check in the AgentDeck browser panel

The user watches the AgentDeck browser panel next to your terminal. Only the `browser` MCP tools drive it
(`navigate_page`, `take_snapshot`, `take_screenshot`, `click`, `fill`, `list_console_messages`,
`list_network_requests`...). Never open a page any other way: no system browser, no other browser MCP
server, no Playwright or Puppeteer window, no `start <url>`.

## Loop

1. **Serve it.** Start the dev server with your own shell tool, in the background, without `--open`.
   Read its output for the URL and port. Static file: serve it over http, do not open `file://`.
2. **Load it.** `list_pages`, then `navigate_page` on the current page to the URL. Keep one page:
   `new_page` only when you need two at once, `close_page` the extra ones when done.
3. **Look.** `take_snapshot` for structure and element uids, `take_screenshot` to see the layout.
   Compare with what was asked: text, layout, states, empty and error states.
4. **Read the errors.** `list_console_messages` (errors and warnings) and `list_network_requests`
   (status 4xx/5xx, failed, CORS). Open one with `get_network_request` when it failed.
5. **Use it.** Do what a user would: `click`, `fill` / `fill_form`, `press_key`, `wait_for` the result.
   Check the result on screen, not only that no error appeared.
6. **Fix and repeat.** Change the code, reload (`navigate_page` again), redo steps 3 to 5 until the page
   is right with no new console error and no failed request.

## Report

Say what you loaded, what you did on the page, what you saw (screenshot or snapshot facts), and the
console or network errors left, if any. Never claim a page works without having loaded it.

## Limits

- A page behind a login: ask the user to log in in the panel, then continue.
- The task says not to run or test (an orchestrator worker): skip this skill.
