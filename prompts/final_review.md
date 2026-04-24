
# Final Review

Before you begin, ground yourself in what this branch actually contains:

- Identify the merge-base against `origin/main`:
  `git merge-base HEAD origin/main`.
- Read the full commit history from merge-base to HEAD
  (`git log --reverse <merge-base>..HEAD`) and the cumulative diff
  (`git diff <merge-base>..HEAD`). The individual commits and their
  messages are the primary context for what this PR is meant to do —
  do not rely on a single tip commit or a task description alone.
- If the working tree has uncommitted changes, decide per-hunk whether
  each is worthwhile work that belongs in this PR. Commit the keepers
  as their own atomic commits with clear messages; discard or stash
  anything that is scratch, debug, or unrelated. Do not carry dirty
  state into the rebase.

When done, update your task list and perform the following:

1. Update TODO.md remove any now completed work. Mark them complete in
   TODO_INDEX.md. Add any newly deferred work and make sure to provide
   context in TODO.md as future agents will be responsible for completing
   it. (TODO_INDEX.md should be a one-liner to keep context low.)
2. Rebase changes onto the main origin tracking branch.
3. Verify ./run.sh presubmit is passing.
4. Commit and push changes to gitlab for review (include the test results
   in review as well).
5. Spawn a separate agent to review your PR with gitlab CLI.
6. Address its requests, do not defer any work.
7. Spawn another agent to perform final review against the PR, once again
   address concerns including P1/P2s.
8. Address any final comments and rebase the PR and push for final human
   review.
