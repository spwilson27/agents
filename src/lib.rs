use std::collections::{HashSet, VecDeque};
use std::env;
use std::error::Error as StdError;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use clap::ValueEnum;
use serde_json::Value;
use tempfile::NamedTempFile;
use wait_timeout::ChildExt;

pub const SOURCE_FILE: &str = ".agents/AGENT.md";
const DEFAULT_AGENT_TIMEOUT: Duration = Duration::from_secs(30);
const PROMPTS_DIR: &str = "~/.config/agents/prompts";
pub const TARGETS: &[(&str, &str)] = &[
    ("claude", "CLAUDE.md"),
    ("codex", "AGENTS.md"),
    ("gemini", "GEMINI.md"),
    ("copilot", ".github/copilot-instructions.md"),
    ("qwen", "QWEN.md"),
];

const PROMPT_PLAN: &str = include_str!("../prompts/todo-workflow/prompt_01.md");
const PROMPT_IMPLEMENT: &str = include_str!("../prompts/todo-workflow/prompt_02.md");
const PROMPT_LAND: &str = include_str!("../prompts/todo-workflow/prompt_03.md");
const PROMPT_REVIEW: &str = include_str!("../prompts/todo-workflow/prompt_04.md");

const PROMPT_PIPECLEAN_FIX: &str = include_str!("../prompts/pipeclean/prompt_01.md");
const PROMPT_PIPECLEAN_REVIEW: &str = include_str!("../prompts/pipeclean/prompt_02.md");

const FINAL_REVIEW_PROMPT: &str = include_str!("../prompts/final_review.md");

const BUG_BASH_REPRODUCE: &str = include_str!("../prompts/bug-bash/prompt_02.md");
const BUG_BASH_FIX: &str = include_str!("../prompts/bug-bash/prompt_03.md");
const BUG_BASH_LAND: &str = include_str!("../prompts/bug-bash/prompt_04.md");
const BUG_SEARCH_TMP_PROMPT: &str = ".bug-hunt-prompt.tmp.md";
const BUG_SEARCH_OUTPUT_ROOT: &str = "docs/bugs";
const BUG_SEARCH_STATE: &str = "docs/bugs/search-state.json";
const BUG_REPRODUCE_STATE: &str = "docs/bugs/reproduce-state.json";
const BUG_SEARCH_PROMPT_TEMPLATE: &str = r#"# Bug-hunt - single file: `{rel_src}`

**Scope of this invocation: bug discovery only.** Your single deliverable is
the bug registry at `{rel_out}` - nothing else. Do NOT write tests, do NOT
modify source code, do NOT attempt fixes, do NOT open PRs. The only repo-state
change you may make is writing `{rel_tmp_out}` and atomically renaming it to
`{rel_out}`. If you catch yourself editing source files or writing tests, stop -
that is out of scope for this phase.

**Partition for this run:** the single source file `{rel_src}`. Read that file
in full. You may read other files in the repo *only* to disambiguate types,
trait impls, callers, or invariants referenced from `{rel_src}`. Do not file
bugs against any other file - only defects whose root cause lives in
`{rel_src}`.

## What counts as a bug

A bug is **incorrect runtime behaviour in production code** - something that
produces wrong results, crashes, corrupts data, leaks resources, or violates
memory safety under a realistic scenario.

### DO file

- Logic errors: off-by-one, wrong operator, inverted condition, incorrect
  state transition.
- Error-handling gaps: swallowed errors, `unwrap()` / `expect()` on fallible
  values in non-test code where the value can realistically be `Err`/`None`,
  missing `?` propagation.
- Concurrency hazards: data races, deadlocks, non-atomic read-modify-write,
  missing locks, lock-order inversions, blocking mutex inside async.
- Resource leaks: unclosed files / sockets / handles, unbounded growth,
  missing cleanup, FD leaks across exec.
- Input validation gaps **at system boundaries** (user input, IPC, file I/O,
  network): integer overflow, missing bounds checks, path traversal,
  injection, NaN/Inf propagation.
- Memory safety: `unsafe` blocks with broken invariants, raw-pointer lifetime
  hazards, `from_raw_parts(_mut)` with wrong length, `Box::from_raw` without
  validation, panic across C ABI.
- API contract violations: function silently breaks its documented invariant.
- Real-time safety: heap allocation or syscall on a thread documented as RT.
- Cross-module contract drift: this file assumes X, callee actually does Y.

### Do NOT file

- **Test quality issues.** "This test doesn't cover enough" or "this test
  exercises stdlib instead of application code" is review feedback, not a bug.
  Only file a bug against test code if the test itself will *pass when the
  code under test is broken* due to an error **in the test's own logic**
  (e.g., assert on wrong variable, tautological assertion, setup that
  accidentally masks the condition being tested).
- **Missing hardening.** "This function doesn't validate X" is not a bug
  unless you can show a realistic caller that passes invalid X. Internal
  functions trusting their callers is normal.
- **Theoretical edge cases with no realistic trigger.** If the only way to
  trigger the issue is adversarial/contrived input that the system never
  receives, do not file it. The reproduction hypothesis must involve a
  scenario that could plausibly occur in production.
- **Style, cosmetic, or "could be cleaner" observations.** Not bugs.
- **Stale comments or docs drift.** These are low-value and noisy.
- **Suggestions for additional error handling, logging, or validation** beyond
  what is needed for correctness.

## Output format

Write `{rel_tmp_out}` with this structure (Markdown), then rename it to
`{rel_out}` only after the file is complete and valid Markdown:

```
# Bug Bash Registry - {rel_src}

Generated: <UTC timestamp>
Source file: {rel_src}
Total: <N> bugs (<H> high, <M> medium, <L> low)

## BUG-001 - <short title>
- Severity: high | medium | low
- Location: {rel_src}:<line> (additional citations if observed in multiple sites in the same file)
- Description: <one paragraph>
- Reproduction hypothesis: <a concrete, realistic scenario - not a contrived adversarial input>
- Suggested regression test: <which test file, which invariant to assert>

## BUG-002 - ...
```

Severity calibration:
  - high: data loss, memory safety, security, silent incorrect results, or
    crashes - AND the trigger is a realistic production scenario, not a
    contrived input. If you cannot articulate a plausible real-world trigger,
    it is not high severity.
  - medium: recoverable crashes or incorrect results on uncommon but realistic
    edge cases, observable but non-fatal contract violations.
  - low: minor correctness issues with narrow impact that are unlikely to
    manifest in practice.

Rules:
  - Every entry MUST cite a real `{rel_src}:<line>` and accurately describe
    what is at that line. No fabricated findings.
  - Every entry MUST give a concrete, realistic reproduction hypothesis -
    "seems wrong" is not a hypothesis, and "pass NaN/Inf to trigger" is not
    realistic unless the function receives external input.
  - Prefer fewer, high-confidence findings over a long list of maybes. If you
    are not reasonably sure something is a genuine bug, do not file it.
  - If you find no defects, write the registry with `Total: 0 bugs` and a
    one-line note explaining what you reviewed. Do not pad.
  - Sort entries by severity (high first), then by line number.
  - Create the parent directory of `{rel_out}` if it does not exist.
  - Do not leave partial `{rel_out}` files behind. Write `{rel_tmp_out}` first,
    then rename it to `{rel_out}`.

When you have written `{rel_out}` and verified it parses as Markdown, exit.
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Phase {
    Plan,
    Implement,
    Land,
    Review,
    All,
}

impl Phase {
    pub fn expand(self) -> Vec<Phase> {
        match self {
            Self::All => vec![Self::Plan, Self::Implement, Self::Land, Self::Review],
            single => vec![single],
        }
    }

    pub(crate) fn prompt_body(self) -> &'static str {
        match self {
            Self::Plan => PROMPT_PLAN,
            Self::Implement => PROMPT_IMPLEMENT,
            Self::Land => PROMPT_LAND,
            Self::Review => PROMPT_REVIEW,
            Self::All => unreachable!("Phase::All must be expanded before prompt lookup"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Implement => "implement",
            Self::Land => "land",
            Self::Review => "review",
            Self::All => "all",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum PipeCleanPhase {
    Fix,
    Review,
    All,
}

impl PipeCleanPhase {
    pub fn expand(self) -> Vec<PipeCleanPhase> {
        match self {
            Self::All => vec![Self::Fix, Self::Review],
            single => vec![single],
        }
    }

    pub(crate) fn prompt_body(self) -> &'static str {
        match self {
            Self::Fix => PROMPT_PIPECLEAN_FIX,
            Self::Review => PROMPT_PIPECLEAN_REVIEW,
            Self::All => unreachable!("PipeCleanPhase::All must be expanded before prompt lookup"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Fix => "fix",
            Self::Review => "review",
            Self::All => "all",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum BugBashPhase {
    #[value(alias = "hunt")]
    Search,
    Reproduce,
    Fix,
    Land,
    All,
}

impl BugBashPhase {
    pub fn expand(self) -> Vec<BugBashPhase> {
        match self {
            Self::All => vec![Self::Search, Self::Reproduce],
            single => vec![single],
        }
    }

    pub(crate) fn prompt_body(self) -> &'static str {
        match self {
            Self::Search => BUG_SEARCH_PROMPT_TEMPLATE,
            Self::Reproduce => BUG_BASH_REPRODUCE,
            Self::Fix => BUG_BASH_FIX,
            Self::Land => BUG_BASH_LAND,
            Self::All => unreachable!("BugBashPhase::All must be expanded before prompt lookup"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Reproduce => "reproduce",
            Self::Fix => "fix",
            Self::Land => "land",
            Self::All => "all",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum AgentCli {
    Qwen,
    Gemini,
    Claude,
    Codex,
    Copilot,
}

impl AgentCli {
    fn binary_name(self) -> &'static str {
        match self {
            Self::Qwen => "qwen",
            Self::Gemini => "gemini",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
        }
    }

    fn env_var_name(self) -> &'static str {
        match self {
            Self::Qwen => "AGENTS_QWEN_BIN",
            Self::Gemini => "AGENTS_GEMINI_BIN",
            Self::Claude => "AGENTS_CLAUDE_BIN",
            Self::Codex => "AGENTS_CODEX_BIN",
            Self::Copilot => "AGENTS_COPILOT_BIN",
        }
    }

    fn command(self) -> Command {
        if let Some(path) = env::var_os(self.env_var_name()) {
            Command::new(path)
        } else {
            Command::new(self.binary_name())
        }
    }
}

#[derive(Debug)]
pub enum AgentsError {
    Io(io::Error),
    MissingEditor,
    NothingStaged,
    TimedOut {
        program: String,
        timeout: Duration,
    },
    CommandFailed {
        program: String,
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
}

impl std::fmt::Display for AgentsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::MissingEditor => write!(f, "$EDITOR is not set"),
            Self::NothingStaged => write!(f, "nothing staged"),
            Self::TimedOut { program, timeout } => {
                write!(f, "{program} timed out after {}s", timeout.as_secs())
            }
            Self::CommandFailed {
                program,
                status,
                stdout,
                stderr,
            } => {
                if !stderr.trim().is_empty() {
                    write!(
                        f,
                        "{program} exited with status {status}: {}",
                        stderr.trim()
                    )
                } else if !stdout.trim().is_empty() {
                    write!(
                        f,
                        "{program} exited with status {status}: {}",
                        stdout.trim()
                    )
                } else {
                    write!(f, "{program} exited with status {status}")
                }
            }
        }
    }
}

impl StdError for AgentsError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::MissingEditor
            | Self::NothingStaged
            | Self::TimedOut { .. }
            | Self::CommandFailed { .. } => None,
        }
    }
}

impl From<io::Error> for AgentsError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitOutcome {
    Committed,
    AbortedEmptyMessage,
}

pub fn doc(root: &Path) -> Result<Vec<PathBuf>, io::Error> {
    let source = root.join(SOURCE_FILE);
    if !source.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("error: {SOURCE_FILE} not found"),
        ));
    }

    let content = fs::read_to_string(&source)?;
    let mut seen_paths = HashSet::new();
    let mut written = Vec::new();

    for (_, rel) in TARGETS {
        let dest = root.join(rel);
        if !seen_paths.insert(dest.clone()) {
            continue;
        }

        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&dest, &content)?;
        written.push(dest);
    }

    Ok(written)
}

pub fn save_prompt(file: &Path, name: Option<&str>, force: bool) -> Result<(), Box<dyn StdError>> {
    // Read the source file
    let content = fs::read_to_string(file).map_err(|e| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("error: cannot read file '{}': {}", file.display(), e),
        )
    })?;

    // Determine the prompt name
    let prompt_name = name.map(|s| s.to_string()).unwrap_or_else(|| {
        file.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("prompt")
            .to_string()
    });

    // Expand the prompts directory path
    let prompts_dir = expand_home(PROMPTS_DIR)?;

    // Create the prompts directory if it doesn't exist
    fs::create_dir_all(&prompts_dir)?;

    let dest_path = prompts_dir.join(format!("{prompt_name}.md"));

    // Check if file exists and handle confirmation
    if dest_path.exists() && !force {
        eprint!(
            "Prompt '{}' already exists. Overwrite? [y/N]: ",
            prompt_name
        );
        io::stderr().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "operation cancelled by user".to_string(),
            )
            .into());
        }
    }

    // Write the prompt file
    fs::write(&dest_path, &content)?;
    println!("Saved prompt '{}' to {}", prompt_name, dest_path.display());

    Ok(())
}

pub fn prompt(name: Option<&str>, list: bool) -> Result<(), Box<dyn StdError>> {
    let prompts_dir = expand_home(PROMPTS_DIR)?;

    if list {
        // List all available prompts
        if !prompts_dir.exists() {
            println!("No prompts saved yet.");
            return Ok(());
        }

        let mut prompts: Vec<_> = fs::read_dir(&prompts_dir)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().map_or(false, |ext| ext == "md"))
            .filter_map(|entry| {
                entry
                    .path()
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            })
            .collect();

        prompts.sort();

        if prompts.is_empty() {
            println!("No prompts saved yet.");
        } else {
            for prompt_name in prompts {
                println!("{}", prompt_name);
            }
        }
    } else {
        // Print a specific prompt
        let name = name.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "error: must provide prompt name or use --list",
            )
        })?;

        let prompt_path = prompts_dir.join(format!("{name}.md"));

        if !prompt_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("error: prompt '{}' not found", name),
            )
            .into());
        }

        let content = fs::read_to_string(&prompt_path)?;
        print!("{}", content);
    }

    Ok(())
}

fn expand_home(path: &str) -> Result<PathBuf, io::Error> {
    if path.starts_with("~/") {
        let home = env::var("HOME")
            .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "$HOME not set"))?;
        Ok(PathBuf::from(home).join(&path[2..]))
    } else {
        Ok(PathBuf::from(path))
    }
}

pub fn commit(root: &Path, cli: AgentCli) -> Result<CommitOutcome, AgentsError> {
    eprintln!("collecting staged diff...");
    let diff = run_text_command(
        Command::new("git")
            .current_dir(root)
            .args(["diff", "--cached"]),
        None,
    )?;
    if diff.trim().is_empty() {
        return Err(AgentsError::NothingStaged);
    }
    let file_context = build_file_context(root)?;

    let prompt = build_commit_prompt(&diff, &file_context.context);
    eprintln!(
        "asking {} for a commit message ({}s timeout)...",
        cli.binary_name(),
        agent_timeout().as_secs()
    );
    let initial_message = run_agent(cli, root, &prompt)?;
    let initial_message = add_status_comment(&initial_message, &file_context.status_comment);
    eprintln!("opening $EDITOR; save and quit to continue...");
    let edited_message = edit_message(&initial_message)?;

    if edited_message.trim().is_empty() {
        println!("message empty, aborting commit");
        return Ok(CommitOutcome::AbortedEmptyMessage);
    }

    let mut message_file = NamedTempFile::new()?;
    message_file.write_all(edited_message.as_bytes())?;
    message_file.flush()?;

    run_status_command(
        Command::new("git")
            .current_dir(root)
            .args(["commit", "--file"])
            .arg(message_file.path()),
    )?;
    eprintln!("git commit completed.");

    Ok(CommitOutcome::Committed)
}

pub struct WorkflowPlanEntry {
    pub phase: Phase,
}

pub fn todo_workflow(
    root: &Path,
    cli: AgentCli,
    phase: Phase,
    dry_run: bool,
) -> Result<Vec<WorkflowPlanEntry>, AgentsError> {
    let expanded: Vec<Phase> = phase.expand();

    let plan: Vec<WorkflowPlanEntry> = expanded
        .iter()
        .map(|p| WorkflowPlanEntry { phase: *p })
        .collect();

    if matches!(cli, AgentCli::Codex) {
        eprintln!(
            "warning: --cli codex uses one-shot exec; claude is recommended for todo-workflow"
        );
    }

    if dry_run {
        println!("todo-workflow plan ({} phase(s)):", plan.len());
        for (idx, entry) in plan.iter().enumerate() {
            println!("  {}. {} (embedded)", idx + 1, entry.phase.label());
        }
        println!("cli: {}", cli.binary_name());
        println!("root: {}", root.display());
        for (idx, entry) in plan.iter().enumerate() {
            println!(
                "\n--- prompt {} ({}) ---\n{}",
                idx + 1,
                entry.phase.label(),
                entry.phase.prompt_body()
            );
        }
        return Ok(plan);
    }

    let timeout = workflow_timeout();
    for (idx, entry) in plan.iter().enumerate() {
        eprintln!("=== Phase {}: {} ===", idx + 1, entry.phase.label());
        let prompt = entry.phase.prompt_body();
        if let Err(err) = run_agent_interactive(cli, root, prompt, timeout) {
            eprintln!(
                "phase {} ({}) failed; resume with --phase {}",
                idx + 1,
                entry.phase.label(),
                entry.phase.label()
            );
            return Err(err);
        }
    }

    Ok(plan)
}

pub struct PipeCleanPlanEntry {
    pub phase: PipeCleanPhase,
}

pub fn pipeclean(
    root: &Path,
    cli: AgentCli,
    phase: PipeCleanPhase,
    dry_run: bool,
) -> Result<Vec<PipeCleanPlanEntry>, AgentsError> {
    let expanded: Vec<PipeCleanPhase> = phase.expand();

    let plan: Vec<PipeCleanPlanEntry> = expanded
        .iter()
        .map(|p| PipeCleanPlanEntry { phase: *p })
        .collect();

    if matches!(cli, AgentCli::Codex) {
        eprintln!("warning: --cli codex uses one-shot exec; claude is recommended for pipeclean");
    }

    if dry_run {
        println!("pipeclean plan ({} phase(s)):", plan.len());
        for (idx, entry) in plan.iter().enumerate() {
            println!("  {}. {} (embedded)", idx + 1, entry.phase.label());
        }
        println!("cli: {}", cli.binary_name());
        println!("root: {}", root.display());
        for (idx, entry) in plan.iter().enumerate() {
            println!(
                "\n--- prompt {} ({}) ---\n{}",
                idx + 1,
                entry.phase.label(),
                entry.phase.prompt_body()
            );
        }
        return Ok(plan);
    }

    let timeout = workflow_timeout();
    for (idx, entry) in plan.iter().enumerate() {
        eprintln!("=== Phase {}: {} ===", idx + 1, entry.phase.label());
        let prompt = entry.phase.prompt_body();
        if let Err(err) = run_agent_interactive(cli, root, prompt, timeout) {
            eprintln!(
                "phase {} ({}) failed; resume with --phase {}",
                idx + 1,
                entry.phase.label(),
                entry.phase.label()
            );
            return Err(err);
        }
    }

    Ok(plan)
}

pub fn final_review(root: &Path, cli: AgentCli, dry_run: bool) -> Result<(), AgentsError> {
    if matches!(cli, AgentCli::Codex) {
        eprintln!(
            "warning: --cli codex uses one-shot exec; claude is recommended for final-review"
        );
    }

    if dry_run {
        println!("final-review plan (1 phase):");
        println!("  1. final-review (embedded)");
        println!("cli: {}", cli.binary_name());
        println!("root: {}", root.display());
        println!("\n--- prompt 1 (final-review) ---\n{FINAL_REVIEW_PROMPT}");
        return Ok(());
    }

    let timeout = workflow_timeout();
    eprintln!("=== Phase 1: final-review ===");
    run_agent_interactive(cli, root, FINAL_REVIEW_PROMPT, timeout)
}

pub struct BugBashPlanEntry {
    pub phase: BugBashPhase,
}

#[derive(Clone, Debug)]
pub struct BugSearchConfig {
    pub source_root: PathBuf,
    pub force: bool,
    pub dry_run: bool,
    pub limit: Option<usize>,
    pub start_at: Option<PathBuf>,
    pub jobs: usize,
    pub restart: bool,
}

impl BugSearchConfig {
    pub fn new(dry_run: bool) -> Self {
        Self {
            source_root: PathBuf::from("src"),
            force: false,
            dry_run,
            limit: None,
            start_at: None,
            jobs: 1,
            restart: false,
        }
    }
}

pub fn bug_bash(
    root: &Path,
    cli: AgentCli,
    phase: BugBashPhase,
    dry_run: bool,
) -> Result<Vec<BugBashPlanEntry>, AgentsError> {
    bug_bash_with_search_config(root, cli, phase, BugSearchConfig::new(dry_run))
}

pub fn bug_bash_with_search_config(
    root: &Path,
    cli: AgentCli,
    phase: BugBashPhase,
    search_config: BugSearchConfig,
) -> Result<Vec<BugBashPlanEntry>, AgentsError> {
    let expanded: Vec<BugBashPhase> = phase.expand();

    let plan: Vec<BugBashPlanEntry> = expanded
        .iter()
        .map(|p| BugBashPlanEntry { phase: *p })
        .collect();

    if matches!(cli, AgentCli::Codex) {
        eprintln!("warning: --cli codex uses one-shot exec; claude is recommended for bug-bash");
    }

    if search_config.dry_run {
        println!("bug-bash plan ({} phase(s)):", plan.len());
        for (idx, entry) in plan.iter().enumerate() {
            let source = if matches!(entry.phase, BugBashPhase::Search) {
                "per-file"
            } else {
                "embedded"
            };
            println!("  {}. {} ({source})", idx + 1, entry.phase.label());
        }
        println!("cli: {}", cli.binary_name());
        println!("root: {}", root.display());
        for (idx, entry) in plan.iter().enumerate() {
            if matches!(entry.phase, BugBashPhase::Search) {
                println!("\n--- phase {} (search) ---", idx + 1);
                bug_search(root, cli, &search_config)?;
            } else {
                let prompt = bug_bash_phase_prompt(entry.phase, &search_config);
                println!(
                    "\n--- prompt {} ({}) ---\n{}",
                    idx + 1,
                    entry.phase.label(),
                    prompt
                );
            }
        }
        return Ok(plan);
    }

    let timeout = workflow_timeout();
    for (idx, entry) in plan.iter().enumerate() {
        eprintln!("=== Phase {}: {} ===", idx + 1, entry.phase.label());
        let result = if matches!(entry.phase, BugBashPhase::Search) {
            bug_search(root, cli, &search_config)
        } else {
            let prompt = bug_bash_phase_prompt(entry.phase, &search_config);
            run_agent_interactive(cli, root, &prompt, timeout)
        };
        if let Err(err) = result {
            eprintln!(
                "phase {} ({}) failed; resume with --phase {}",
                idx + 1,
                entry.phase.label(),
                entry.phase.label()
            );
            return Err(err);
        }
    }

    Ok(plan)
}

fn bug_bash_phase_prompt(phase: BugBashPhase, config: &BugSearchConfig) -> String {
    match phase {
        BugBashPhase::Reproduce => render_bug_reproduce_prompt(config),
        other => other.prompt_body().to_owned(),
    }
}

fn bug_search(root: &Path, cli: AgentCli, config: &BugSearchConfig) -> Result<(), AgentsError> {
    if config.jobs < 1 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "--jobs must be >= 1").into());
    }

    let source_root = root.join(&config.source_root);
    if !source_root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a directory", source_root.display()),
        )
        .into());
    }

    let mut files = find_bug_search_files(&source_root)?;
    if let Some(start_at) = &config.start_at {
        let target = root.join(start_at);
        files.retain(|path| path >= &target);
    }
    if let Some(limit) = config.limit {
        files.truncate(limit);
    }

    if files.is_empty() {
        println!("no .rs files matched");
        return Ok(());
    }

    if !config.dry_run {
        write_bug_search_state(root, config, &files)?;
    }

    let source_label = config
        .source_root
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    println!("found {} rust files under {source_label}", files.len());
    println!("output root: {BUG_SEARCH_OUTPUT_ROOT}");
    if config.jobs == 1 {
        println!("tmp prompt:  {BUG_SEARCH_TMP_PROMPT}");
    } else {
        println!(
            "parallel jobs: {} (per-task temp prompts under {BUG_SEARCH_TMP_PROMPT}.*)",
            config.jobs
        );
    }

    let mut skipped = 0usize;
    let mut work = Vec::new();
    for (idx, src) in files.iter().enumerate() {
        let out = bug_search_output_path(root, src);
        let rel_src = rel_display(root, src);
        if out.exists() && !config.force {
            println!("[{}/{}] skip (exists): {rel_src}", idx + 1, files.len());
            skipped += 1;
            continue;
        }
        work.push((idx + 1, src.clone(), out));
    }

    let (done, failed, first_error) = if config.jobs == 1 {
        run_bug_search_sequential(root, cli, config, files.len(), &work)?
    } else {
        run_bug_search_parallel(root, cli, config, files.len(), work)?
    };

    let tmp_prompt = root.join(BUG_SEARCH_TMP_PROMPT);
    if tmp_prompt.exists() && !config.dry_run {
        let _ = fs::remove_file(tmp_prompt);
    }

    println!("\ndone: {done}  skipped: {skipped}  failed: {failed}");
    if let Some(err) = first_error {
        Err(err)
    } else {
        Ok(())
    }
}

fn run_bug_search_sequential(
    root: &Path,
    cli: AgentCli,
    config: &BugSearchConfig,
    total_files: usize,
    work: &[(usize, PathBuf, PathBuf)],
) -> Result<(usize, usize, Option<AgentsError>), AgentsError> {
    let mut done = 0usize;
    let mut failed = 0usize;
    let mut first_error = None;

    for (idx, src, out) in work {
        match bug_search_one(root, cli, config, total_files, *idx, src, out) {
            Ok(()) => done += 1,
            Err(err) => {
                eprintln!("  [{idx}] {} exited with error: {err}", cli.binary_name());
                failed += 1;
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
    }

    Ok((done, failed, first_error))
}

fn run_bug_search_parallel(
    root: &Path,
    cli: AgentCli,
    config: &BugSearchConfig,
    total_files: usize,
    work: Vec<(usize, PathBuf, PathBuf)>,
) -> Result<(usize, usize, Option<AgentsError>), AgentsError> {
    let root = root.to_path_buf();
    let config = config.clone();
    let worker_count = config.jobs.min(work.len());
    let queue = Arc::new(Mutex::new(VecDeque::from(work)));
    let (tx, rx) = mpsc::channel();

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            let root = root.clone();
            let config = config.clone();
            scope.spawn(move || {
                loop {
                    let item = {
                        let mut queue = queue.lock().expect("bug search queue poisoned");
                        queue.pop_front()
                    };
                    let Some((idx, src, out)) = item else {
                        break;
                    };
                    let result = bug_search_one(&root, cli, &config, total_files, idx, &src, &out);
                    let _ = tx.send((idx, result));
                }
            });
        }
        drop(tx);
    });

    let mut done = 0usize;
    let mut failed = 0usize;
    let mut first_error = None;
    for (idx, result) in rx {
        match result {
            Ok(()) => done += 1,
            Err(err) => {
                eprintln!("  [{idx}] {} exited with error: {err}", cli.binary_name());
                failed += 1;
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
    }

    Ok((done, failed, first_error))
}

fn bug_search_one(
    root: &Path,
    cli: AgentCli,
    config: &BugSearchConfig,
    total_files: usize,
    idx: usize,
    src: &Path,
    out: &Path,
) -> Result<(), AgentsError> {
    let rel_src = rel_display(root, src);
    let rel_out = rel_display(root, out);
    let prompt_path = if config.jobs == 1 {
        root.join(BUG_SEARCH_TMP_PROMPT)
    } else {
        root.join(format!("{BUG_SEARCH_TMP_PROMPT}.{idx}.md"))
    };

    if !config.dry_run {
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let prompt = render_bug_search_prompt(&rel_src, &rel_out);
        fs::write(&prompt_path, prompt)?;
    }

    println!("[{idx}/{total_files}] search: {rel_src} -> {rel_out}");
    let result = run_bug_search_agent(cli, root, &prompt_path, config.dry_run);

    if config.jobs != 1 && !config.dry_run {
        let _ = fs::remove_file(prompt_path);
    }

    result
}

fn run_bug_search_agent(
    cli: AgentCli,
    root: &Path,
    prompt_path: &Path,
    dry_run: bool,
) -> Result<(), AgentsError> {
    let prompt_ref = format!("@{}", rel_display(root, prompt_path));
    let instruction = format!("Read and follow the following prompt {prompt_ref}");
    let mut command = cli.command();
    command.current_dir(root);
    match cli {
        AgentCli::Gemini => {
            command.arg("-p").arg(&prompt_ref).arg("--yolo");
        }
        AgentCli::Qwen => {
            command.arg("-p").arg(&instruction).arg("--yolo");
        }
        AgentCli::Copilot => {
            command.arg("--allow-all-tools").arg("-p").arg(&instruction);
        }
        AgentCli::Claude => {
            command
                .arg("--dangerously-skip-permissions")
                .arg("-p")
                .arg(&instruction);
        }
        AgentCli::Codex => {
            command
                .arg("exec")
                .arg("--skip-git-repo-check")
                .arg("--sandbox")
                .arg("workspace-write")
                .arg("-C")
                .arg(root)
                .arg(&instruction);
        }
    }

    if dry_run {
        print_dry_run_command(&command);
        Ok(())
    } else {
        run_interactive_tty_command(&mut command, workflow_timeout())
    }
}

fn print_dry_run_command(command: &Command) {
    let program = command.get_program().to_string_lossy();
    let args = command
        .get_args()
        .map(|arg| shell_quote(&arg.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    if args.is_empty() {
        println!("  (dry-run) {program}");
    } else {
        println!("  (dry-run) {program} {args}");
    }
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "-_./:@".contains(ch))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn render_bug_search_prompt(rel_src: &str, rel_out: &str) -> String {
    let rel_tmp_out = format!("{rel_out}.tmp");
    BUG_SEARCH_PROMPT_TEMPLATE
        .replace("{rel_src}", rel_src)
        .replace("{rel_out}", rel_out)
        .replace("{rel_tmp_out}", &rel_tmp_out)
}

fn render_bug_reproduce_prompt(config: &BugSearchConfig) -> String {
    let restart_mode = if config.restart {
        "Restart mode: archive any existing reproduce state before building a fresh queue."
    } else {
        "Resume mode: if reproduce state exists, reconcile it and continue from durable state."
    };
    BUG_BASH_REPRODUCE
        .replace("{jobs}", &config.jobs.to_string())
        .replace("{restart_mode}", restart_mode)
        .replace("{reproduce_state}", BUG_REPRODUCE_STATE)
        .replace("{search_state}", BUG_SEARCH_STATE)
}

fn write_bug_search_state(
    root: &Path,
    config: &BugSearchConfig,
    files: &[PathBuf],
) -> Result<(), AgentsError> {
    let entries = files
        .iter()
        .map(|src| {
            let rel_src = rel_display(root, src);
            let out = bug_search_output_path(root, src);
            (
                rel_src,
                serde_json::json!({
                    "status": if out.exists() && !config.force { "skipped-existing" } else { "pending" },
                    "output": rel_display(root, &out),
                    "temp_output": format!("{}.tmp", rel_display(root, &out)),
                }),
            )
        })
        .collect::<serde_json::Map<String, Value>>();

    let state = serde_json::json!({
        "source_root": config.source_root.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"),
        "force": config.force,
        "limit": config.limit,
        "start_at": config.start_at.as_ref().map(|path| path.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/")),
        "jobs": config.jobs,
        "files": entries,
    });

    let state_path = root.join(BUG_SEARCH_STATE);
    if let Some(parent) = state_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = state_path.with_extension("json.tmp");
    let serialized = serde_json::to_string_pretty(&state).map_err(io::Error::other)? + "\n";
    fs::write(&tmp_path, serialized)?;
    fs::rename(tmp_path, state_path)?;
    Ok(())
}

fn find_bug_search_files(root: &Path) -> Result<Vec<PathBuf>, AgentsError> {
    let mut files = Vec::new();
    visit_bug_search_dir(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn visit_bug_search_dir(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), AgentsError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            if should_skip_bug_search_dir(&entry.file_name()) {
                continue;
            }
            visit_bug_search_dir(&path, files)?;
        } else if is_bug_search_source(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn should_skip_bug_search_dir(name: &OsStr) -> bool {
    matches!(
        name.to_str(),
        Some("target" | "node_modules" | ".git" | "vendor" | "third_party" | "tests")
    )
}

fn is_bug_search_source(path: &Path) -> bool {
    path.extension() == Some(OsStr::new("rs"))
        && !path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.ends_with("_tests.rs"))
}

fn bug_search_output_path(root: &Path, src: &Path) -> PathBuf {
    let rel = src.strip_prefix(root).unwrap_or(src);
    root.join(BUG_SEARCH_OUTPUT_ROOT)
        .join(rel)
        .with_extension("md")
}

fn rel_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

pub fn workflow_timeout() -> Option<Duration> {
    env::var("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
}

fn agent_timeout() -> Duration {
    env::var("AGENTS_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_AGENT_TIMEOUT)
}

struct GitContext {
    status_comment: String,
    context: String,
}

fn build_file_context(root: &Path) -> Result<GitContext, AgentsError> {
    let files = run_text_command(
        Command::new("git").current_dir(root).args([
            "diff",
            "--cached",
            "--name-only",
            "--diff-filter=ACMRD",
        ]),
        None,
    )?;
    let numstat = run_text_command(
        Command::new("git").current_dir(root).args([
            "diff",
            "--cached",
            "--numstat",
            "--diff-filter=ACMRD",
        ]),
        None,
    )?;

    let mut sections = Vec::new();
    for path in files.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let staged = run_command(
            Command::new("git")
                .current_dir(root)
                .arg("show")
                .arg(format!(":{path}")),
            None,
            Some(agent_timeout()),
        );

        match staged {
            Ok(output) => {
                let content = String::from_utf8_lossy(&output.stdout);
                sections.push(format!("File: {path}\n```text\n{content}\n```"));
            }
            Err(AgentsError::CommandFailed { .. }) => {
                sections.push(format!("File: {path}\n[deleted from staged tree]"));
            }
            Err(err) => return Err(err),
        }
    }

    if sections.is_empty() {
        Ok(GitContext {
            status_comment: build_status_comment(&numstat),
            context: String::from("[no staged file contents available]"),
        })
    } else {
        Ok(GitContext {
            status_comment: build_status_comment(&numstat),
            context: sections.join("\n\n"),
        })
    }
}

fn build_status_comment(numstat: &str) -> String {
    let mut lines = Vec::new();
    lines.push(String::from("#"));
    lines.push(String::from("# Changes to be committed:"));
    for entry in numstat
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let mut parts = entry.splitn(3, '\t');
        let additions = parts.next().unwrap_or_default();
        let deletions = parts.next().unwrap_or_default();
        let path = parts.next().unwrap_or_default();

        if path.is_empty() {
            continue;
        }

        let summary = match (additions.parse::<u64>(), deletions.parse::<u64>()) {
            (Ok(additions), Ok(deletions)) => {
                let total = additions + deletions;
                format!("{total} lines changed (+{additions} -{deletions})")
            }
            _ => String::from("binary file changed"),
        };
        lines.push(format!("# {path} | {summary}"));
    }
    lines.push(String::from("#"));
    lines.join("\n")
}

fn add_status_comment(message: &str, status_comment: &str) -> String {
    let mut combined = message.to_owned();
    if status_comment.is_empty() {
        return combined;
    }

    if !combined.ends_with('\n') {
        combined.push('\n');
    }
    if !combined.ends_with("\n\n") {
        combined.push('\n');
    }
    combined.push_str(status_comment);
    combined.push('\n');
    combined
}

fn build_commit_prompt(diff: &str, file_context: &str) -> String {
    format!(
        "You are writing a git commit message from a staged diff.\n\
You have exactly one turn to respond, so provide the final commit message immediately.\n\
Return only the commit message text, with no surrounding quotes, markdown, or commentary.\n\
Write a standard git commit message with a short imperative subject line.\n\
Wrap all commit message lines to 80 columns. Add a body only if it improves clarity.\n\
Do not run tools or inspect the repository. Use only the staged diff and file contents below.\n\
\n\
Staged diff:\n\
```diff\n\
{diff}\n\
```\n\
\n\
Staged file contents:\n\
{file_context}"
    )
}

fn run_agent(cli: AgentCli, root: &Path, prompt: &str) -> Result<String, AgentsError> {
    match cli {
        AgentCli::Gemini => {
            run_text_command(cli.command().current_dir(root).arg("-y"), Some(prompt))
        }
        AgentCli::Claude => run_parsed_command(
            cli.command().current_dir(root).args([
                "-p",
                "--dangerously-skip-permissions",
                "--output-format",
                "stream-json",
                "--include-partial-messages",
                "--verbose",
            ]),
            Some(prompt),
            parse_stream_json_line,
        ),
        AgentCli::Qwen => run_parsed_command(
            cli.command().current_dir(root).args([
                "-y",
                "--output-format",
                "stream-json",
                "--include-partial-messages",
            ]),
            Some(prompt),
            parse_stream_json_line,
        ),
        AgentCli::Codex => run_codex_command(root, prompt),
        AgentCli::Copilot => run_parsed_command(
            cli.command()
                .current_dir(root)
                .args(["-i", prompt, "--allow-all-tools"]),
            Some(prompt),
            parse_stream_json_line,
        ),
    }
}

pub fn run_agent_interactive(
    cli: AgentCli,
    root: &Path,
    prompt: &str,
    timeout: Option<Duration>,
) -> Result<(), AgentsError> {
    match cli {
        AgentCli::Claude => {
            let mut c = cli.command();
            c.current_dir(root)
                .arg("--dangerously-skip-permissions")
                .arg(prompt);
            run_interactive_tty_command(&mut c, timeout)
        }
        AgentCli::Gemini => {
            let mut c = cli.command();
            c.current_dir(root).arg("-y");
            run_interactive_command(&mut c, prompt, timeout)
        }
        AgentCli::Qwen => {
            let mut c = cli.command();
            c.current_dir(root).arg("-y");
            run_interactive_command(&mut c, prompt, timeout)
        }
        AgentCli::Copilot => {
            let mut c = cli.command();
            c.current_dir(root)
                .arg("--allow-all-tools")
                .arg("-p")
                .arg(prompt);
            run_interactive_tty_command(&mut c, timeout)
        }
        AgentCli::Codex => {
            let mut c = cli.command();
            c.current_dir(root)
                .arg("exec")
                .arg("--skip-git-repo-check")
                .arg("--color")
                .arg("never")
                .arg("-C")
                .arg(root)
                .arg("-");
            run_interactive_command(&mut c, prompt, timeout)
        }
    }
}

fn run_interactive_tty_command(
    command: &mut Command,
    timeout: Option<Duration>,
) -> Result<(), AgentsError> {
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let mut child = command.spawn()?;

    let status = if let Some(timeout) = timeout {
        match child.wait_timeout(timeout)? {
            Some(status) => status,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AgentsError::TimedOut {
                    program: command.get_program().to_string_lossy().into_owned(),
                    timeout,
                });
            }
        }
    } else {
        child.wait()?
    };

    if status.success() {
        Ok(())
    } else {
        Err(AgentsError::CommandFailed {
            program: command.get_program().to_string_lossy().into_owned(),
            status,
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

fn run_interactive_command(
    command: &mut Command,
    prompt: &str,
    timeout: Option<Duration>,
) -> Result<(), AgentsError> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let mut child = command.spawn()?;

    if let Some(mut pipe) = child.stdin.take() {
        if let Err(err) = pipe.write_all(prompt.as_bytes()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err.into());
        }
        drop(pipe);
    }

    let status = if let Some(timeout) = timeout {
        match child.wait_timeout(timeout)? {
            Some(status) => status,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AgentsError::TimedOut {
                    program: command.get_program().to_string_lossy().into_owned(),
                    timeout,
                });
            }
        }
    } else {
        child.wait()?
    };

    if status.success() {
        Ok(())
    } else {
        Err(AgentsError::CommandFailed {
            program: command.get_program().to_string_lossy().into_owned(),
            status,
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

fn run_codex_command(root: &Path, prompt: &str) -> Result<String, AgentsError> {
    let output_file = NamedTempFile::new()?;
    let output_path = output_file.path().to_path_buf();

    run_command(
        AgentCli::Codex
            .command()
            .current_dir(root)
            .arg("exec")
            .arg("--json")
            .arg("--skip-git-repo-check")
            .arg("--color")
            .arg("never")
            .arg("--sandbox")
            .arg("read-only")
            .arg("-C")
            .arg(root)
            .arg("-o")
            .arg(&output_path)
            .arg("-"),
        Some(prompt),
        Some(agent_timeout()),
    )?;

    let message = fs::read_to_string(output_path)?;
    Ok(message.trim().to_owned())
}

fn edit_message(initial_message: &str) -> Result<String, AgentsError> {
    let editor = env::var("EDITOR").map_err(|_| AgentsError::MissingEditor)?;
    if editor.trim().is_empty() {
        return Err(AgentsError::MissingEditor);
    }

    let mut message_file = NamedTempFile::new()?;
    message_file.write_all(initial_message.as_bytes())?;
    message_file.flush()?;

    run_interactive_status_command(
        Command::new("sh")
            .arg("-c")
            .arg("exec $EDITOR \"$1\"")
            .arg("sh")
            .arg(message_file.path()),
    )?;

    Ok(fs::read_to_string(message_file.path())?)
}

fn run_text_command(command: &mut Command, stdin: Option<&str>) -> Result<String, AgentsError> {
    let output = run_command(command, stdin, Some(agent_timeout()))?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_parsed_command(
    command: &mut Command,
    stdin: Option<&str>,
    parser: fn(&str) -> Option<String>,
) -> Result<String, AgentsError> {
    let output = run_command(command, stdin, Some(agent_timeout()))?;
    let parsed = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parser)
        .collect::<Vec<_>>()
        .join("\n");
    Ok(parsed)
}

fn run_command(
    command: &mut Command,
    stdin: Option<&str>,
    timeout: Option<Duration>,
) -> Result<std::process::Output, AgentsError> {
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }

    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;

    if let Some(input) = stdin {
        if let Some(mut pipe) = child.stdin.take() {
            pipe.write_all(input.as_bytes())?;
        }
    }

    let output = if let Some(timeout) = timeout {
        match child.wait_timeout(timeout)? {
            Some(_) => child.wait_with_output()?,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AgentsError::TimedOut {
                    program: command.get_program().to_string_lossy().into_owned(),
                    timeout,
                });
            }
        }
    } else {
        child.wait_with_output()?
    };

    if output.status.success() {
        Ok(output)
    } else {
        Err(command_failed(
            command.get_program(),
            output.status,
            &output.stdout,
            &output.stderr,
        ))
    }
}

fn run_status_command(command: &mut Command) -> Result<(), AgentsError> {
    let output = command.output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failed(
            command.get_program(),
            output.status,
            &output.stdout,
            &output.stderr,
        ))
    }
}

fn run_interactive_status_command(command: &mut Command) -> Result<(), AgentsError> {
    let status = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(AgentsError::CommandFailed {
            program: command.get_program().to_string_lossy().into_owned(),
            status,
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

fn command_failed(
    program: &OsStr,
    status: ExitStatus,
    stdout: &[u8],
    stderr: &[u8],
) -> AgentsError {
    AgentsError::CommandFailed {
        program: program.to_string_lossy().into_owned(),
        status,
        stdout: String::from_utf8_lossy(stdout).into_owned(),
        stderr: String::from_utf8_lossy(stderr).into_owned(),
    }
}

fn parse_stream_json_line(raw: &str) -> Option<String> {
    let stripped = raw.trim();
    if stripped.is_empty() || !stripped.starts_with('{') {
        return None;
    }

    let obj: Value = serde_json::from_str(stripped).ok()?;
    let msg_type = obj.get("type")?.as_str()?;

    if msg_type == "stream_event" {
        let event = obj.get("event")?;
        if event.get("type")?.as_str()? == "content_block_delta" {
            let delta = event.get("delta")?;
            match delta.get("type")?.as_str()? {
                "text_delta" => return delta.get("text")?.as_str().map(ToOwned::to_owned),
                "input_json_delta" => {
                    return delta.get("partial_json")?.as_str().map(ToOwned::to_owned);
                }
                _ => return None,
            }
        }
        return None;
    }

    if msg_type == "result" {
        return obj
            .get("result")?
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned);
    }

    if msg_type == "assistant" {
        let content = obj.get("message")?.get("content")?.as_array()?;
        let mut parts = Vec::new();
        for block in content {
            if block.get("type")?.as_str()? == "text" {
                let text = block.get("text")?.as_str()?.trim();
                if !text.is_empty() {
                    parts.push(text.to_owned());
                }
            }
        }
        return (!parts.is_empty()).then(|| parts.join("\n"));
    }

    None
}

#[cfg(test)]
fn parse_codex_json_line(raw: &str) -> Option<String> {
    let stripped = raw.trim();
    if stripped.is_empty() || !stripped.starts_with('{') {
        return None;
    }

    let obj: Value = serde_json::from_str(stripped).ok()?;
    let msg_type = obj.get("type")?.as_str()?;

    if matches!(
        msg_type,
        "thread.started" | "turn.started" | "turn.completed" | "item.started"
    ) {
        return None;
    }

    if msg_type != "item.completed" {
        return None;
    }

    let item = obj.get("item")?;
    if item.get("type")?.as_str()? != "agent_message" {
        return None;
    }

    item.get("text")?
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{
        AgentCli, BugSearchConfig, Phase, TARGETS, add_status_comment, build_commit_prompt,
        build_status_comment, doc, parse_codex_json_line, parse_stream_json_line,
        render_bug_reproduce_prompt, render_bug_search_prompt, run_agent_interactive,
        todo_workflow, workflow_timeout,
    };
    use std::sync::Mutex;
    use std::time::Duration;

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn doc_copies_to_all_targets() {
        let tmpdir = tempdir().unwrap();
        let source_dir = tmpdir.path().join(".agents");
        fs::create_dir_all(&source_dir).unwrap();

        let content = "# My Project\n\nUse pytest. Format with ruff.\n";
        fs::write(source_dir.join("AGENT.md"), content).unwrap();

        let written = doc(tmpdir.path()).unwrap();

        let mut seen_paths = HashSet::new();
        for (_, rel) in TARGETS {
            let dest = tmpdir.path().join(rel);
            if !seen_paths.insert(dest.clone()) {
                continue;
            }

            assert!(dest.is_file(), "{rel} was not created");
            assert_eq!(
                fs::read_to_string(&dest).unwrap(),
                content,
                "{rel} has wrong content"
            );
        }

        let expected: HashSet<PathBuf> = TARGETS
            .iter()
            .map(|(_, rel)| tmpdir.path().join(rel))
            .collect();
        let actual: HashSet<PathBuf> = written.into_iter().collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn doc_errors_when_source_file_is_missing() {
        let tmpdir = tempdir().unwrap();
        let err = doc(tmpdir.path()).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(err.to_string(), "error: .agents/AGENT.md not found");
    }

    #[test]
    fn commit_prompt_includes_diff_and_response_constraints() {
        let prompt = build_commit_prompt("diff --git a/foo b/foo", "File: foo\n```text\nbody\n```");

        assert!(prompt.contains("Return only the commit message text"));
        assert!(prompt.contains("Write a standard git commit message"));
        assert!(prompt.contains("Wrap all commit message lines to 80 columns"));
        assert!(prompt.contains("diff --git a/foo b/foo"));
        assert!(prompt.contains("Staged file contents:"));
        assert!(prompt.contains("File: foo"));
    }

    #[test]
    fn status_comment_formats_files_with_change_counts() {
        let comment = build_status_comment("2\t1\tsrc/main.rs\n-\t-\tassets/logo.png\n");

        assert!(comment.contains("# Changes to be committed:"));
        assert!(comment.contains("# src/main.rs | 3 lines changed (+2 -1)"));
        assert!(comment.contains("# assets/logo.png | binary file changed"));
        assert!(comment.lines().all(|line| line.starts_with('#')));
    }

    #[test]
    fn add_status_comment_separates_message_from_comment_block() {
        let combined = add_status_comment(
            "feat: update flow\n",
            "#\n# Changes to be committed:\n# src/main.rs | 1 lines changed (+1 -0)\n#",
        );

        assert!(combined.starts_with("feat: update flow\n\n#\n# Changes to be committed:\n"));
        assert!(combined.ends_with("#\n"));
    }

    #[test]
    fn agent_cli_binary_names_match_expected_commands() {
        assert_eq!(AgentCli::Qwen.binary_name(), "qwen");
        assert_eq!(AgentCli::Gemini.binary_name(), "gemini");
        assert_eq!(AgentCli::Claude.binary_name(), "claude");
        assert_eq!(AgentCli::Codex.binary_name(), "codex");
        assert_eq!(AgentCli::Copilot.binary_name(), "copilot");
    }

    #[test]
    fn parses_stream_json_result_messages() {
        let parsed =
            parse_stream_json_line(r#"{"type":"result","result":"feat: add commit helper"}"#);
        assert_eq!(parsed.as_deref(), Some("feat: add commit helper"));
    }

    #[test]
    fn workflow_timeout_reads_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("AGENTS_WORKFLOW_TIMEOUT_SECS");
        }
        assert!(workflow_timeout().is_none());

        unsafe {
            std::env::set_var("AGENTS_WORKFLOW_TIMEOUT_SECS", "0");
        }
        assert!(workflow_timeout().is_none());

        unsafe {
            std::env::set_var("AGENTS_WORKFLOW_TIMEOUT_SECS", "42");
        }
        assert_eq!(workflow_timeout(), Some(Duration::from_secs(42)));

        unsafe {
            std::env::remove_var("AGENTS_WORKFLOW_TIMEOUT_SECS");
        }
    }

    #[test]
    fn dry_run_plan_lists_phases_in_order() {
        let root = tempdir().unwrap();
        let plan = todo_workflow(root.path(), AgentCli::Claude, Phase::All, true).unwrap();
        let phases: Vec<_> = plan.iter().map(|e| e.phase).collect();
        assert_eq!(
            phases,
            vec![Phase::Plan, Phase::Implement, Phase::Land, Phase::Review]
        );
    }

    #[test]
    fn phase_parses_from_clap() {
        assert_eq!(
            Phase::All.expand(),
            vec![Phase::Plan, Phase::Implement, Phase::Land, Phase::Review]
        );
        assert_eq!(Phase::Plan.expand(), vec![Phase::Plan]);
        assert_eq!(Phase::Implement.expand(), vec![Phase::Implement]);
        assert_eq!(Phase::Land.expand(), vec![Phase::Land]);
    }

    #[test]
    fn bug_search_prompt_requires_atomic_output_rename() {
        let prompt = render_bug_search_prompt("src/lib.rs", "docs/bugs/src/lib.md");

        assert!(prompt.contains("docs/bugs/src/lib.md.tmp"));
        assert!(prompt.contains("atomically renaming it to\n`docs/bugs/src/lib.md`"));
        assert!(prompt.contains("Do not leave partial `docs/bugs/src/lib.md` files behind"));
    }

    #[test]
    fn bug_reproduce_prompt_renders_resume_settings() {
        let mut config = BugSearchConfig::new(false);
        config.jobs = 4;
        let prompt = render_bug_reproduce_prompt(&config);

        assert!(prompt.contains("Concurrency: keep at most 4 reproduce worker(s) active"));
        assert!(prompt.contains("State file: `docs/bugs/reproduce-state.json`"));
        assert!(prompt.contains("Search snapshot: `docs/bugs/search-state.json`"));
        assert!(prompt.contains("Resume mode: if reproduce state exists"));
    }

    #[test]
    fn bug_reproduce_prompt_defines_manifest_contract() {
        let prompt = render_bug_reproduce_prompt(&BugSearchConfig::new(false));

        assert!(prompt.contains("Worker manifest format:"));
        assert!(prompt.contains("\"work_item\": \"docs/bugs/src/lib.md#BUG-001\""));
        assert!(prompt.contains("\"status\": \"reproduced\""));
        assert!(prompt.contains("\"test_file\": \"tests/regression.rs\""));
        assert!(prompt.contains("\"test_name\": \"regression_bug_001_src_lib_empty_input\""));
        assert!(
            prompt.contains("\"command\": \"cargo test regression_bug_001_src_lib_empty_input\"")
        );
        assert!(prompt.contains("\"failure_excerpt\": \"assertion failed: ...\""));
        assert!(prompt.contains("worker_commit"));
        assert!(
            prompt.contains("Allowed manifest statuses: `reproduced`, `withdrawn`, `blocked`,")
        );
        assert!(prompt.contains("`needs-review`"));
    }

    #[test]
    fn bug_reproduce_prompt_restricts_registry_and_summary_writes_to_coordinator() {
        let prompt = render_bug_reproduce_prompt(&BugSearchConfig::new(false));

        assert!(prompt.contains("You are the only writer of `docs/bugs/**/*.md`"));
        assert!(prompt.contains("Worker subagents write tests in their own worktrees"));
        assert!(prompt.contains("They must not edit registries, summaries, or coordinator state"));
        assert!(prompt.contains("Keep registry annotations local to the per-file registry"));
    }

    #[test]
    fn bug_reproduce_prompt_requires_manifest_copy_and_commit_accounting() {
        let prompt = render_bug_reproduce_prompt(&BugSearchConfig::new(false));

        assert!(prompt.contains("coordinator_commit"));
        assert!(
            prompt.contains("Copy every accepted worker manifest into the coordinator checkout")
        );
        assert!(
            prompt.contains("The number of coordinator-local manifest files equals the number")
        );
        assert!(prompt.contains("recover in place"));
        assert!(prompt.contains("do not wait for worker processes from a\n  previous invocation"));
        assert!(prompt.contains("Worker completion is mandatory"));
        assert!(
            prompt.contains("Do not require the full suite to\n  pass inside a worker worktree")
        );
        assert!(prompt.contains("Reproduced tests are intentionally failing"));
        assert!(prompt.contains("match\n     exactly between"));
        assert!(prompt.contains("Do not later overwrite an earlier bug's\n  `coordinator_commit`"));
        assert!(prompt.contains("Do not add a top-level `coordinator_commit`"));
    }

    #[test]
    fn bug_reproduce_prompt_covers_interrupted_worker_recovery() {
        let prompt = render_bug_reproduce_prompt(&BugSearchConfig::new(false));

        assert!(prompt.contains("do not wait for worker processes from a\n  previous invocation"));
        assert!(prompt.contains("inspect the\n  worktree immediately"));
        assert!(prompt.contains("If test edits exist, recover in place"));
        assert!(prompt.contains("Worker completion is mandatory"));
        assert!(prompt.contains("Do not leave a worker worktree with uncommitted\n"));
    }

    #[test]
    fn bug_reproduce_prompt_covers_intentionally_failing_tests() {
        let prompt = render_bug_reproduce_prompt(&BugSearchConfig::new(false));

        assert!(
            prompt.contains("Do not require the full suite to\n  pass inside a worker worktree")
        );
        assert!(prompt.contains("Reproduced tests are intentionally failing"));
        assert!(prompt.contains("previously accepted regression tests"));
        assert!(prompt.contains("must not block\n  accepting a new narrow reproduced test"));
        assert!(prompt.contains("Passing\n  the suite is the fix phase's job"));
    }

    #[test]
    fn bug_reproduce_prompt_covers_dynamic_registry_discovery() {
        let prompt = render_bug_reproduce_prompt(&BugSearchConfig::new(false));

        assert!(prompt.contains("Re-scan `docs/bugs/**/*.md` at startup"));
        assert!(prompt.contains("between worker batches"));
        assert!(prompt.contains("Enqueue new bug entries found mid-run"));
        assert!(prompt.contains("Ignore `*.tmp` registries"));
        assert!(prompt.contains("If a registry entry changes while its worker is in progress"));
    }

    #[test]
    fn bug_reproduce_prompt_renders_restart_settings() {
        let mut config = BugSearchConfig::new(false);
        config.restart = true;
        let prompt = render_bug_reproduce_prompt(&config);

        assert!(prompt.contains("Restart mode: archive any existing reproduce state"));
    }

    #[cfg(unix)]
    #[test]
    fn run_agent_interactive_does_not_hang_when_child_exits_without_reading_stdin() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::mpsc;
        use std::thread;

        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        let stub = tmp.path().join("stub.sh");
        fs::write(&stub, "#!/usr/bin/env bash\nexit 1\n").unwrap();
        let mut perms = fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&stub, perms).unwrap();

        unsafe {
            std::env::set_var("AGENTS_CLAUDE_BIN", &stub);
        }

        // Prompt large enough to overflow a typical pipe buffer (64 KiB on Linux).
        let prompt = "x".repeat(1024 * 1024);
        let root = tempdir().unwrap();
        let root_path = root.path().to_path_buf();

        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let result = run_agent_interactive(
                AgentCli::Claude,
                &root_path,
                &prompt,
                Some(Duration::from_secs(5)),
            );
            let _ = tx.send(result.is_err());
        });

        let outcome = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("run_agent_interactive hung when child exited without consuming stdin");
        assert!(outcome, "expected an error when child exits 1");
        handle.join().unwrap();

        unsafe {
            std::env::remove_var("AGENTS_CLAUDE_BIN");
        }
    }

    #[test]
    fn parses_codex_agent_messages() {
        let parsed = parse_codex_json_line(
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"feat: add commit helper"}}"#,
        );
        assert_eq!(parsed.as_deref(), Some("feat: add commit helper"));
    }
}
