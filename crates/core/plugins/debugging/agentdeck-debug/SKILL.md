---
name: agentdeck-debug
description: Find the root cause of a bug with evidence before changing code - reproduce, read the error, narrow down, prove the cause, fix once, prove the fix, guard with a test. Use for any bug, crash, panic, exception, failing test, flaky test, wrong output, regression, hang or "it does not work".
---

# Debug from evidence

Never change code to "see if it helps" before you know the cause.

## 1. Reproduce

Get the exact error, input, steps, versions and environment. Turn them into a command or a test that
fails every time. Cannot reproduce: collect logs, ask for the missing piece, do not guess a fix.

## 2. Read the error fully

The whole message and stack trace, the first frame in *your* code, the logs just before. Search the
exact error text in the codebase. Errors often point at the symptom: follow the bad value back to
where it was made.

## 3. Narrow down

- **Input**: shrink to the smallest failing case.
- **Code**: `git bisect` between a good and a bad commit (`git bisect run <test cmd>` automates it);
  comment out halves; replace parts with fixed values.
- **Environment**: versions (`--version`, lock file diff), config, env vars, OS, clean build
  (`cargo clean`, delete `node_modules` / `.next` / `__pycache__` caches).
- **Observe**: logs at the boundaries (input and output of each step), a debugger with breakpoints,
  in the browser `set_logpoints` and `get_network_request`.

## 4. Prove the cause

Write one sentence: "X happens because Y". Check it by making the bug appear or disappear on purpose.
A theory you did not test is not the cause. Two causes are possible: check the first fix fully solves it.

## Common causes by symptom

| Symptom | Look at |
|---|---|
| Works for me, fails elsewhere | env vars, paths, versions, locale, time zone, permissions, case of file names |
| Fails sometimes (flaky) | race, shared state between tests, order, time, network, missing `await` |
| Worked before | `git log -p` on the area, dependency updates, data changes |
| `undefined` / `None` / null | an optional value not checked, wrong key, async value read too early |
| Hang | deadlock, lock held across `await`, infinite loop, waiting on a closed channel, no timeout |
| Wrong number | units, integer division, float rounding, off-by-one, overflow |
| Encoding | UTF-8 vs Latin-1, BOM, `\r\n` |
| Only in production build | minification, tree-shaking, env vars at build time, SSR vs client |

## 5. Fix the root cause once

In the shared code, not at each caller, and not by hiding the error (`try/catch` that ignores,
`?.` everywhere). Search for the same mistake elsewhere and fix it too.

## 6. Prove the fix

The failing test or command now passes; the other tests still pass. Add the reproduction as a
regression test. Remove temporary logs and debug code. Flaky bug: run the test many times
(`for i in $(seq 50)`) before calling it fixed.

## Report

The cause in one sentence, the evidence, the fix, how you verified it, the test added.
