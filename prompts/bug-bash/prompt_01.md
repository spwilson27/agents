**Scope of this invocation: bug discovery only.** Your deliverables are the bug registry at `docs/plan/bug-bash.md` — nothing else. Do NOT write tests, do NOT modify source code, do NOT attempt fixes, do NOT open PRs. Reproduction happens in prompt_02.md, fixes happen in prompt_03.md, landing happens in prompt_04.md. The only repo-state changes you may make are writing `docs/plan/bug-bash.md` (and any supporting notes it references). If you catch yourself editing source files or writing tests, stop — that is out of scope for this phase.

You are the bug-bash discovery orchestrator. Other agents may be operating in parallel from separate clones; the bug registry at `docs/plan/bug-bash.md` is the coordination artifact downstream phases consume.

Phase 0 — Ground yourself

1. Read AGENTS.md, CLAUDE.md, README.md, and any docs/ entries that describe the system architecture.
2. Survey the source tree: identify every module, service, and public surface (CLI flags, HTTP/RPC endpoints, config files, schemas, trait contracts).
3. If `docs/plan/bug-bash.md` already exists, delete it. Start from a clean registry.

Phase 1 — Dispatch investigators

Spawn investigator subagents in parallel (single message, multiple Agent calls, default concurrency cap 4). Partition the codebase so every module is covered by at least one investigator. Each subagent's brief must include:

- The exact subtree / module boundary it owns
- A standing instruction to read-only: no edits, no test runs that mutate state
- The bug classes to hunt for, explicitly:
  * Logic errors (off-by-one, wrong operator, inverted condition, incorrect state transition)
  * Error-handling gaps (swallowed errors, `unwrap`/`expect` on fallible values, missing `?` propagation)
  * Concurrency hazards (races, deadlocks, non-atomic read-modify-write, missing locks)
  * Resource leaks (unclosed files/sockets, unbounded growth, missing cleanup)
  * Input validation failures (injection, path traversal, integer overflow, missing bounds checks)
  * API contract violations (functions that silently break their documented invariants)
  * Dead/unreachable code masking real bugs
  * Stale comments/docs contradicting the code
  * Cross-module contract drift (caller assumes X, callee provides Y)
- Required return payload: a list of candidate bugs, each with `file:line` citation, one-paragraph description, suspected impact, suggested severity (high/medium/low), and a concrete reproduction hypothesis (what input or state triggers it).

Phase 2 — Triage and dedupe

1. Collect every candidate returned by every investigator.
2. Dedupe: collapse duplicates; merge overlapping reports into a single entry that cites all observers.
3. Triage severity. Use these calibrations:
   - **high**: data loss, memory safety, security, silent incorrect results, crashes on realistic input, broken invariants in core paths
   - **medium**: recoverable crashes, incorrect results on edge cases, observable but non-fatal contract violations
   - **low**: cosmetic, docs drift, dead code with no active caller
4. If you have fewer than 100 total candidates or fewer than 25 `high`, dispatch another round of investigators with a brief that names the under-covered modules or bug classes. Repeat until thresholds are met. Do not pad the registry — every entry must be grounded in a specific `file:line` citation. If after a second round the codebase genuinely does not contain 100+ distinct defects, record that finding at the top of the registry with evidence and proceed with what you have.

Phase 3 — Write the registry

Write `docs/plan/bug-bash.md` with this structure:

```
# Bug Bash Registry

Generated: <UTC timestamp>
Total: <N> bugs (<H> high, <M> medium, <L> low)

## BUG-001 — <short title>
- Severity: high
- Location: <file:line> (and additional citations if observed in multiple places)
- Description: <one paragraph>
- Reproduction hypothesis: <what input / state triggers it>
- Suggested regression test: <which test file, which invariant to assert>

## BUG-002 — ...
```

Rules for the registry:
- IDs are monotonic `BUG-NNN`, zero-padded to three digits, never reused.
- Every entry must name a concrete reproduction hypothesis — prompt_02 consumes this to author a failing test. "Seems wrong" is not a hypothesis.
- Sort entries by severity (high first), then by ID.
- Do not mark entries fixed, do not delete entries, do not add a status column — lifecycle tracking happens in later phases.

Phase 4 — Self-review

Spawn a reviewer subagent with the completed registry and a sample of ~10 random entries' source locations. Brief it to verify: (a) each sampled `file:line` exists and says what the entry claims, (b) severity calibration matches the guidance above, (c) reproduction hypotheses are concrete enough for a test author to act on. Address every concern before exiting.

Rules

- You orchestrate; investigators discover. Do not read source yourself except to adjudicate conflicting reports.
- Do not write tests, do not edit source, do not run destructive commands.
- Never fabricate bugs to hit the 100/25 threshold. Citations must be real.
- Resolve ambiguity by picking the best-supported option and noting the choice inline in the affected registry entry.
