# `agents todo-workflow` — Implementation & Verification Plan

## Goal

Add a `todo-workflow` subcommand to the `agents` CLI that runs the three-phase
orchestration defined by `prompt_01.md`, `prompt_02.md`, and `prompt_03.md`:

1. Plan — meta-orchestrator produces `docs/plan/meta-plan/` artifacts.
2. Implement — implementation orchestrator drives subagents on a feature branch.
3. Land — last-mile reviewer rebases, runs presubmit, MRs, and lands.

Each phase is a long-running agent invocation using the same `AgentCli`
infrastructure already used by `commit`.

## Current-state anchors

- CLI entry: `src/main.rs:18` (`Command` enum, extend here).
- Subcommand dispatch: `src/main.rs:44`.
- Agent invocation + streaming: `src/lib.rs:343` (`run_agent`) and
  `src/lib.rs:375` (`run_codex_command`).
- Agent binary selection + env override: `src/lib.rs:35`.
- Timeout handling: `src/lib.rs:213` (`agent_timeout`) — 30s default; too low
  for multi-hour orchestration, must be overridable per-phase.
- Target files list (unused here but precedent for const tables):
  `src/lib.rs:19`.
- Existing integration test shape: `tests/cli.rs`.
- Prompt files already on disk: `prompts/todo-workflow/prompt_01.md`,
  `prompt_02.md`, `prompt_03.md`.

## Design

### CLI surface

```
agents todo-workflow [--cli <agent>] [--root <dir>] [--phase plan|implement|land|all]
                     [--prompts-dir <dir>] [--resume-from <phase>]
                     [--dry-run]
```

- `--cli`: reuses `AgentCli` (`src/lib.rs:27`). Default `claude` (long-running
  orchestration; codex `exec` is one-shot and less suited — decision
  documented, not deferred).
- `--root`: repo to operate on. Default `.`.
- `--phase`: which phases to run. Default `all`. Explicit values let users
  re-run a single phase.
- `--prompts-dir`: where `prompt_0{1,2,3}.md` live. Default:
  `<root>/prompts/todo-workflow`, overridable via `AGENTS_PROMPTS_DIR` env
  var. Prompts are
  read at runtime — not compiled in — so editing a prompt does not require
  a rebuild. Decision: runtime read > `include_str!` because the prompts
  are the product here and iterate faster than the binary.
- `--dry-run`: print the resolved plan (agent, prompt path, cwd, env) and
  exit 0 without invoking the agent. Used by tests and humans.

### Execution model

- Each phase maps to one agent invocation. The agent itself spawns
  subagents per its prompt; the CLI only launches the top-level orchestrator.
- The prompt file is read from disk and passed as stdin prompt to the agent,
  identical to how `commit` passes the prompt today (`src/lib.rs:343`).
- Phases run sequentially. The CLI stops on the first phase that fails and
  reports which phase + how to resume (`--phase implement` etc.).
- Between phases, the CLI prints a clear banner (`=== Phase 2: implement ===`)
  to stderr so interleaved agent output is navigable.

### Timeout

The current `AGENTS_TIMEOUT_SECS` gate (`src/lib.rs:213`) defaults to 30s and
kills the child. Orchestration runs for hours. Changes:

- Introduce `agent_timeout_for(purpose: TimeoutPurpose)` where `Commit` keeps
  the 30s default and `Workflow` defaults to `None` (no timeout).
- New env var `AGENTS_WORKFLOW_TIMEOUT_SECS` overrides for the workflow
  path; unset = no timeout.
- `run_command` already accepts `Option<Duration>` (`src/lib.rs:443`) — pass
  `None` to disable. Verified: the `None` branch calls
  `child.wait_with_output()` directly (`src/lib.rs:473`), so this is a
  one-line plumbing change, not a rewrite.

### Streaming output

`commit` buffers agent output and parses at the end. For `todo-workflow`,
the user needs to watch progress live. Decision: stream stdout/stderr
straight through via `Stdio::inherit()` for the workflow path. Parsing the
final message is unnecessary — the agent's deliverable is files on disk,
not a returned string.

New helper: `run_agent_interactive(cli, root, prompt)` that:
- Builds the same `Command` as `run_agent` per CLI variant.
- Pipes the prompt to stdin.
- Inherits stdout/stderr.
- Returns `Ok(())` on exit-zero, `AgentsError::CommandFailed` otherwise.
- Applies the workflow timeout (or none).

Keeps `run_agent` unchanged so `commit` is not disturbed.

### File layout

- `src/lib.rs`: add `pub fn todo_workflow(root, cli, phases, prompts_dir,
  dry_run)`, a `Phase` enum, and the `run_agent_interactive` helper.
- `src/main.rs`: add the `TodoWorkflow` `Command` variant and wire dispatch.
- No new modules needed; the file is ~600 lines and the addition is ~150.
  Splitting can happen later if warranted.

### Impacted APIs

- Public (crate): new `todo_workflow` function, new `Phase` enum,
  `AgentsError` gains no new variants (reuses `CommandFailed` / `Io` /
  `TimedOut`).
- CLI: new subcommand, additive — no existing flag or behavior changes.
- Internal: `run_command`'s signature is unchanged; a sibling
  `run_agent_interactive` is added.

### Configuration

- `AGENTS_WORKFLOW_TIMEOUT_SECS` (new, optional, unset = no timeout).
- `AGENTS_PROMPTS_DIR` (new, optional, overrides prompt lookup).
- Existing `AGENTS_<CLI>_BIN` overrides continue to work unchanged.

### Performance

Agent calls dominate. CLI overhead is negligible. No benchmarks required.

### Trade-offs (decided, not deferred)

- **Prompts on disk vs `include_str!`**: on disk. Prompts iterate faster
  than the binary and are the product.
- **Default CLI = claude vs codex**: claude. Codex `exec` is one-shot and
  the codex path writes to a temp file (`src/lib.rs:375`) rather than
  streaming — wrong shape for multi-hour work.
- **Sequential vs parallel phases**: sequential. Each phase's output is
  the next phase's input.
- **Timeout default**: none for workflow, 30s stays for commit. A default
  timeout on orchestration would kill legitimate long runs.
- **One subcommand with `--phase` vs three subcommands**: one with
  `--phase`. The phases are a fixed pipeline; three subcommands would
  duplicate flag plumbing.

## Validation plan

### Unit tests (`src/lib.rs` `#[cfg(test)]`)

1. `phase_parses_from_clap` — `Phase::All` expands to `[Plan, Implement,
   Land]`; individual variants expand to singletons.
2. `resolve_prompt_path_prefers_env_var` — with `AGENTS_PROMPTS_DIR` set,
   prompt path resolves relative to it; without, falls back to `root`.
3. `resolve_prompt_path_errors_when_missing` — returns a clear error
   naming the missing file and the directories searched.
4. `workflow_timeout_reads_env` — with `AGENTS_WORKFLOW_TIMEOUT_SECS=0`
   or unset, returns `None`; with a positive integer, returns
   `Some(Duration)`.
5. `dry_run_plan_lists_phases_in_order` — `todo_workflow` with
   `dry_run=true` returns a structured plan (vec of resolved (phase,
   prompt_path)) without invoking any agent.

### Integration tests (`tests/todo_workflow_cli.rs`, new)

Use `CARGO_BIN_EXE_agents` and a `tempdir` repo with fixture
`prompt_0{1,2,3}.md` files. Override the agent binary via
`AGENTS_CLAUDE_BIN` (already supported, `src/lib.rs:45`) pointing at a
shell-script stub that echoes its stdin to a known path — lets us assert
the prompt was piped correctly and the CWD was the target root.

1. `todo_workflow_runs_three_phases_in_order` — stub writes each prompt
   it receives to `phase_N.txt`; assert files exist and contain the
   expected prompt bodies in order.
2. `todo_workflow_stops_on_phase_failure` — stub exits 1 for phase 2;
   assert phase 3's sentinel file is absent and exit code is non-zero
   and stderr names phase 2.
3. `todo_workflow_single_phase_flag` — `--phase land` only invokes the
   stub once with `prompt_03.md`'s body.
4. `todo_workflow_dry_run_prints_plan_and_skips_agent` — assert stdout
   lists all three phases with resolved prompt paths and stub was never
   invoked (no sentinel file).
5. `todo_workflow_missing_prompt_errors_cleanly` — remove `prompt_02.md`;
   assert exit non-zero, stderr names the missing file path, no partial
   execution of phase 1.
6. `todo_workflow_respects_prompts_dir_env` — put prompts in a sibling
   dir, set `AGENTS_PROMPTS_DIR`; assert stub receives correct content.

Edge cases deliberately covered:
- Missing prompt file (catches path-resolution regressions).
- Mid-pipeline failure (catches silent swallow of phase errors).
- Env var override (catches hardcoded path regressions).
- Dry-run (lets users verify plan without burning agent time — also the
  cheap smoke test for CI).

No quality/fidelity testing applies — output is a process exit code, not
a rendered artifact.

### Manual smoke (documented, not automated)

- `agents todo-workflow --dry-run` on this repo prints three phases.
- `agents todo-workflow --phase plan --cli claude` on a throwaway branch
  in a scratch repo produces `docs/plan/meta-plan/plan.md`.

### Presubmit

- `cargo test` must pass (the project's existing gate; `run.sh` is not
  present in this repo yet — verified by `ls`).
- `cargo clippy -- -D warnings` if configured; otherwise `cargo build`.

## Implementation steps

1. Add `Phase` enum + `Phases` parsing in `src/lib.rs`.
2. Add `resolve_prompt_path` + `workflow_timeout` helpers with unit tests.
3. Add `run_agent_interactive` helper mirroring `run_agent` variants.
4. Add `todo_workflow` entry point with `dry_run` support.
5. Wire `TodoWorkflow` variant in `src/main.rs`.
6. Write integration tests using a shell-stub for the agent binary.
7. Update `README.md` with a one-paragraph usage section and env-var table.
8. Run `cargo test` — all green.

## Rollout

- No feature flag; additive subcommand.
- No user-facing docs site — `README.md` is the user surface for this CLI.
  A short section there covers invocation, `--phase`, env vars, and a
  pointer to `prompt_0{1,2,3}.md` as the source of truth for behavior.

## Risks

- **Agent CLI streams differently across providers.** Mitigation:
  `Stdio::inherit()` delegates formatting to the agent; we don't parse.
- **Long runs with no timeout can hang forever on a wedged agent.**
  Mitigation: `AGENTS_WORKFLOW_TIMEOUT_SECS` escape hatch; user can
  ^C (the child inherits the TTY).
- **Codex `exec` one-shot shape doesn't match orchestration.** Mitigation:
  document that `--cli codex` is not recommended for workflow; consider
  a warning at invocation time (simple `eprintln!`).

## Out of scope (not deferred — explicitly excluded)

- Resuming mid-phase after a crash. Users re-run with `--phase <next>`.
- Parallelism across phases (inherently sequential).
- Parsing agent output to drive phase transitions (phases are fixed).

No work is deferred; everything needed for a working `todo-workflow` is
in the steps above.
