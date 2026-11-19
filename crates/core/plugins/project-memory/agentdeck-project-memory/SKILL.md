---
name: agentdeck-project-memory
description: Use to onboard agents to an unfamiliar repository or maintain AGENTS.md, CLAUDE.md or GEMINI.md with verified commands, conventions and project-specific traps.
---

# Project memory

The agent instructions file is read by every agent at every start: each true line saves minutes,
each wrong or useless line costs every agent forever.

## 1. Learn the project (before the first edit)

Read in this order, stop when you can build and test:

1. Existing `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `.cursorrules`, `CONTRIBUTING.md`, `README.md`.
2. Manifests: `package.json` (scripts, packageManager, engines), `Cargo.toml` (workspace members),
   `pyproject.toml`, `go.mod`, `Makefile`, `justfile`, lock files (they name the package manager).
3. CI config (`.github/workflows/*`): the commands CI really runs are the truth.
4. Layout: `git ls-files | head -200` or a tree of the top two levels; entry points; where tests live.
5. Recent history: `git log --oneline -20` for commit style and what is moving.

Then **run** the build and the tests once. A command in the file that fails is worse than none.

## 2. Write or fix the file

Which file: the one the project already has. None: `AGENTS.md` (read by most agents); for Claude Code
add a `CLAUDE.md` containing `@AGENTS.md`, for Gemini set it in settings or add `GEMINI.md` the same
way. Never duplicate the same content in two files.

Keep it under ~150 lines. Only what an agent cannot guess from the code:

```markdown
# <Project>
<one line: what it is, stack and versions>

## Commands
- Install: `pnpm install`
- Dev: `pnpm dev` (http://localhost:5173)
- Test all: `pnpm test` · one file: `pnpm vitest run src/x.test.ts`
- Lint + types: `pnpm check`   (CI runs: test, check, build)

## Layout
- `api/` Fastify backend · `web/` React frontend · `db/migrations/` SQL, never edit an applied one

## Conventions
- <rule the code follows that a newcomer would break>

## Gotchas
- <trap found the hard way, with the fix>
```

Rules: exact commands that were run and work; paths that exist; rules phrased as do or do not; no
generic advice ("write clean code", "add tests"), no copy of the README, no secrets, no long history.

## 3. Keep it true while working

- Found a non-obvious command, flag, env var or trap that cost you time? Add one line under the right
  heading, now, not at the end.
- A line turned out wrong or stale: fix or delete it in the same change.
- The user states a lasting rule ("always use pnpm", "never touch generated/"): add it.
- Personal preferences that are not about this project do not go in the repository file.

## Report

What you added, changed or removed in the instructions file, and which commands you ran to check them.
