# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

- Build: `cargo build`
- Run CLI: `cargo run -- <subcommand>` (subcommands: `doc`, `commit`, `todo-workflow`, `pipeclean`, `bug-bash`)
- Tests: `cargo test` (run a single integration test file with `cargo test --test commit_cli`, or a single case with `cargo test --test commit_cli -- <name>`)

## Architecture

Single Rust binary (`agents`) that orchestrates other agent CLIs (claude, codex, gemini, copilot, qwen). `src/main.rs` is a thin clap dispatcher; all logic lives in `src/lib.rs` (one large module, ~1200 lines) so integration tests under `tests/` can drive it end-to-end.

Key concepts:

- **Agent instruction fan-out (`doc`)**: copies `.agents/AGENT.md` into per-agent instruction files listed in the `TARGETS` table at the top of `lib.rs`. To add a new agent target, extend that slice.
- **Phase-based orchestrations**: `todo-workflow` (plan/implement/land), `pipeclean` (fix/review), `bug-bash` (hunt/reproduce/fix/land) all follow the same pattern: a `Phase` enum with an `All` variant that expands into ordered sub-phases, each phase mapped to a prompt file via `include_str!`. Prompts are embedded at build time — edit the source in `prompts/<workflow>/prompt_NN.md` and rebuild.
- **Agent CLI invocation**: each phase shells out to the selected agent CLI interactively. `codex` is discouraged for multi-phase workflows (one-shot `exec`). The `commit` subcommand pipes `git diff --cached` through an agent CLI, opens the draft in `$EDITOR`, then runs `git commit --file`.
- **Timeouts**: `commit` uses a fixed 30s timeout (`DEFAULT_AGENT_TIMEOUT`). Workflow phases honor `AGENTS_WORKFLOW_TIMEOUT_SECS` (unset/`0` = no timeout). The binary used per agent CLI can be overridden via `AGENTS_<CLI>_BIN` (e.g. `AGENTS_CLAUDE_BIN`) — tests rely on this to inject fake binaries.
- **`--dry-run`**: workflow commands print the resolved plan (phases, prompts, binary) and exit without spawning the agent; integration tests assert against this output.

## User preferences

Workflow phases should launch the agent CLI **interactively** with the prompt, not in headless mode.
