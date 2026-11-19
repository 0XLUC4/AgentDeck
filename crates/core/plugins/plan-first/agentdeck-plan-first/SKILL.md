---
name: agentdeck-plan-first
description: Use before coding unclear work, new projects, multi-system refactors, a migration or tasks split between agents. Map dependencies and size the plan to the work.
---

# Plan first

Choose the smallest plan that fits.

- Small, clear change in one file: implement and verify directly.
- One feature in one layer: write one `PLAN.md`. State the goal, completion check and scope. List steps with stable IDs, dependencies, status, affected files and an exact check each. End with risks and mitigations. Keep this plan current during execution.
- Multiple features, layers, agents or a substantial migration: create a linked plan in the project's usual location or `docs/plans/<yyyy-mm-dd>-<slug>/`. Use [reference/templates.md](reference/templates.md) for the file layouts.

## Ground the plan

Read entry points, tests, build commands, versions and existing data. Identify reusable patterns, external services and compatibility constraints. Define what changes for the user and how to prove it. Record consequential choices, alternatives and trade-offs. Ask one question at a time when an unknown changes the design. Propose a default and explain why.

## Linked plans

Index all files in `README.md`. Show dependencies in a Mermaid graph and track step status (todo, doing, done, blocked). Include architecture, decisions, affected data and service areas, one file per feature, a file/owner map, ordered steps and risks with detection and mitigation. Omit irrelevant areas.

- Give features, steps, decisions and risks stable IDs (F1, S1, D1, R1). Retire IDs with a reason.
- Link relatively to exact headings. Each file starts with a breadcrumb to the index and has an incoming link from another plan file besides the index.
- Link each feature's acceptance criteria to its steps, contracts, UI and entities. Each of those links back to its features. Define each fact once. Link the frontend call to its backend contract and update both together.
- Specify method/path, input, output, errors and authorization. Give entity fields, types, constraints and relations.
- Run `node <this skill folder>/scripts/check-plan.mjs <plan-folder>`. Resolve broken links, missing anchors, orphan files and placeholders. Without Node, inspect them manually.

## Execute and verify

Sequence small steps with concrete checks. Resolve design-changing uncertainty with an early spike, then order types/data, backend, UI and docs by dependency. For parallel work identify the owner, group, allowed files, excluded files and verification command. Give each agent its step link.

Present the index and main decisions for explicit approval before big or risky work such as a data migration, public API change or deletion. Verify each step before starting its dependents. Update status in the index and steps as you go. If reality differs, update affected links and record the decision. Finish with the plan path, completed checks, deviations and outstanding work.
