---
name: agentdeck-plan-first
description: Plan before coding big or vague tasks - clarify the goal, map the code involved, split the work into small ordered steps with a check for each, spot risks early, then execute step by step. Use for features touching several files, migrations, refactors, new projects, unclear requests, or work split between several agents.
---

# Plan first

Ten minutes of planning saves hours of rework. Skip it for one-file, obvious changes.

## 1. Goal

- One sentence: what changes for the user. One line: how you will check it is done.
- Out of scope: list what you will not do.
- Unknowns that change the design: ask now, in one message, with your proposed default for each.

## 2. Map

- Find the code involved (search, read entry points, data model, tests). List files to change.
- Existing patterns to reuse. Constraints: versions, platforms, performance, compatibility, data
  already saved.

## 3. Steps

Small, ordered, each one shippable or at least compiling and tested:

```
1. <step>  — check: <command/test/what you will see>
2. <step>  — check: ...
```

- Order: data/types first, then logic, then UI, then docs. Riskiest unknown first when it could
  change the plan (spike it).
- Each step touches few files and ends green.
- Mark steps that can run in parallel (independent files) — useful to split between agents. Give
  each parallel task its files, its goal, its check, and what not to touch.

## 4. Risks

For each: what could go wrong, how you will notice, what you will do (data migration with backup,
feature flag, fallback).

## 5. Execute

- Do the steps in order; run the check after each one.
- Reality differs from the plan: stop, update the plan, say what changed.
- Keep a short progress list (done / doing / next) so the work can be resumed.

## Report

Plan up front (short), then at the end: steps done, checks passed, deviations, what is left.
