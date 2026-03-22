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




### TODOs

- mcp add/delete - Enable an mcp tool either globally or for the specific
  project across all agents.
