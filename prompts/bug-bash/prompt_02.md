**Scope of this invocation: one bug only.** You are reproducing a single work
item: `{work_item}`. Turn that registry entry into a failing regression test
(or mark it withdrawn/blocked with clear rationale). Do NOT modify production
source code to fix the bug. Do NOT open PRs. Do NOT dispatch subagents for
other bugs — other invocations handle the rest of the queue.

Registry file: `{registry_path}` (read the full file; edit only this bug's
section and only as allowed below).

Concurrency: this run is scoped to `{work_item}` only; do not start parallel
reproduce work for other bugs.

State file: `{reproduce_state}`.
Search snapshot: `{search_state}` if present.
{restart_mode}

Ownership

- You may write tests, git branches/worktrees, worker manifests, and update
  `{registry_path}` for `{work_item}` only.
- You may read and merge-update `{reproduce_state}` for all keys (read the full
  file, update only `{work_item}`, write via `.tmp` + atomic rename).
- You may append or rewrite `docs/bugs/reproduce-summary.md` only when you can
  verify every other work item in `{reproduce_state}` is already in a terminal
  state (`reproduced`, `withdrawn`, `blocked`, or terminal `failed`); keep the
  summary consistent with the state file.

Phase 0 — Ground yourself

1. Read AGENTS.md, CLAUDE.md, README.md, test-related docs, `{search_state}` if
   it exists, and `{registry_path}` in full.
2. If `{reproduce_state}` exists, load it and reconcile the entry for
   `{work_item}` before acting (see Resume reconciliation).
3. Identify the repo's test harness and the narrowest command that can run one
   new regression test.

Durable state

Persist `{reproduce_state}` before destructive git operations and after every
status transition for `{work_item}`. Use this status machine:

`pending -> in_progress -> reproduced | withdrawn | blocked | failed`

Do not add a top-level `coordinator_commit` to `{reproduce_state}`. Commit
accounting belongs on each reproduced item.

State entries should include at least:

```json
{
  "docs/bugs/src/lib.md#BUG-001": {
    "status": "reproduced",
    "worktree": "../agents-repro-src-lib-bug-001",
    "branch": "bug-bash/repro/src-lib-bug-001",
    "started_at": "<UTC timestamp>",
    "result_file": ".agents/bug-bash/results/src-lib-bug-001.json",
    "worker_commit": "<commit in worker branch>",
    "coordinator_commit": "<commit on coordinator branch after merge or cherry-pick>"
  }
}
```

Resume reconciliation

- If `{work_item}` is `in_progress` with a complete manifest, validate before
  deciding the next state.
- If `in_progress` with a worktree but no manifest, inspect the worktree: run
  the narrow test, commit valid test edits, write the manifest, then aggregate.
- If `in_progress` and the worktree disappeared, mark `failed` with a reason or
  requeue as `pending`.
- If `reproduced`, verify the test commit is on the coordinator branch or
  re-merge/cherry-pick from the recorded branch.
- If the registry already lists `Regression test:` for this bug and the test
  exists on the coordinator branch, treat as terminal.

Work plan

1. Mark `{work_item}` `in_progress` in `{reproduce_state}` (unless already
   terminal), then create a dedicated worktree/branch from the coordinator
   checkout.
2. Author the narrowest failing regression test for the documented invariant.
3. Commit test changes with the bug id in the commit message.
4. Write `.agents/bug-bash/results/<slug>.json.tmp`, then rename to
   `.agents/bug-bash/results/<slug>.json`.

Worker manifest format:

```json
{
  "work_item": "{work_item}",
  "status": "reproduced",
  "test_file": "tests/regression.rs",
  "test_name": "regression_bug_001_src_lib_empty_input",
  "command": "cargo test regression_bug_001_src_lib_empty_input",
  "failure_excerpt": "assertion failed: ...",
  "worker_commit": "abc1234",
  "notes": "Fails for the documented invariant."
}
```

Allowed manifest statuses: `reproduced`, `withdrawn`, `blocked`,
`needs-review`.

What counts as a regression test

A valid regression test MUST:
- Call the production code (functions, structs, traits) directly and observe
  incorrect *runtime* behaviour: a wrong return value, a panic, a failed
  assertion on live data, etc.
- Fail on current source *because the code does the wrong thing*, not because
  the code *looks* a certain way.

A static-analysis test is NOT a regression test. Specifically, you MUST NOT:
- Use `include_str!`, `fs::read_to_string`, or any other mechanism to load
  source files inside a test, then search for or assert on text patterns.
- Assert on function names, identifiers, or code structure via string matching.
- Use `proc_macro2`, `syn`, or any parser to inspect AST of production files.

A static-analysis test that passes after someone renames a variable or adds a
comment is useless. If you cannot think of a way to exercise the bug through
the production API, mark the item `blocked` with a clear explanation rather
than substituting a source-inspection test.

Validation and aggregation

- Rerun the reported test command in the worktree before accepting `reproduced`.
- Accept `reproduced` only when the test fails against current source for the
  documented invariant.
- Never edit production source to make the suite pass during reproduce.
- If reproduced, merge or cherry-pick the test commit into the coordinator
  branch. Annotate this bug in `{registry_path}` with
  `Regression test: <path>::<test name>` and `Failure command: <command>`.
- Capture `git rev-parse HEAD` on the coordinator branch after merge as
  `coordinator_commit`. The test commit from the worktree is `worker_commit`.
- Copy the accepted manifest into the coordinator checkout at
  `.agents/bug-bash/results/<slug>.json`. Normalize `commit` to `worker_commit`
  if needed, then add `coordinator_commit`.
- If withdrawn, annotate with `Withdrawn: <reason>`. If blocked, use
  `Blocked: <reason>`.
- If needs-review, inspect and resolve to one of the terminal outcomes.

Sweep for this bug

1. Confirm no unintended production edits outside allowed test-only patterns.
2. Verify `{work_item}` in `{reproduce_state}` matches the manifest and
  registry annotations.
3. If every entry in `{reproduce_state}` is terminal, write
   `docs/bugs/reproduce-summary.md` with totals.

Rules

- Prefer one precise failing test over broad failures.
- Never mark reproduced unless the failure points at the documented invariant.
- Never keep sham tests (ignored, skipped assertions, tautologies, or static analysis via include_str!/source text search/AST inspection — these are not runtime tests).
- Use unique branch and test names so merges stay idempotent.
