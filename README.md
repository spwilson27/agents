# Agents

A small Rust CLI to help with agent instruction-file workflows.

## Subcommands

### doc

Examples:

`cargo run -- doc`

Copies contents from `.agents/AGENT.md` into all supported agent instruction files at the project root:

- `CLAUDE.md`
- `AGENTS.md`
- `GEMINI.md`
- `.github/copilot-instructions.md`
- `QWEN.md`

### commit

Examples:

`EDITOR=vim cargo run -- commit --cli codex`

Reads `git diff --cached`, asks the selected agent CLI for a commit message draft, opens that draft in `$EDITOR`, and then runs `git commit --file <tempfile>` if the edited message is non-empty. If the edited message is empty, it prints `message empty, aborting commit`.




### todo-workflow

Runs a three-phase orchestration (plan, implement, land) using an agent CLI.
Each phase is a separate agent invocation driven by an embedded prompt.

Examples:

```
cargo run -- todo-workflow --dry-run
cargo run -- todo-workflow --cli claude
cargo run -- todo-workflow --cli claude --phase plan
cargo run -- todo-workflow --phase implement
```

Flags:

- `--cli` selects the agent CLI. Default `claude`. `codex` is discouraged
  because its `exec` is one-shot and ill-suited to long orchestration.
- `--phase` selects which phase(s) to run: `plan`, `implement`, `land`, or
  `all` (default). `all` expands to `plan`, `implement`, `land` in order.
- `--root` repository root (default `.`).
- `--dry-run` prints the resolved plan and exits without invoking the agent.

Environment variables:

- `AGENTS_WORKFLOW_TIMEOUT_SECS` — per-phase timeout in seconds. Unset (or
  `0`) means no timeout, which is the default: the orchestration can run for
  hours. The `commit` subcommand's 30-second timeout is unaffected.
- `AGENTS_<CLI>_BIN` (e.g. `AGENTS_CLAUDE_BIN`) — overrides the binary used
  for that agent CLI, same as for `commit`.

The per-phase prompts are embedded into the binary at build time via
`include_str!`. The source of truth lives under `prompts/todo-workflow/`:
`prompt_01.md` (plan), `prompt_02.md` (implement), `prompt_03.md` (land).
Edit those and rebuild to change behavior.

### TODOs

- mcp add/delete - Enable an mcp tool either globally or for the specific
  project across all agents.
