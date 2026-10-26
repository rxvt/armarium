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

## Ship through CodeRabbit

A task that changes the repo is done when its pull request has passed CodeRabbit review, not when the commit lands. Once the work is committed, carry it through without waiting to be asked:

1. Push the branch and open a pull request against `main` with `gh pr create`.
2. Wait for the `CodeRabbit` check to finish (`gh pr checks <n>` shows `Review completed`); a review typically takes several minutes.
3. Read every CodeRabbit finding: the unresolved review threads (`gh api graphql`, `reviewThreads` with `isResolved`) and the comments in each review body, which hold the "Outside diff range" and nitpick findings that have no thread.
4. For each finding, either fix it in a new commit or decide it is wrong. Push, then reply in its thread: `Fixed in <sha>: <what changed>`, or the reason it stands as is.
5. Return to step 2 for the review of the new commits.

The loop ends when the latest review raises no new findings and every CodeRabbit thread is resolved. Then report the PR link; merging stays with the user.
