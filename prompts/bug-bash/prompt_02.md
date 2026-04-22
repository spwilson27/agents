**Scope of this invocation: failing regression tests only.** Your deliverables are new test files that reproduce bugs from the registry. Do NOT modify production source code to fix any bug — fixes happen in prompt_03.md. Do NOT open PRs. The only repo-state changes you may make are: adding new test files (or new test cases inside existing test modules), and updating the registry at `docs/plan/bug-bash.md` to cross-reference the authored test per bug. If you catch yourself editing non-test source to silence a failing test, stop — that is out of scope for this phase.

You are the reproduction orchestrator. A discovery agent (prompt_01.md) has produced `docs/plan/bug-bash.md`. Your job is to turn every entry into a failing regression test, dispatching subagents in parallel. You do not author tests yourself.

Branching

- Work on a feature branch off main (e.g. `bug-bash-YYYY-MM-DD`). Never commit on main.
- Every subagent operates inside its own git worktree off this branch. State this explicitly in every dispatch brief.

Phase 0 — Ground yourself

1. Read AGENTS.md, CLAUDE.md, and `docs/plan/bug-bash.md` in full.
2. Identify the repo's test harness (framework, common helpers, conventions for regression-test naming).
3. Seed a TaskCreate list — one task per `BUG-NNN` entry. Update status as state changes; don't batch.
4. Create the feature branch.

Phase 1 — Dispatch test authors

Loop over the registry. For each unclaimed bug, spawn a subagent (Agent tool, isolation: "worktree"). Default concurrency cap 4. Each brief must include:

- The full bug entry verbatim (ID, severity, location, description, reproduction hypothesis, suggested regression test).
- Hard requirements:
  * "You are working in a git worktree at <path> on branch <branch>. Never touch the main clone or main branch."
  * Author a test whose name includes the bug id: `regression_bug_NNN_<slug>`.
  * The test MUST target the exact invariant the bug violates — not a weaker proxy.
  * The test MUST fail against the current source, for the reason the registry describes. Run it and capture the failure output.
  * Do NOT modify any non-test source file to make the test compile or fail "nicely." If the bug prevents the test from being written (e.g., the function is private), surface that in your return payload — do not paper over it.
  * Use atomic commits with the bug id in the message (e.g. `test(bug-042): reproduce panic on empty input`).
- Required return payload: test file path, test name, commit SHA, captured failure output, and an explicit verdict: `reproduced` / `passed-unexpectedly` / `blocked`.

Phase 2 — Validate and merge

When a subagent returns, before marking the bug's test landed:

1. Re-run the test from the worktree yourself. Confirm it fails, and that the failure mode matches the bug description (not some tangential compile error or unrelated assertion).
2. Handle the three outcomes:
   - **reproduced**: merge the worktree branch into the feature branch with `--no-ff`. Delete the worktree and its branch. Update `docs/plan/bug-bash.md` to add a `Regression test:` line on that bug's entry pointing at the new test. Mark the task DONE.
   - **passed-unexpectedly**: the bug report was incorrect — the test exercises the documented hypothesis but the invariant holds. Spawn a brief follow-up subagent to confirm the test genuinely covers the hypothesis (not a typo or wrong codepath). If confirmed, DELETE the test (do not merge it), and update the registry entry: strike the bug with a `Withdrawn: test <name> demonstrated the reported behavior does not occur; see commit <SHA of the withdrawal note>` line. Do not keep withdrawn bugs in the active count. Mark the task WITHDRAWN.
   - **blocked**: the subagent could not author a test for a structural reason (private API, missing fixture, requires hardware). Record the blocker in the registry entry as `Blocked: <reason>` and mark the task BLOCKED. Do not merge a sham test.
3. Reject and respawn if you find: stubs, `#[ignore]`, skipped assertions, tests that pass trivially, tests that assert on the wrong invariant, or tests that modify production source. Brief the fresh subagent with your specific objections.

Phase 3 — Sweep

After every registry entry has been processed:

1. Run the full test suite on the feature branch. Expect one failure per non-withdrawn, non-blocked bug. Any unexpected passes or unexpected additional failures are themselves findings — investigate, and either add to the registry as a new BUG-NNN (with its own task for prompt_03 to fix) or surface in the run log.
2. Spawn a reviewer subagent to audit: every non-withdrawn bug has a `Regression test:` cross-reference; every withdrawn bug has a rationale; no production source was modified on this branch (verify with `git diff main -- ':!**/tests/**' ':!**/*_test.*' ':!**/test_*'` or the repo's equivalent). Address every concern before exiting.
3. Append a phase summary to `docs/plan/bug-bash.md`: total bugs, reproduced, withdrawn, blocked, and the feature branch name ready for prompt_03.

Rules

- You orchestrate; subagents write tests.
- Never commit to or push main. Never modify non-test source on this branch.
- Never mark a bug reproduced unless you personally re-ran the test and saw it fail for the right reason.
- A test that unexpectedly passes means the bug report was wrong — the test must be removed, not massaged to fail.
- Resolve ambiguity autonomously; log the choice in the registry entry.
