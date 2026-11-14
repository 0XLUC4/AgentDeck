---
name: agentdeck-git
description: Safe, clean git for agents - check state first, atomic Conventional Commits staged on purpose, branches, rebase or merge, conflict resolution, pull requests with gh, finishing a branch, undoing mistakes without losing work. Use when committing, branching, syncing, merging, resolving conflicts, opening or updating a PR, finishing a branch, or recovering from a git mistake.
---

# Git

**Never lose work, never act on history without being asked.** Commit or push only when the user
asks. Anything that discards work (`reset --hard`, `clean -fd`, `branch -D`, `checkout -- .`,
`restore` on edits, force push, `stash drop`) needs the user's explicit yes after you show what
would be lost.

## Before anything

```
git status --short --branch && git stash list && git log --oneline -5
```

Know what is staged, unstaged, untracked, and where the branch stands. On the default branch and
asked to commit work: create a branch first (`git switch -c feat/<short-name>`) unless the project
commits straight to it. "dubious ownership": tell the user; `git -c safe.directory=<path>` for a
one-off read only.

## Commits

- One logical change per commit; it builds and passes tests.
- Stage by name (`git add <files>`, `git add -p`). Never `git add -A` / `.` blindly. Check
  `git diff --staged` before committing: no `.env`, keys, build output, large binaries, stray files.
- Message: the repo's style first (`git log --oneline -15`); otherwise Conventional Commits
  `type(scope): summary`, imperative, under 72 characters; types feat, fix, refactor, perf, test,
  docs, build, ci, chore; body says *why*; breaking change `feat!:` plus a `BREAKING CHANGE:` footer.
- Pass multi-line messages with a heredoc or `-F file`, not escaped `\n`.
- A pre-commit hook fails: fix the cause and make a new commit. Never `--no-verify` unless asked;
  never `--amend` a commit that is pushed or not yours.

## Sync

- Own branch: `git fetch` then `git rebase origin/<base>`. Shared branch or a merge-based project:
  `git merge origin/<base>`.
- After a rebase: `git push --force-with-lease`, own branch only. Never plain `--force`, never on
  the default branch, never again after a rejected push without asking.

## Conflicts

1. `git status` lists them. For each file, understand both sides: `git log -p --merge -- <file>`,
   `git diff --ours/--theirs`.
2. Keep the intent of both changes, not one side by reflex. Remove every marker
   (`git diff --check`).
3. Lock files: take one side, then regenerate (`pnpm install`, `cargo update -p <crate>`).
4. Build and run tests, then `git add` and `git rebase --continue` / commit. Lost: `git rebase --abort`.

## Pull requests (gh)

- Self-review the diff first (`git diff <base>...HEAD`), tests green.
- `git push -u origin HEAD`, then `gh pr create --title "<like a commit>" --body-file <file>`. Body:
  what, why, how it was tested, screenshots for UI, risks.
- Follow-ups: `gh pr view`, `gh pr checks`, `gh run view --log-failed` for a red CI.

## Finishing a branch

Tests pass first (if not, stop and report). Then offer the user, and wait for the choice: merge
locally (re-run tests on the merged result), push and open a PR, or keep the branch as is. Delete
a branch or worktree only after it is merged or the user says so.

## Undo

| Want | Command |
|---|---|
| Unstage | `git restore --staged <file>` |
| Fix the last message, not pushed | `git commit --amend` |
| Undo a pushed commit | `git revert <sha>` |
| Move back, keep changes | `git reset --soft <sha>` |
| Park work | `git stash push -m "<msg>"` (`-u` for untracked) |
| Find lost commits or a dropped stash | `git reflog`, `git fsck --lost-found` |
| Discard edits in a file (lost for good) | `git restore <file>`, after the user's yes |

## Find things

`git log -S "<text>"` (when code appeared), `git log -L :<func>:<file>`, `git blame -w -C <file>`,
`git bisect run <test>` (first bad commit), `git diff <base>...HEAD` (branch changes).
