---
name: agentdeck-form-flow
description: Test a user flow end to end in the AgentDeck browser panel - sign up, log in, checkout, create, edit, delete, upload, search, multi-step wizards - with a written plan, valid, invalid and edge-case input, keyboard and accessibility of errors, double submit, offline, network payload and stored data checks, and a pass/fail table. Use when a form, a multi-step flow or the API behind it changed, or when asked to test a flow, before saying it works.
---

# Flow test in the AgentDeck browser panel

Only the `browser` MCP tools (the panel the user watches).

## 1. Plan

Write the steps first: start URL, each action, the expected result after each one (text on screen,
URL, request sent, data stored). Note the test data. Never use the user's real accounts or real
money; ask when the flow needs them. Payments: provider test mode and test cards only.

## 2. Happy path

`navigate_page`, `wait_for` the page text, `take_snapshot` for uids, then `fill_form` (all fields in
one call: faster and more reliable than several `fill`), `click`, and `wait_for` the expected result
text after each step. Take a fresh snapshot after every navigation or DOM change: old uids go stale.
Screenshot the end state.

## 3. Bad and edge input

For each field that matters:

| Case | Example |
|---|---|
| Empty required | submit with nothing |
| Wrong format | `abc` as an email, letters in a phone number |
| Limits | 1 character, max length, max + 1, 10 000 characters |
| Special text | accents `éàü`, emoji, RTL text, leading and trailing spaces, spaces only |
| Injection | `<img src=x onerror=alert(1)>`, `' OR 1=1 --`, `{{7*7}}` |
| Numbers | 0, negative, `1,5` and `1.5`, huge |
| Duplicate | same email twice, same name twice |
| Auth | wrong password, expired session, user without the role |
| Files | wrong type, too big, empty (`upload_file`) |

Each must show a clear message next to the field, keep what was typed, and never show a raw server
error or stack trace. Injected markup is shown as text, never run (no dialog appears; check with
`list_console_messages`).

## 4. Errors are accessible

In the snapshot after a failed submit: the error is text (not only a red border), linked to its
field (`aria-describedby`, `aria-invalid`), and focus moves to the first error. Inputs have labels
and `autocomplete` values (`email`, `current-password`, `new-password`, `one-time-code`):
`list_console_messages` with `types: ["issue"]` reports missing ones.

## 5. Behaviour around the form

- **Double submit**: click twice fast; exactly one request, button disabled while sending.
- **Keyboard only**: `press_key` Tab through the form in a logical order, focus visible, Enter
  submits, Escape closes dialogs.
- **Offline**: `emulate` with `networkConditions: "Offline"`, submit: a clear message, data kept,
  retry works once back online (`emulate` with nothing). Never pass `viewport`.
- **Server error**: the page shows a message and allows retry (force a failure if the project can).
- **Back, forward and reload** in the middle of a wizard: no crash, no duplicate record, no data lost
  without warning (`handle_dialog` for `beforeunload`).

## 6. Network

For each submit (`list_network_requests` with `resourceTypes: ["fetch", "xhr", "document"]`,
`get_network_request`): right method and URL, payload matches what was typed, expected status, no
password or token in the URL or in console logs, no request for input the page should reject
itself. The server must reject bad input on its own: client validation is not security (send a bad
payload with `evaluate_script` + `fetch` when the project allows).

## 7. Data

Check the result outside the page when possible (database row, file, email log, queue), then reload:
still there, shown the same. Edit and delete: really changed or gone, related data handled.

## 8. Clean up

Delete the test data you created when the project allows it.

## Report

A table: step or case | expected | seen | pass/fail, then the bugs found with steps to reproduce,
and what was not tested.
