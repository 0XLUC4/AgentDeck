---
name: agentdeck-debug
description: Find the root cause of a bug with evidence before changing code - reproduce, narrow down, prove the cause, fix once, prove the fix. Use for any bug, crash, failing test, wrong output or "it does not work".
---

# Debug from evidence

1. **Reproduce.** Get the exact error, input and steps. Turn them into a command or test that fails.
   Cannot reproduce: collect logs and ask, do not guess a fix.
2. **Read the error fully**: message, stack trace, the line it points to, the logs just before.
3. **Narrow down.** Bisect: inputs (smallest failing case), code (comment out, `git bisect` against a
   good commit), environment (versions, config, OS). Add temporary logs at the boundaries.
4. **Prove the cause.** Write one sentence: "X happens because Y". Check it with a change that makes
   the bug appear or disappear on purpose. A theory you did not test is not the cause.
5. **Fix the root cause once**, in the shared code, not at each caller. Search for the same mistake
   elsewhere.
6. **Prove the fix.** The failing test or command now passes, the other tests still pass. Remove the
   temporary logs.

Report: the cause in one sentence, the evidence, the fix, how you verified it.
