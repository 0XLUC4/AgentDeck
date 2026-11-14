---
name: agentdeck-tests-first
description: Test-driven changes - write one failing test for the behaviour or the bug, watch it fail for the right reason, make it pass with the smallest change, refactor while green, keep the suite fast and deterministic. Use when fixing a bug, adding or changing logic, or when asked for tests or TDD, before writing the implementation code.
---

# Tests first

**Iron law: no production code without a test that failed first.** A test you never saw fail proves
nothing.

## Loop

1. **Find the setup.** Framework, where tests live, helpers, how to run one test fast:
   `cargo test name`, `pnpm vitest run file -t name`, `pytest path::test -x`, `go test -run Name ./pkg`,
   `dotnet test --filter Name`. Copy the style of the nearest existing test.
2. **Red.** One test, one behaviour, named after it (`rejects_expired_token`). Run it. It must fail
   *on the assertion*, not on a typo, an import or a missing function. Passes at once? It tests
   existing behaviour: fix the test.
3. **Green.** The smallest change that passes. Nothing extra.
4. **Refactor** code and test while green. Run the related suite.
5. **Next case**: boundaries, empty, invalid input, errors from dependencies.

Bug fix: the test reproduces the reported bug exactly, fails before the fix, passes after.

Wrote implementation before the test? Write the test now, then revert the implementation and check
the test fails, then restore it. Otherwise the test is not proven.

## What makes a good test

| Rule | Why |
|---|---|
| Through the public interface, not private details | survives refactors |
| Real code inside, fakes only at boundaries (HTTP, DB, clock, file system) | tests real logic, not mocks |
| No sleeps: wait on conditions, fake timers | fast, not flaky |
| No real network, clock or randomness: inject or seed them | deterministic |
| Each test builds its own data, no order dependence | runs alone and in parallel |
| Exact assertions (values, error types, messages) | catches regressions |
| Snapshots only for stable output, reviewed on change | no rubber-stamping |

Integration tests for wiring (DB queries, HTTP handlers, CLI) where a unit test would only test a
mock. End-to-end only for the few critical user flows (in the browser panel: `agentdeck-form-flow`).

## Anti-patterns

- Asserting that a mock was called instead of what the code produced.
- Test-only methods or flags added to production code.
- Mocking something you do not understand (its errors, its ordering).
- Deleting, skipping or loosening a test to get green. Fix the code, or explain why the test was
  wrong and get agreement.
- Leaving `.only`, `.skip`, `#[ignore]`, `xit`, debug prints.

## Red flags: stop when you think

| Thought | Reality |
|---|---|
| "Too simple to test" | Simple code breaks; the test takes a minute. |
| "I'll test after" | Then the test only confirms what the code does, not what it should do. |
| "I tested it by hand" | Not repeatable, gone at the next change. |
| "No tests exist here, so skip" | Add the first one. |
| "The test is flaky, rerun it" | Flaky is a bug: find the shared state or the race. |

## Report

Tests added (names), the red output (failing for the right reason), the green output, the suite
result.
