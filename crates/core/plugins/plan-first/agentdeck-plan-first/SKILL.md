---
name: agentdeck-plan-first
description: Plan before coding big or vague work - clarify the goal one question at a time, map the code, then write the plan as a folder of linked Markdown files (one index linking architecture, backend, frontend, data, one file per feature, a file map and ordered steps, every file linked both ways), check its links, get approval, then execute step by step. Use when a task touches several files or systems, is a new project, a migration or a refactor, is unclear, or will be split between agents, before writing any code.
---

# Plan first

Pick the size first:

| Work | Plan |
|------|------|
| Small and clear (one file, obvious change) | No plan: just do it. |
| Medium (a few files, one feature, one layer) | One `PLAN.md` with the sections of `README.md` + `steps.md` from [reference/templates.md](reference/templates.md). |
| Big (new project, several features, backend + frontend, migration, several agents) | The linked plan folder below. |

## 1. Clarify

- Read the project first; never ask what the code or docs already answer.
- Write the goal in one sentence (what changes for the user) and the check that proves it is done.
- List non-goals: what you will not do.
- Unknowns that change the design: ask **one question at a time**, each with your proposed default and
  why. A detail with a sensible default: pick it and write it in the decisions.
- Several valid approaches: 2-3 in one line each with the trade-off, recommend one.

## 2. Map

Read entry points, existing modules and tests; never guess. Note: stack and versions, folder layout,
patterns to reuse, data already stored, external services, constraints (platforms, performance,
backward compatibility), how to build, run and test. For a new project, choose the stack and layout
and write them down as decisions.

## 3. Write the linked plan folder

Location: `docs/plans/<yyyy-mm-dd>-<slug>/` (or where the project keeps its docs). Templates for
every file: [reference/templates.md](reference/templates.md).

```
docs/plans/2026-03-02-billing/
├── README.md            index: goal, done when, map of every file, step table, dependency graph
├── architecture.md      layers, boundaries, data flow diagram, conventions
├── decisions.md         D1, D2... choice, alternatives, why
├── data.md              entities, fields, relations, migrations, seed data
├── backend/
│   └── <area>.md        one per backend area: endpoints or commands with exact contracts, services, jobs
├── frontend/
│   └── <area>.md        one per screen or UI area: routes, components, state, calls to backend contracts
├── features/
│   └── <feature>.md     one per user-facing feature: story, acceptance criteria, links to its backend,
│                        frontend and data parts and to its steps
├── files.md             every file to create, change or delete → feature, step, owner
├── steps.md             ordered steps: files, check, depends on, parallel group, owner
└── risks.md             risk → how you notice → what you do
```

Leave out a folder that has nothing in it (no `frontend/` for a CLI); add one the project needs
(`infra/`, `mobile/`, `jobs/`). Split by real boundary, not by habit.

### Linking rules (the plan is a graph, not a pile of files)

- Relative Markdown links only (`[POST /invoices](../backend/invoices.md#post-invoices)`), so they work
  on GitHub, in editors and offline. Link to headings with anchors for contracts and steps.
- Every file starts with a breadcrumb back to the index: `[Plan](../README.md) › Backend › Invoices`.
- `README.md` links every other file. No orphan: every file is linked from the index and at least one
  other file.
- Every feature links each backend contract, frontend area, data entity and step that implements it.
- Every backend and frontend area, entity and step links back to the features it serves
  (`Used by:` line). Links go both ways.
- A frontend call names the exact backend contract it uses, by link. A contract change means updating
  both sides in the same edit.
- One fact in one place: define a contract, an entity or a decision once and link to it; never copy it.
- Stable IDs: features `F1`, steps `S1`, decisions `D1`, risks `R1`, used in titles and links, never
  renumbered (a removed item is struck through with the reason).

### Content rules

- Contracts are exact: method, path or command, input and output types with field names, errors,
  auth. Entities have field types, constraints and relations.
- Each step is small, touches few files, ends green, and has an exact check (command, test name, or
  what you will see on screen). Order: data and types, then backend logic, then frontend, then docs.
  Riskiest unknown first when it could change the plan (a throwaway spike, labelled as such).
- Parallel steps (independent files) get a parallel group, their files, the files they must not
  touch, and their check, so each can go to another agent with only its step and the files it links.
- `README.md` holds a Mermaid graph of the steps and their dependencies, and a status per step
  (`todo`, `doing`, `done`, `blocked`).
- Defects: "TBD", "handle edge cases", "add validation", "update as needed", a name never defined, a
  step without a check, a feature without acceptance criteria, a link to nothing. Replace each with
  the concrete thing.
- Size follows the work: a plan much longer than the work it describes is too detailed, cut.

### Check the plan

Run `node <this skill folder>/scripts/check-plan.mjs docs/plans/<plan>`: it reports broken links and
anchors, orphan files, files without a link back to the index, and leftover placeholders. Fix
everything it reports. Without Node, do the same checks by reading the links.

## 4. Approval gate

Show the index (`README.md`) and the main decisions, then wait for an explicit yes before big or risky
work (data migration, public API, many files, deleting things). "Looks fine I guess" on a destructive
step is not a yes: ask again.

## 5. Execute

- Steps in order; run each step's check before starting the next.
- Keep the plan true: set each step's status in `README.md` and `steps.md` as you go, so anyone can
  resume from the files alone.
- Reality differs from the plan: stop, update the affected files (and their links), add a decision
  saying what changed and why, then continue.
- Several agents: give each one its step's anchor link; it reads the step, the feature and contracts
  it links, and nothing else.

## Report

At the end: steps done with their check results, deviations (with their decision IDs), what is left,
and the link to the plan's `README.md`.
