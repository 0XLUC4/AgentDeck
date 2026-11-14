---
name: agentdeck-self-review
description: Review your own changes before saying a task is done - read the full diff, hunt bugs, missed callers, error handling, security, performance, tests and leftovers, run the project's checks. Use at the end of every coding task, before committing, or when asked to review a diff or a pull request.
---

# Self-review before "done"

Review as a stranger who has to maintain this code tomorrow. Assume there is a bug and look for it.

## 1. Read the whole diff

`git status` and `git diff` (plus untracked files, plus `git diff --staged`). Every line must be wanted:
no debug prints, commented-out code, TODO you could do now, stray files, unrelated formatting, changed
lock files you did not mean to change, secrets or `.env` files.

## 2. Does it do the task?

Re-read the request. Every part asked is done; nothing extra slipped in. The behaviour changed only
where it had to.

## 3. Callers and contracts

For each changed function, type, route, config key, DB column or event: search every use (code,
tests, scripts, docs) and check each one still works. Renamed or removed public API: all callers
updated or the old one kept. Changed data format: old saved data still loads (migration or default).

## 4. Hunt bugs on purpose

| Area | Ask |
|---|---|
| Input | empty, null/None/undefined, zero, negative, huge, unicode, whitespace |
| Bounds | off-by-one, first/last item, empty list, one item |
| Errors | I/O, network, timeout, parse failure, partial failure: handled? |
| State | called twice, called concurrently, called out of order, after cleanup |
| Async | missing `await`, unhandled promise, race between two requests, stale closure |
| Time | time zones, DST, timestamps in seconds vs ms |
| Platform | Windows paths and `\r\n`, case-insensitive file systems, spaces in paths |
| Numbers | integer overflow, float rounding on money (use integers or decimals) |

## 5. Errors

None swallowed silently (`catch {}`, `.ok()`, `_ =`). Messages say what failed and with what value.
Resources closed on every path (files, connections, listeners, timers).

## 6. Security at boundaries

User input validated server-side; no injection (SQL with parameters, no shell string building, HTML
escaped); auth and permission checked on every new endpoint; no secret in code, logs or URLs; file
paths kept inside their folder (no `../`); new dependencies are known, maintained and needed.

## 7. Performance

No query or network call inside a loop, no full load of a big table, no O(n²) on lists that grow,
no blocking call on a UI or async thread.

## 8. Tests and checks

A test covers the new behaviour or the bug fixed (fails without the change). Run the project's own
checks: build, type check, lint, format, tests (find them in `package.json`, `Cargo.toml`, `Makefile`,
CI config). Read the output; warnings you introduced count.

## 9. Fix and repeat

Fix what you found, then review the new diff again from step 1.

## Report

What you checked, what you fixed, the checks run and their result, and what you could not verify.
