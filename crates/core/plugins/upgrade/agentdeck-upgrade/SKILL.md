---
name: agentdeck-upgrade
description: Use to upgrade outdated or vulnerable dependencies, frameworks, language runtimes and toolchains. Check migration guides and codemods for each major version.
---

# Upgrade

**One major version at a time, green after each step, never from memory: read the migration guide.**

## 1. Inventory

- Start from a clean tree on a branch: `git status` empty, tests green before anything changes.
  Already red: note which tests fail, so the upgrade is not blamed for them.
- List what is outdated and vulnerable with the project's tools: `pnpm outdated` / `npm outdated`,
  `pnpm audit`, `cargo outdated` / `cargo update --dry-run`, `cargo audit`, `pip list --outdated`,
  `pip-audit`, `go list -m -u all`, `govulncheck ./...`.
- Group: patch and minor (usually safe together), each major on its own, framework and its plugins
  together (React + react-dom + types; Next.js + eslint-config-next).

## 2. Read before changing

For each major crossed: the official changelog, release notes and migration guide, found with the
docs or web research, matched to the exact from and to versions. Write down the breaking changes that
touch this project (search the code for each removed or renamed API) and the official codemod, if any.

## 3. Upgrade in steps

1. Patch and minor updates within the allowed ranges; install; build and test.
2. Each major: bump it and its companion packages; run the codemod
   (`npx @next/codemod`, `npx react-codemod`, `ng update`, `cargo fix --edition`); fix what remains,
   following the guide, not by guessing; build and test; commit.
3. Language or runtime version: update every place it is pinned (`.nvmrc`, `engines`,
   `rust-toolchain.toml`, `pyproject.toml`, Dockerfile, CI matrix) in the same step.
4. Deprecation warnings in build and test output: fix them now; they are next major's errors.

Rules: keep the lock file and use the project's package manager only; never delete the lock file to
"fix" a conflict; no `--force` or `--legacy-peer-deps` without explaining the conflict it hides; a
peer dependency conflict means the companion packages need the same upgrade.

## 4. Prove it

Clean install from the lock file (`pnpm install --frozen-lockfile`, `npm ci`, `cargo build --locked`),
then build, types, lint and the full test suite, then run the app and use its main flows (web: in the
AgentDeck browser panel, console and network clean). Re-run the audit: no new advisory.

## Report

Table of package, from, to; breaking changes handled and how; codemods run; what was not upgraded and
why (blocked by, waiting for); the checks run with their result.
