use std::collections::HashSet;
use std::env;
use std::error::Error as StdError;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;

use clap::ValueEnum;
use serde_json::Value;
use tempfile::NamedTempFile;
use wait_timeout::ChildExt;

pub const SOURCE_FILE: &str = ".agents/AGENT.md";
const DEFAULT_AGENT_TIMEOUT: Duration = Duration::from_secs(30);
pub const TARGETS: &[(&str, &str)] = &[
    ("claude", "CLAUDE.md"),
    ("codex", "AGENTS.md"),
    ("gemini", "GEMINI.md"),
    ("copilot", ".github/copilot-instructions.md"),
    ("qwen", "QWEN.md"),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Phase {
    Plan,
    Implement,
    Land,
    All,
}

impl Phase {
    pub fn expand(self) -> Vec<Phase> {
        match self {
            Self::All => vec![Self::Plan, Self::Implement, Self::Land],
            single => vec![single],
        }
    }

    pub fn prompt_filename(self) -> &'static str {
        match self {
            Self::Plan => "prompt_01.md",
            Self::Implement => "prompt_02.md",
            Self::Land => "prompt_03.md",
            Self::All => "",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Implement => "implement",
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
}

impl AgentCli {
    fn binary_name(self) -> &'static str {
        match self {
            Self::Qwen => "qwen",
            Self::Gemini => "gemini",
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    fn env_var_name(self) -> &'static str {
        match self {
            Self::Qwen => "AGENTS_QWEN_BIN",
            Self::Gemini => "AGENTS_GEMINI_BIN",
            Self::Claude => "AGENTS_CLAUDE_BIN",
            Self::Codex => "AGENTS_CODEX_BIN",
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
    pub prompt_path: PathBuf,
}

pub fn todo_workflow(
    root: &Path,
    cli: AgentCli,
    phases: &[Phase],
    prompts_dir: Option<&Path>,
    dry_run: bool,
) -> Result<Vec<WorkflowPlanEntry>, AgentsError> {
    let mut expanded: Vec<Phase> = Vec::new();
    for phase in phases {
        for sub in phase.expand() {
            if !expanded.contains(&sub) {
                expanded.push(sub);
            }
        }
    }

    let mut plan: Vec<WorkflowPlanEntry> = Vec::new();
    for phase in &expanded {
        let prompt_path = resolve_prompt_path(root, prompts_dir, *phase)?;
        plan.push(WorkflowPlanEntry {
            phase: *phase,
            prompt_path,
        });
    }

    if dry_run {
        println!("todo-workflow plan ({} phase(s)):", plan.len());
        for (idx, entry) in plan.iter().enumerate() {
            println!(
                "  {}. {} -> {}",
                idx + 1,
                entry.phase.label(),
                entry.prompt_path.display()
            );
        }
        println!("cli: {}", cli.binary_name());
        println!("root: {}", root.display());
        return Ok(plan);
    }

    if matches!(cli, AgentCli::Codex) {
        eprintln!(
            "warning: --cli codex uses one-shot exec; claude is recommended for todo-workflow"
        );
    }

    let timeout = workflow_timeout();
    for (idx, entry) in plan.iter().enumerate() {
        eprintln!(
            "=== Phase {}: {} ===",
            idx + 1,
            entry.phase.label()
        );
        let prompt = fs::read_to_string(&entry.prompt_path)?;
        if let Err(err) = run_agent_interactive(cli, root, &prompt, timeout) {
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

pub fn resolve_prompt_path(
    root: &Path,
    prompts_dir: Option<&Path>,
    phase: Phase,
) -> Result<PathBuf, AgentsError> {
    let filename = phase.prompt_filename();
    let mut searched: Vec<PathBuf> = Vec::new();

    let try_dir = |dir: PathBuf, searched: &mut Vec<PathBuf>| -> Option<PathBuf> {
        let candidate = dir.join(filename);
        if candidate.is_file() {
            Some(candidate)
        } else {
            searched.push(dir);
            None
        }
    };

    if let Some(explicit) = prompts_dir {
        if let Some(found) = try_dir(explicit.to_path_buf(), &mut searched) {
            return Ok(found);
        }
    } else if let Some(from_env) = env::var_os("AGENTS_PROMPTS_DIR") {
        if let Some(found) = try_dir(PathBuf::from(from_env), &mut searched) {
            return Ok(found);
        }
    } else {
        let default_dir = root.join("prompts").join("todo-workflow");
        if let Some(found) = try_dir(default_dir, &mut searched) {
            return Ok(found);
        }
    }

    let dirs_rendered = searched
        .iter()
        .map(|d| d.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(AgentsError::Io(io::Error::new(
        io::ErrorKind::NotFound,
        format!("missing prompt file {filename} (searched: {dirs_rendered})"),
    )))
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
        Command::new("git")
            .current_dir(root)
            .args(["diff", "--cached", "--numstat", "--diff-filter=ACMRD"]),
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
    for entry in numstat.lines().map(str::trim).filter(|line| !line.is_empty()) {
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
        AgentCli::Gemini => run_text_command(
            cli.command().current_dir(root).arg("-y"),
            Some(prompt),
        ),
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
    }
}

pub fn run_agent_interactive(
    cli: AgentCli,
    root: &Path,
    prompt: &str,
    timeout: Option<Duration>,
) -> Result<(), AgentsError> {
    let mut command = match cli {
        AgentCli::Gemini => {
            let mut c = cli.command();
            c.current_dir(root).arg("-y");
            c
        }
        AgentCli::Claude => {
            let mut c = cli.command();
            c.current_dir(root)
                .args(["-p", "--dangerously-skip-permissions"]);
            c
        }
        AgentCli::Qwen => {
            let mut c = cli.command();
            c.current_dir(root).arg("-y");
            c
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
            c
        }
    };

    run_interactive_command(&mut command, prompt, timeout)
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
        pipe.write_all(prompt.as_bytes())?;
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
        AgentCli, Phase, TARGETS, add_status_comment, build_commit_prompt, build_status_comment,
        doc, parse_codex_json_line, parse_stream_json_line, resolve_prompt_path, todo_workflow,
        workflow_timeout,
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
        let combined = add_status_comment("feat: update flow\n", "#\n# Changes to be committed:\n# src/main.rs | 1 lines changed (+1 -0)\n#");

        assert!(combined.starts_with("feat: update flow\n\n#\n# Changes to be committed:\n"));
        assert!(combined.ends_with("#\n"));
    }

    #[test]
    fn agent_cli_binary_names_match_expected_commands() {
        assert_eq!(AgentCli::Qwen.binary_name(), "qwen");
        assert_eq!(AgentCli::Gemini.binary_name(), "gemini");
        assert_eq!(AgentCli::Claude.binary_name(), "claude");
        assert_eq!(AgentCli::Codex.binary_name(), "codex");
    }

    #[test]
    fn parses_stream_json_result_messages() {
        let parsed =
            parse_stream_json_line(r#"{"type":"result","result":"feat: add commit helper"}"#);
        assert_eq!(parsed.as_deref(), Some("feat: add commit helper"));
    }

    #[test]
    fn resolve_prompt_path_prefers_env_var() {
        let _guard = ENV_LOCK.lock().unwrap();
        let root = tempdir().unwrap();
        let env_dir = tempdir().unwrap();
        let default_dir = root.path().join("prompts").join("todo-workflow");
        fs::create_dir_all(&default_dir).unwrap();
        fs::write(default_dir.join("prompt_01.md"), "default").unwrap();
        fs::write(env_dir.path().join("prompt_01.md"), "from-env").unwrap();

        // Safety: tests that touch env vars serialize via ENV_LOCK.
        unsafe {
            std::env::set_var("AGENTS_PROMPTS_DIR", env_dir.path());
        }
        let resolved = resolve_prompt_path(root.path(), None, Phase::Plan).unwrap();
        assert_eq!(fs::read_to_string(&resolved).unwrap(), "from-env");

        unsafe {
            std::env::remove_var("AGENTS_PROMPTS_DIR");
        }
        let resolved = resolve_prompt_path(root.path(), None, Phase::Plan).unwrap();
        assert_eq!(fs::read_to_string(&resolved).unwrap(), "default");
    }

    #[test]
    fn resolve_prompt_path_errors_when_missing() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("AGENTS_PROMPTS_DIR");
        }
        let root = tempdir().unwrap();
        let err = resolve_prompt_path(root.path(), None, Phase::Implement).unwrap_err();
        let rendered = err.to_string();
        assert!(rendered.contains("prompt_02.md"), "msg was: {rendered}");
        assert!(
            rendered.contains("prompts/todo-workflow") || rendered.contains("prompts"),
            "msg was: {rendered}"
        );
    }

    #[test]
    fn resolve_prompt_path_errors_when_env_dir_missing_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        let root = tempdir().unwrap();
        let env_dir = tempdir().unwrap();
        // Populate the default dir to prove the env var path is authoritative:
        // if env var is set and its file is missing, we must NOT fall back.
        let default_dir = root.path().join("prompts").join("todo-workflow");
        fs::create_dir_all(&default_dir).unwrap();
        fs::write(default_dir.join("prompt_01.md"), "default").unwrap();

        // Safety: tests that touch env vars serialize via ENV_LOCK.
        unsafe {
            std::env::set_var("AGENTS_PROMPTS_DIR", env_dir.path());
        }
        let err = resolve_prompt_path(root.path(), None, Phase::Plan).unwrap_err();
        let rendered = err.to_string();
        unsafe {
            std::env::remove_var("AGENTS_PROMPTS_DIR");
        }

        assert!(rendered.contains("prompt_01.md"), "msg was: {rendered}");
        assert!(
            rendered.contains(&env_dir.path().display().to_string()),
            "msg was: {rendered}"
        );
        assert!(
            !rendered.contains("prompts/todo-workflow"),
            "should not have searched default dir; msg was: {rendered}"
        );
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
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("AGENTS_PROMPTS_DIR");
        }
        let root = tempdir().unwrap();
        let dir = root.path().join("prompts").join("todo-workflow");
        fs::create_dir_all(&dir).unwrap();
        for name in ["prompt_01.md", "prompt_02.md", "prompt_03.md"] {
            fs::write(dir.join(name), name).unwrap();
        }
        let plan = todo_workflow(root.path(), AgentCli::Claude, &[Phase::All], None, true).unwrap();
        let phases: Vec<_> = plan.iter().map(|e| e.phase).collect();
        assert_eq!(phases, vec![Phase::Plan, Phase::Implement, Phase::Land]);
        assert!(plan[0].prompt_path.ends_with("prompt_01.md"));
        assert!(plan[1].prompt_path.ends_with("prompt_02.md"));
        assert!(plan[2].prompt_path.ends_with("prompt_03.md"));
    }

    #[test]
    fn phase_parses_from_clap() {
        assert_eq!(
            Phase::All.expand(),
            vec![Phase::Plan, Phase::Implement, Phase::Land]
        );
        assert_eq!(Phase::Plan.expand(), vec![Phase::Plan]);
        assert_eq!(Phase::Implement.expand(), vec![Phase::Implement]);
        assert_eq!(Phase::Land.expand(), vec![Phase::Land]);
    }

    #[test]
    fn parses_codex_agent_messages() {
        let parsed = parse_codex_json_line(
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"feat: add commit helper"}}"#,
        );
        assert_eq!(parsed.as_deref(), Some("feat: add commit helper"));
    }
}
