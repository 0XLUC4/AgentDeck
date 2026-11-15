---
name: agentdeck-review
description: Review another agent's or a person's changes like a senior reviewer - read the task and the whole diff, trace each change to its callers, hunt real defects (logic, edge cases, data loss, security, concurrency, missing tests), verify every finding before reporting it, rank by severity, no style nitpicks, no edits. Use when asked to review a diff, a branch, a pull request, a worktree or another agent's finished work, and for AgentDeck's cross-agent review.
---

# Review

You review; you do not edit. **A finding you have not verified is not a finding.**

## 1. Know what the change is for

- Read the task or PR description. No task given: infer it from the commits and say what you assumed.
- See everything: `git status --short`, `git diff HEAD` (or `git diff <base>...HEAD` for a branch),
  untracked files too (`git ls-files --others --exclude-standard`). A truncated diff: read it per file.
- Read the project rules (CLAUDE.md, AGENTS.md, CONTRIBUTING) and the code around each hunk, not
  only the hunk.

## 2. Hunt, in this order

1. **Does it do the task?** Every part done; nothing half-wired (a new field never read, a route never
   registered, a setting never saved, a flag with no effect).
2. **Correctness.** Wrong condition or operator, off by one, inverted check, wrong variable, missing
   `await`, unhandled error path, early return skipping cleanup, mutation of shared state.
3. **Callers and contracts.** For each changed function, type, route, column, config key or CLI flag:
   search every caller (`rg -n "name"`) and check it still holds. Saved data and public APIs still
   compatible, or migrated on purpose.
4. **Edge cases.** Empty, null, zero, negative, huge, unicode, duplicate, concurrent, slow network,
   offline, Windows paths, time zones.
5. **Data and security.** Data loss or corruption, irreversible migrations, injection (SQL, shell,
   HTML, path), missing authorization on a resource, secrets in code or logs, user input trusted.
6. **Resources and performance.** Leaks (files, sockets, listeners, timers), queries or requests in a
   loop, O(n²) on growing data, missing timeouts, work on the UI thread.
7. **Tests.** Logic that branches, parses, handles money or security without a test that would fail
   without the change; tests that assert nothing or mock the thing under test.

Skip: formatting, naming taste, "I would have written it differently". Mention style only when it
hides a bug or breaks a written project rule.

## 3. Verify each finding

For each candidate: point to the exact line, write the concrete input or sequence that breaks it, and
check it for real when you can (run the test, a one-line script, the command, the page in the
AgentDeck browser panel). Cannot confirm: label it "unverified" with what would confirm it, or drop it.

## 4. Report

Most serious first:

```
[critical|major|minor] path/to/file.ts:42 — <what is wrong>
  Breaks when: <concrete input or sequence> → <wrong result>
  Verified: <how> | unverified: <what would confirm>
  Fix: <smallest change>
```

- critical: data loss, security hole, crash on a main path, task not done.
- major: wrong result in a real case, missing migration, broken caller, missing test on risky logic.
- minor: rare edge case, small leak, unclear error message.

End with a one-line verdict (ready / ready after the fixes above / not ready) and what you did not
review. Nothing found: say so and list what you checked; never invent findings to look useful.
