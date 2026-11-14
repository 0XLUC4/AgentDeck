---
name: agentdeck-self-review
description: Review your own changes before saying a task is done - read the full diff, hunt bugs, missed callers, error handling, security and leftovers, run the checks. Use at the end of every coding task, before committing, or when asked to review a diff.
---

# Self-review before "done"

1. Read the whole diff (`git diff`, plus untracked files). Every line must be wanted: no debug prints,
   commented-out code, stray files, unrelated formatting.
2. For each changed function, find its callers (search the code) and check they still work.
3. Hunt bugs on purpose: empty input, None/null, zero, huge input, errors from I/O and network,
   concurrent calls, off-by-one, wrong units, time zones, Windows paths.
4. Errors: none swallowed silently; messages say what failed and what to do.
5. Security at boundaries: user input validated, no injection (SQL, shell, HTML), no secret in code
   or logs, file paths kept inside their folder.
6. Tests: a test covers the new behaviour or the bug fixed; run the project's checks (build, lint,
   tests) and read the output.
7. Fix what you found, then repeat from 1 on the new diff.

Report what you checked, what you fixed, and what you could not verify.
