# Agent instructions

## Work in a worktree

Do all work in a git worktree, never in the main checkout. Before your first edit, create a worktree on a new branch (or switch into an existing one for the task) and make every change, build and commit there.

```sh
git fetch origin
git worktree add .claude/worktrees/<name> -b worktree-<name> origin/main
```

- Never commit directly to `main`; changes reach `main` through a pull request from the worktree's branch.
- Commit your work in the worktree before finishing, so it survives the worktree being removed.
- Don't use `git stash`: the stash is shared by every worktree, so another session can pop your changes. Set work aside with a temporary commit instead.
- Read-only tasks (answering questions, searching, reviewing) can run in place.
