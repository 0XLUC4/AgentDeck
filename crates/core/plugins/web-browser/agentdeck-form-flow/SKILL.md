---
name: agentdeck-form-flow
description: Test a user flow end to end in the AgentDeck browser panel - sign up, log in, checkout, create, edit, delete - with valid and invalid input. Use when a form, a multi-step flow or its API changed, or when asked to test a flow.
---

# Flow test in the AgentDeck browser panel

Use only the `browser` MCP tools (the AgentDeck panel the user watches).

1. Write the steps first: start URL, each action, the expected result after each one.
2. Run the happy path: `navigate_page`, `take_snapshot` for uids, `fill_form` / `fill` / `click`,
   `wait_for` the expected text after each step. Screenshot the end state.
3. Run the failure paths: empty required field, wrong format, too long, duplicate, wrong password.
   Each one must show a clear message next to the field and keep what the user typed.
4. Watch the requests of each submit (`list_network_requests`, `get_network_request`): right method,
   payload, status; no request sent for input the page itself must reject.
5. Check the result outside the page when there is one (database row, file, email log), then reload:
   the data must still be there.
6. Clean up test data you created when the project allows it.

Report each step as passed or failed with what you saw. Never use the user's real accounts or pay with
real money; ask when the flow needs them.
