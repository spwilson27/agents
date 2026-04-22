**Scope of this invocation: bug fixes only.** Your deliverables are source changes that flip each failing regression test from red to green, one bug at a time. Do NOT author new tests (those already exist from prompt_02.md), do NOT weaken existing regression tests to make them pass, do NOT open PRs. Landing happens in prompt_04.md.

You are the fix orchestrator. Reproduction (prompt_02.md) produced a feature branch with one failing regression test per non-withdrawn bug in `docs/plan/bug-bash.md`. Your job is to dispatch subagents to fix each bug, validating each fix against its regression test.

Branching

- Continue on the feature branch produced by prompt_02.md. Never commit on main.
- Every subagent operates inside its own git worktree off this branch.

Phase 0 — Ground yourself

1. Read AGENTS.md, CLAUDE.md, and `docs/plan/bug-bash.md` in full.
2. Identify the feature branch from the phase summary at the bottom of the registry. Confirm it exists locally; confirm the expected count of failing regression tests matches by running the suite.
3. Seed a TaskCreate list — one task per non-withdrawn, non-blocked bug. Track a dependency graph where bugs that touch the same file serialize (same-file concurrency = 1); otherwise run in parallel.

Phase 1 — Dispatch fixers

Loop. Compute ready set (unclaimed, no in-flight sibling touching the same files). For each ready bug, spawn a subagent (Agent tool, isolation: "worktree"). Default concurrency cap 4. Each brief must include:

- The full bug entry verbatim, including the `Regression test:` cross-reference.
- Hard requirements:
  * "You are working in a git worktree at <path> on branch <branch>. Never touch the main clone or main branch."
  * Run the regression test first and confirm it fails with the expected error.
  * Fix the root cause. Do NOT modify the regression test's assertions, inputs, or expected values. If the test looks wrong (asserts on the wrong invariant), surface that instead of editing it.
  * After the fix, the regression test MUST pass, and the full existing test suite MUST still pass.
  * No stubs, no `todo!()`/`unimplemented!()`, no `#[ignore]`, no `--no-verify`. Fix root causes, not symptoms.
  * Use atomic commits with the bug id in the message (e.g. `fix(bug-042): guard empty input before indexing`).
- Required return payload: bug id, files modified, commit SHA(s), regression-test output (pass), full-suite output (pass), and an explicit verdict: `fixed` / `incomplete` / `regression-test-wrong` / `blocked`.

Phase 2 — Validate and merge

When a subagent returns, before marking the bug DONE:

1. Re-run the specific regression test from the worktree — must pass.
2. Re-run the full test suite from the worktree — must pass.
3. Read the diff. Reject if you find: changes to the regression test body, `#[ignore]` added anywhere, skipped assertions, stubs, TODO/FIXME added by the worker, scope creep (changes unrelated to the bug), convention violations per CLAUDE.md, or new abstractions the fix didn't require.
4. Handle outcomes:
   - **fixed**: merge the worktree branch into the feature branch with `--no-ff` (preserve the atomic fix commit). Delete the worktree and its branch. Update the registry entry with `Fixed: <commit SHA>`. Mark the task DONE.
   - **incomplete** or dirty: spawn a FRESH subagent with the prior worker's output and your specific file:line objections. Do not accept "mostly working."
   - **regression-test-wrong**: the subagent claims the test asserts on the wrong invariant. Spawn an independent adjudicator subagent to rule. If the adjudicator agrees, update the registry entry, have the adjudicator (not the original fixer) author the corrected test in a new worktree, then respawn the fixer against the corrected test. If the adjudicator disagrees, respawn the fixer with the adjudicator's reasoning and a reminder that the test is authoritative.
   - **blocked**: record the blocker in the registry entry as `Blocked (fix): <reason>` and mark the task BLOCKED. Move on.

Phase 3 — Integration sweep

After every non-withdrawn, non-blocked bug is DONE:

1. Run the full test suite on the feature branch. Every regression test must pass; no other test may have regressed.
2. Spawn a reviewer subagent to audit: every non-withdrawn bug has a `Fixed:` entry pointing at a real commit, every `Blocked (fix):` has a concrete reason, no regression-test assertion was weakened (`git diff main -- <regression test paths>` must show only additions or the adjudicated corrections), and CLAUDE.md conventions were followed. Address every concern before exiting.
3. Append a phase summary to `docs/plan/bug-bash.md`: total fixed, blocked, adjudicated-test-corrections, and the feature branch SHA ready for prompt_04.

Rules

- You orchestrate; subagents implement.
- Never commit to or push main. Never force-push, never `--no-verify`, never `#[ignore]` a regression test.
- Never weaken a regression test to make it pass. If the test is wrong, go through adjudication.
- One bug, one atomic fix commit. No bundled multi-bug commits unless an adjudicator explicitly documents the coupling.
- Resolve ambiguity autonomously; log the choice in the registry entry.
