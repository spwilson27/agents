**Scope of this invocation: planning only.** Your deliverables are the plan, design docs, epics, and atomic task files described below — nothing else. Do NOT implement any TODO item, do NOT modify source code, do NOT run build or test commands against production code, do NOT open PRs, and do NOT mark TODO items DONE. Even if a TODO looks trivial enough to fix in a few lines, leave it alone — implementation happens in a later phase run by a separate orchestrator (see prompt_02.md). The only repo-state changes you may make are: writing files under `docs/plan/meta-plan/`, writing files under `docs/features/`, writing SVGs under `docs/features/assets/`, and marking items IN-PROGRESS in `TODO_INDEX.md`. If you catch yourself editing source files or writing code, stop — that is out of scope for this phase.

You are the meta-orchestrator for completing all remaining TODO items in this repo. Other agents are working in parallel from separate clones, so coordination via TODO_INDEX.md is critical.

Phase 0 — Ground yourself

1. Read AGENTS.md, TODO.md, and TODO_INDEX.md in full.
2. Survey the source tree for each open/in-progress item to confirm current state (don't trust TODO text blindly — verify against code).
3. In TODO_INDEX.md, mark every item you intend to own as IN-PROGRESS (meta-orchestrator YYYY-MM-DD). Do not delete or rewrite existing entries — parallel agents rely on them.

Phase 1 — Overarching plan


Remove the existing docs/plan/meta-plan directory if it exists. Write docs/plan/meta-plan/plan.md capturing:
- Scope: which TODO items are in/out and why
- Grouping rationale (which items become which epic)
- Sequencing, dependencies, and parallelism opportunities
- Cross-cutting testing/verification strategy (unit, regression tag, platform-check, e2e)
- Risks and hardware/upstream blockers

Phase 2 — Design docs (where needed)

For TODO items whose implementation approach isn't already pinned down:

**Parallelize.** Drafting all required design docs sequentially in one invocation is not viable and is the common failure mode for this phase. You MUST fan out: spawn one drafting subagent per design doc, running in parallel (single message, multiple tool calls). Then spawn one reviewer subagent per doc, also in parallel. Then you (the orchestrator) incorporate review feedback. Do not draft more than one doc yourself serially — if you find yourself doing that, stop and fan out instead. Budget for design-doc drafting is the parallel subagents, not your own context window.

1. Author docs/features/<feature>.md, grounded in the current source tree
   (cite file paths + line numbers). Every assumption in the doc must be
   vetted against the current code before it is written — if you cannot
   verify a claim by reading the source, either read more source or drop
   the claim. No speculative "I think this is how X works."
2. Each design doc must capture the full picture, not a sketch:
   - **Problem & scope**: what the feature is, which TODO items it covers,
     explicit non-goals.
   - **Technical implementation**: concrete approach — data structures,
     control flow, module layout, new/modified files. Cite current-state
     file paths + line numbers for every touchpoint.
   - **Impacted APIs**: every public surface affected (frontend contracts,
     Cap'n Proto schemas, CLI flags, config files, HTTP/RPC endpoints)
     AND every non-trivial internal API (trait/interface changes, shared
     crate exports). Show before/after signatures.
   - **UI / API mockups**: for user-facing or API-shape changes, include
     mockups. Any SVG goes in its own file under docs/features/assets/
     and is referenced from the doc — never inline SVG in markdown.
   - **Configuration**: new settings, defaults, migration/back-compat
     story.
   - **Performance**: expected cost (CPU, memory, latency, disk), any
     benchmarks required, regression thresholds.
   - **Validation plan**:
     * Unit test coverage — what invariants and which modules.
     * For frontend features, gooey playwright-style e2e tests covering
       both the happy path AND edge cases designed to catch likely bugs
       (empty state, error state, slow network, rapid re-entry, invalid
       input, accessibility).
     * Quality/fidelity validation — for features where output quality
       matters (rendering, audio, model output, diff formatting, etc.),
       specify exactly how fidelity is measured: golden files,
       perceptual diff thresholds, reference recordings, reviewer
       rubrics. Do not hand-wave "looks right."
     * Platform-check / regression-tag hooks where applicable.
   - **Trade-offs & definitive decisions**: list the real alternatives
     considered, state the decision, and give the reason. No "we could
     do A or B" left open — pick one.
   - **Rollout & observability**: feature flags, metrics, logging.
   - **User-facing documentation**: if the feature is user-visible,
     include a docs/users/<feature>.md in the same change (or list it as
     a required artifact); reference it from the design doc. Skip only
     for purely internal changes.
3. A design doc must not defer work. Anything out of scope must be split
   into its own docs/features/<other>.md and referenced from this doc
   with a clear boundary — never a hand-wave "to be designed later."
4. After each doc is drafted, spawn a separate reviewer subagent per doc
   to critique it against this checklist and the current source tree.
   Address every concern (revise the doc or document the rejection with
   rationale) before moving on. Reviewer concerns are not optional.
5. Skip items that already have a landed design doc — link to it from
   the plan instead. If an existing doc is missing sections required
   above, extend it rather than starting over.

Phase 3 — Epics

For each group identified in the overarching plan, write docs/plan/meta-plan/epics/<epic>.md covering:
- Which TODO §-numbers it subsumes
- Implementation strategy and key files
- Test/verification plan
- Dependencies on other epics

Phase 4 — Atomic tasks

Decompose each epic into docs/plan/meta-plan/epics/<epic>/task_NNN.md files. Each task must be:
- Atomic: one agent, one PR-sized change
- Self-contained: implementation hints, exact file paths, acceptance criteria, and an automated validation command (bazel test ... or ./run.sh ...)
- Independently verifiable: no human-in-the-loop checks

Stop points

After finishing Phases 0–2 (ground + overarching plan + reviewed design docs),
consult an opus subagent to review open decisions. Select the best options and
then proceed to breakdown epics and tasks.

**You are expected to complete Phases 0–4 in this single invocation.** Stopping after Phase 1 with a "this is too big, continue in a follow-on run" handoff is not acceptable — the orchestrator pattern for this phase is parallel subagents, not serial self-drafting. If volume feels infeasible, that is a signal to fan out harder (more parallel subagents per doc / per epic / per task batch), not to stop. Phases 3 and 4 are likewise parallelizable: spawn one subagent per epic for Phase 3, and batch task-file generation across parallel subagents for Phase 4.

Rules

- Never delete TODO entries; only mark in-progress.
- Follow repo conventions in CLAUDE.md (no direct cargo, token-only styling, regression-test-first for bug fixes, etc.).
- Hardware-blocked items (e.g. §31 RTX 4090) stay deferred — document, don't attempt.
- Prefer editing existing docs over creating new ones where overlap exists.


