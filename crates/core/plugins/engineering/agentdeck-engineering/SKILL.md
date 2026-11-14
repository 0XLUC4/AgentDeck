---
name: agentdeck-engineering
description: Senior-engineer discipline for writing, changing or reviewing code - understand before editing, the smallest complete change that fits the project, surgical diffs, no speculative code, then a self-review and fresh proof before saying done. Use on any feature, fix, refactor or code review, before the first edit and again before claiming the work is done, committed or ready; also when the user asks for simple, minimal, KISS, clean, production-grade or reviewed code, or complains about bloat.
---

# Engineering discipline

**Iron law: no "done", "fixed", "works" or "tests pass" without evidence produced in this turn.**

Run the four phases in order. A one-line obvious change still gets phases 3 and 4.

## 1. Understand before editing

- Restate the goal in one sentence and how you will know it works (a test, a command, a page).
- Read the code around the change: entry points, callers, data shapes, tests, the project rules
  (CLAUDE.md, AGENTS.md, CONTRIBUTING), `git log --oneline -15 -- <path>`.
- Several readings of the request? Name them. A choice that changes the design: ask one precise
  question. A detail with a sensible default: pick it and say so. Never assume silently.
- List the scope: every caller, test, type, config, migration, doc the change must reach. Nothing
  outside that list gets touched.

## 2. Design the smallest complete change

Take the first option that fully works:

1. Not needed? Skip it and say so in one line.
2. Already in the project (helper, component, pattern)? Use it the way nearby code does. Search the
   verb and the noun before writing.
3. Standard library or platform API? Use it.
4. Installed dependency? Use it. A new dependency needs a real reason; verify the exact package name
   exists and is maintained (`npm view <pkg>`, crates.io, PyPI): models invent package names.
5. Otherwise the minimum code that works, in the project's style, layers and error handling.

Design rules: data and types first; invalid states unrepresentable; one function, one job; no
abstraction with a single user, no option nobody sets, no error handling for impossible cases.
Shortcut with a known limit: comment `shortcut: <limit>, <when to upgrade>`.

## 3. Build surgically

- **Every changed line traces back to the request.** No reformatting, renaming or "improving"
  neighbouring code. Unrelated problem seen: mention it, do not fix it silently.
- Remove only what your change made dead.
- Errors: add context ("reading config X: ..."), never swallow (`catch {}`, `.ok()`, `_ =`), fail
  loud on programmer errors and gracefully on user, network and I/O errors.
- Edge cases on purpose: empty, null, huge, concurrent, slow, offline, unicode, time zones,
  Windows paths and `\r\n`.
- Boundaries: validate input server-side, parameterized queries, no shell strings, no secrets in
  code or logs.
- No query or network call in a loop, no O(n²) on growing data, timeouts on network calls.
- Logic that branches, parses, handles money or security gets a test that fails without the change.

## 4. Review, then prove

Read the full diff as a stranger who must maintain it, assuming there is a bug:

```
git status --short && git diff && git diff --staged
```

- [ ] Every part of the request done; nothing extra slipped in
- [ ] No debug prints, commented-out code, stray files, secrets, unintended lock-file changes
- [ ] Each changed function, type, route, config key or column: every caller found and still right
- [ ] Saved data, public APIs, CLI flags and env vars still compatible (or migrated on purpose)
- [ ] Inputs: empty, null, zero, negative, huge, unicode; bounds: first, last, one item
- [ ] Async: missing `await`, unhandled rejection, race, stale closure; resources closed on every path
- [ ] Errors shown to users are generic; details only in logs

Then run the project's own checks (build, types, lint, tests: find them in `package.json`,
`Cargo.toml`, `Makefile`, CI config) **now, in full**, and read the output and exit code. Run the
real thing too: the command, the page in the AgentDeck browser panel, the API call. Fix what you
find and repeat from the diff.

## Red flags: stop when you think

| Thought | Reality |
|---|---|
| "It should work now" / "probably fixed" | Not verified. Run it. |
| "The linter/build passed" | A build is not a test; a test is not the real app. |
| "Tiny change, no need to check" | Small diffs break callers too. |
| "While I'm here I'll clean this up" | Out of scope. Mention it. |
| "Let me add an option in case" | Speculative. Remove it. |
| "The subagent said it passed" | Check its diff and rerun the check yourself. |
| "I'll add tests later" | Later never comes. Now, or say it is untested. |

## Report

What changed and why (a few lines), the checks run with their result, what was not done or not
verified, risks and follow-ups. Never claim a check you did not run in this turn.
