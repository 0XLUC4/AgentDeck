---
name: agentdeck-form-flow
description: Test a user flow end to end in the AgentDeck browser panel - sign up, log in, checkout, create, edit, delete, upload, search, multi-step wizards - with valid, invalid and edge-case input, network checks and data checks. Use when a form, a multi-step flow or its API changed, or when asked to test a flow.
---

# Flow test in the AgentDeck browser panel

Use only the `browser` MCP tools (the AgentDeck panel the user watches).

## 1. Plan

Write the steps first: start URL, each action, the expected result after each one (text on screen,
URL, request sent, data stored). Note the test data you will use.

## 2. Happy path

`navigate_page`, `take_snapshot` for uids, `fill_form` (several fields in one call) / `fill` /
`click`, `wait_for` the expected text after each step. Screenshot the end state.

## 3. Failure and edge cases

For each field that matters:

| Case | Example |
|---|---|
| Empty required | submit with nothing |
| Wrong format | `abc` for an email, letters in a phone |
| Limits | 1 char, max length, max + 1, very long text |
| Special text | accents `éàü`, emoji, `<script>alert(1)</script>`, `' OR 1=1 --`, spaces only |
| Numbers | 0, negative, decimals with `,` and `.`, huge |
| Duplicate | same email twice, same name twice |
| Auth | wrong password, expired session, user without the right role |
| Files | wrong type, too big, empty file (`upload_file`) |

Each one must show a clear message next to the field, keep what the user typed, and never show a raw
server error or stack trace. Injected HTML must be displayed as text, not run.

## 4. Behaviour around the form

- Double submit: click twice fast; only one request, button disabled while sending.
- Server error: the page shows a message and the user can retry (check with a failing request if the
  project has a way to force one).
- Back button and reload in the middle of a wizard: no crash, no duplicate record.
- Keyboard: Enter submits, Tab order is logical, errors get focus or are announced.

## 5. Network

For each submit (`list_network_requests`, `get_network_request`): right method and URL, payload
matches what was typed, status code, no secrets in the URL, no request sent for input the page must
reject itself. Server must also reject bad input on its own: client validation is not security.

## 6. Data

Check the result outside the page when there is one (database row, file, email log, queue), then
reload: the data must still be there and appear the same. Edit and delete flows: check it really
changed or disappeared, and that related data is handled.

## 7. Clean up

Delete test data you created when the project allows it.

## Report

A table: step or case, expected, seen, pass/fail. Never use the user's real accounts or pay with real
money; ask when the flow needs them. Payment: use the provider's test mode and test cards only.
