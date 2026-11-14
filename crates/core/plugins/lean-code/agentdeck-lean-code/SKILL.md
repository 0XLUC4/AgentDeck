---
name: agentdeck-lean-code
description: Solve the whole task with the least new code - reuse what the project already has, no speculative features, abstractions or dependencies, delete before adding, finish every caller and test the change touches. Use on any coding task (write, fix, refactor, review, choose a dependency) and when the user says simple, minimal, lean, yagni, KISS or complains about bloat or over-engineering.
---

# Lean code

The best code is code you did not have to write. Be lazy about the solution, never about finishing it.

## Before writing

1. Read the task and the code it touches. Find how the project already does similar things.
2. List every place the change must reach: callers, tests, fixtures, types, config, exports, docs,
   migrations.
3. Note what it could break for users: data lost or exposed, callers that stop working, saved files
   that no longer load.

That list is the scope. Anything else is extra and stays out.

## Pick the first option that fully works

1. **Not needed?** Skip features, options and flexibility nobody asked for. Say in one line what you
   skipped. A vague request ("make me X") gets the smallest version that does the core job.
2. **Already in the project?** A helper, component, hook, service or pattern: use it the way nearby
   code does. Search before writing (`grep` the verb and the noun).
3. **Standard library or platform?** `fetch`, `URL`, `Intl`, `structuredClone`, `<dialog>`,
   `<details>`, CSS grid, `std::fs`, `pathlib`... unless the project has its own wrapper.
4. **Installed dependency?** Use it. Never add a dependency for a few lines; a new one needs a real
   reason (security, correctness, lots of code saved) and a maintained, small package.
5. **One clear line?** Write one line, but not one that needs decoding.
6. **Otherwise** the minimum code that works.

## Smells to remove

| Smell | Instead |
|---|---|
| Interface or trait with one implementation | the concrete type |
| Factory, manager, service class around one function | the function |
| Config option nobody sets | a constant |
| Wrapper that only renames | call the original |
| Generic "utils" for one use | inline it |
| Converting data to a new shape and back | keep the shape the platform gives |
| Copy-pasted block 3+ times | one function (2 copies are fine) |
| Dead code, unused params, old flags | delete them |
| Comment repeating the code | delete it; keep only the *why* |

## Rules

- Keep the structure the project already has: its layers, names, error style, file layout.
- Smallest diff that is complete wins. Do not reformat or rename unrelated code.
- Bug fix: grep every caller first, fix the root cause once in the shared code.
- Moved or merged code keeps its validation and error handling.
- New logic that branches, loops, parses, handles money or security gets one small test.
- A shortcut with a known limit gets a comment: `shortcut: <limit>, <when to upgrade>`.
- Between two options of equal size, take the one correct on edge cases.

Never cut: validation of untrusted input, error handling that prevents data loss, security,
accessibility, anything the user asked for.

## Report

What changed, in a few lines. Last line: what you skipped or did not check, and any risk.
