---
name: agentdeck-debug
description: Root-cause debugging from evidence - reproduce, read the full error, trace the bad value back to its origin, prove one hypothesis at a time, fix once at the source, prove the fix with a regression test. Use when hitting any bug, crash, panic, exception, failing or flaky test, wrong output, hang, regression or "it does not work", before proposing or writing any fix.
---

# Debug from evidence

**Iron law: no fix without a proven root cause.** A change made "to see if it helps" is not debugging.

## Phase 1: Reproduce and read

1. Get the exact error text, input, steps, versions, environment. Turn them into one command or test
   that fails every time. Cannot reproduce: collect logs and ask for the missing piece; do not guess.
2. Read the whole message and stack trace: the first frame in project code, the logs just before.
   Search the exact error text in the code base and in the dependency's issues.
3. What changed? `git log -p --since=... -- <area>`, `git diff`, lock-file diff, config, env vars.

## Phase 2: Trace back to the origin

The error line shows where the bad value was *noticed*, not where it was *made*. Walk the call chain
backwards (caller of caller) until you find the first place where the value is wrong. Add a log or
breakpoint at each component boundary (input and output of each step) instead of reading code and
guessing. Compare with a working case (another input, an older commit, a similar feature) and list
every difference.

Narrowing tools: smallest failing input; `git bisect run <test cmd>` between a good and a bad
commit; clean build (`cargo clean`, delete `node_modules`/`.next`/`__pycache__`); in the browser
panel `set_logpoints`, `get_network_request`, `list_console_messages` with `includeStackTraces`.

## Phase 3: One hypothesis at a time

Write it: "X happens because Y". Test it with the smallest change or observation that would prove it
wrong; change one variable at a time. Make the bug appear and disappear on purpose. Disproved: go
back to Phase 2 with what you learned, never stack a second guess on top.

## Phase 4: Fix and prove

1. A failing test that reproduces the bug (see `agentdeck-tests-first`).
2. One fix, at the origin found in Phase 2, in shared code, not at each caller. Not by hiding the
   error (`try/catch` that ignores, `?.` everywhere, a retry around a race).
3. Search for the same mistake elsewhere (same pattern, same API misuse) and fix those too.
4. Test passes, whole suite passes, the original reproduction passes. Flaky bug: run it 20-50 times
   (`for i in $(seq 50); do <cmd> || break; done`). Remove temporary logs.

**Three fixes failed? Stop.** The model of the problem is wrong, or the design is. Tell the user
what you tried, what each attempt showed, and what you suspect, before trying a fourth.

## Usual causes by symptom

| Symptom | Look at |
|---|---|
| Works here, fails there | env vars, paths, versions, locale, time zone, permissions, file name case |
| Sometimes fails | race, shared state between tests, order, time, network, missing `await` |
| Worked before | `git bisect`, dependency updates, data or config changes |
| null / undefined / None | optional not checked, wrong key, async value read too early |
| Hang | deadlock, lock held across `await`, loop, waiting on a closed channel, no timeout |
| Wrong number | units, integer division, float rounding, off-by-one, overflow, ms vs s |
| Garbled text | UTF-8 vs Latin-1, BOM, `\r\n` |
| Only in production build | minification, tree-shaking, build-time env vars, SSR vs client |

## Red flags: stop when you think

- "I see the problem, let me fix it" (before reproducing).
- "Quick fix for now, investigate later."
- "Let me change a few things and run the tests."
- "One more attempt" (after the third).
- "It is probably a caching / flaky / environment issue" (without evidence).

## Report

The cause in one sentence, the evidence that proves it, the fix and where, the regression test, how
it was verified. If not proven, say so and what is left to check.
