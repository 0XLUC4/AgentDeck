---
name: agentdeck-tests-first
description: Drive changes with tests - write a failing test that shows the bug or the new behaviour, make it pass with the smallest change, refactor, keep the suite fast and reliable. Use when fixing a bug, adding logic, changing behaviour, or when asked to add tests or TDD.
---

# Tests first

A test that never failed proves nothing. Red, green, refactor.

## Loop

1. **Find the test setup**: framework, where tests live, how to run one test fast
   (`cargo test name`, `pnpm vitest run file -t name`, `pytest path::test -x`, `go test -run Name`).
   Follow the existing style and helpers.
2. **Red**: write one test for one behaviour. Run it; it must fail *for the right reason* (the
   assertion, not a typo or import error). Bug fix: the test reproduces the bug exactly.
3. **Green**: smallest change that makes it pass. No extra features.
4. **Refactor**: clean the code and the test with the test green. Run the whole related suite.
5. Repeat for the next case: edge cases, errors, boundaries.

## What to test

- Behaviour through the public interface, not private details: tests survive refactors.
- Happy path, then: empty, null, boundary values, invalid input, errors from dependencies.
- One reason to fail per test; name says the behaviour (`rejects_expired_token`).
- Bugs: every fixed bug gets a regression test.

## Good tests

| Rule | Why |
|---|---|
| Arrange, act, assert, clearly separated | readable |
| No sleeps; wait on conditions or use fake timers | fast, not flaky |
| No real network, clock or randomness; inject or fake them | deterministic |
| Each test sets up its own data; no order dependence | can run alone and in parallel |
| Fakes at boundaries (HTTP, DB, file system), real code inside | tests real logic |
| Exact assertions (values, error types), not just "no throw" | catches regressions |
| Snapshot tests only for stable output, reviewed on change | avoid rubber-stamping |

Integration tests for wiring (DB, HTTP handlers, CLI) where unit tests would only test mocks.
End-to-end for the few critical user flows (in the browser panel: see `agentdeck-form-flow`).

## Never

- Delete, skip or weaken a test to get green. Fix the code, or explain why the test was wrong.
- Assert on what the code does instead of what it should do.
- Leave `.only`, `skip`, `#[ignore]` or debug prints.

Report: tests added (names), red then green output, suite result.
