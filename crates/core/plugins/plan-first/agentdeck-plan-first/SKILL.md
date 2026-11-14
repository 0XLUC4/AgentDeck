---
name: agentdeck-plan-first
description: Plan before coding big or vague work - clarify the goal one question at a time, map the code, write a plan of small ordered steps each with an exact check, flag risks, mark parallel parts for several agents, get approval, then execute step by step. Use when a task touches several files or systems, is a migration, refactor or new project, is unclear, or will be split between agents, before writing any code.
---

# Plan first

Skip this for a small, clear change: just do it.

## 1. Clarify

- Read the project first; never ask what the code or docs already answer.
- Write the goal in one sentence (what changes for the user) and the check that proves it is done.
- List non-goals: what you will not do.
- Unknowns that change the design: ask **one question at a time**, each with your proposed default
  and why. A detail with a sensible default: pick it and state it.
- Several valid approaches: give 2-3 in one line each with the trade-off, recommend one.

## 2. Map

Files to create or change (with paths), existing patterns to reuse, data already saved, constraints
(versions, platforms, performance, backward compatibility). Read entry points and tests, do not guess.

## 3. Write the plan

Write it for an engineer who has never seen this code base:

```
Goal: <one sentence>    Done when: <check>
Non-goals: <list>

1. <step> — files: <paths> — check: <exact command / test / what you will see>
2. <step> — files: ... — check: ...   [parallel with 3]
3. ...
Risks: <what could go wrong> → <how you will notice> → <what you will do>
```

- Order: data and types, then logic, then UI, then docs. Riskiest unknown first when it could change
  the plan (a throwaway spike, labelled as such).
- Each step small, touching few files, ending green (builds, tests pass).
- Parallel steps (independent files): give each its files, goal, check, and the files it must not
  touch, so it can go to another agent.
- Defects in a plan: "TBD", "handle edge cases", "add validation", "update as needed", a name never
  defined, a step without a check. Replace each with the concrete thing.
- Plan much longer than the request: too detailed, cut.

## 4. Approval gate

Big or risky work (data migration, public API, many files, deleting things): show the plan and wait
for an explicit yes. "Looks fine I guess" on a destructive step is not a yes: ask again.

## 5. Execute

- Steps in order; run each step's check before the next one.
- Keep a short progress list (done / doing / next) in your messages or a task list, so the work can
  be resumed by someone else.
- Reality differs from the plan: stop, update the plan, say what changed and why.

## Report

At the end: steps done with their check results, deviations from the plan, what is left.
