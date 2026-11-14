---
name: agentdeck-git
description: Clean git work - check state before acting, small atomic commits with Conventional Commit messages, branches, rebase vs merge, resolving conflicts, pull requests, undoing mistakes safely. Use when committing, branching, merging, resolving conflicts, opening a PR, or recovering from a git mistake.
---

# Git workflow

## Before anything

- `git status` and `git branch --show-current`. Know what is staged, unstaged, untracked.
- Never commit or push unless asked. On the main branch: create a branch first
  (`git switch -c feat/short-name`).
- "dubious ownership" on Windows/external drives: tell the user;
  `git -c safe.directory=<path>` for a one-off read is fine.

## Commits

- One logical change per commit; it builds and passes tests.
- Stage on purpose: `git add <files>` or `git add -p`; never blindly `git add -A` with stray files,
  secrets, `.env`, build output, large binaries.
- Conventional Commits: `type(scope): summary` in imperative, under 72 chars.
  Types: feat, fix, refactor, perf, test, docs, build, ci, chore. Body: *why*, not what.
  Breaking change: `feat!:` + `BREAKING CHANGE:` footer.
- Follow the repo's own style if it differs (`git log --oneline -15`).

## Branches and syncing

- Update: `git fetch` then `git rebase origin/main` on your own branch (clean history), or merge if
  the branch is shared or the project prefers merges.
- Never rewrite history others pulled. `push --force-with-lease`, never plain `--force`, and only
  on your own branch.

## Conflicts

1. `git status` lists the files. Open each; understand both sides (`git log -p --merge <file>`).
2. Keep the intent of both changes, not just one side. Remove all markers.
3. Build and run tests, then `git add` + `git rebase --continue` / commit.
4. Lock files: regenerate (`pnpm install`, `cargo update -p x`) instead of hand-merging.

## Pull requests

- Small and focused. Title like a commit. Body: what, why, how tested, screenshots for UI, risks.
- Self-review the diff first (`agentdeck-self-review`). CI green before asking for review.

## Undo safely

| Want | Command |
|---|---|
| Unstage | `git restore --staged <file>` |
| Discard local edits in a file (lost!) | `git restore <file>` — confirm first |
| Fix last commit message (not pushed) | `git commit --amend` |
| Undo a pushed commit | `git revert <sha>` |
| Move branch back, keep changes | `git reset --soft <sha>` |
| Find lost commits | `git reflog` |
| Park work | `git stash push -m "msg"` / `git stash pop` |

Ask before anything destructive: `reset --hard`, `clean -fd`, `branch -D`, force push, `restore` on
uncommitted work. Look at what would be lost first.

## Find things

`git log -S "text"` (when code appeared), `git blame -w -C <file>`, `git bisect run <test>` (first
bad commit), `git diff main...HEAD` (branch changes).
