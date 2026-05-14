use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::{TempDir, tempdir};

// Exact bytes of the embedded reproduction prompt under prompts/bug-bash/.
const PROMPT_REPRODUCE: &str = include_str!("../prompts/bug-bash/prompt_02.md");

fn expected_reproduce_prompt(work_item: &str, restart: bool) -> String {
    let registry_path = work_item
        .split_once('#')
        .map(|(path, _)| path)
        .unwrap_or(work_item);
    let restart_mode = if restart {
        "Restart mode: the `agents` tool archived any prior `docs/bugs/reproduce-state.json` before this run; treat your work item as pending unless the new state file already records progress."
    } else {
        "Resume mode: if `docs/bugs/reproduce-state.json` exists, load it and reconcile your entry before acting."
    };
    PROMPT_REPRODUCE
        .replace("{work_item}", work_item)
        .replace("{registry_path}", registry_path)
        .replace("{restart_mode}", restart_mode)
        .replace("{reproduce_state}", "docs/bugs/reproduce-state.json")
        .replace("{search_state}", "docs/bugs/search-state.json")
}

fn write_reproduce_targets_registry(root: &Path) {
    // Must not mirror `src/lib.rs` -> `docs/bugs/src/lib.md` or search skips work.
    let p = root.join("docs/bugs/extra-for-repro.md");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        &p,
        r#"# Bug Bash Registry - extra

Generated: 2020-01-01
Total: 1 bugs (1 high, 0 medium, 0 low)

## BUG-001 - Example bug
- Severity: high
- Location: src/lib.rs:1
- Description: test
- Reproduction hypothesis: invoke example()
- Suggested regression test: assert return value
"#,
    )
    .unwrap();
}

fn write_reproduce_targets_registry_two_bugs(root: &Path) {
    let p = root.join("docs/bugs/extra-for-repro.md");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        &p,
        r#"# Bug Bash Registry - extra

Generated: 2020-01-01
Total: 2 bugs (2 high, 0 medium, 0 low)

## BUG-001 - Example bug one
- Severity: high
- Location: src/lib.rs:1
- Description: test
- Reproduction hypothesis: invoke example()
- Suggested regression test: assert return value

## BUG-002 - Example bug two
- Severity: high
- Location: src/lib.rs:2
- Description: test two
- Reproduction hypothesis: invoke example() with edge input
- Suggested regression test: assert edge return value
"#,
    )
    .unwrap();
}

fn write_reproduce_targets_registry_many_bugs(root: &Path, count: usize) {
    let p = root.join("docs/bugs/extra-for-repro.md");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    let mut content =
        format!("# Bug Bash Registry - extra\n\nGenerated: 2020-01-01\nTotal: {count} bugs\n\n");
    for n in 1..=count {
        content.push_str(&format!(
            "## BUG-{n:03} - Example bug {n}\n- Severity: high\n- Location: src/lib.rs:{n}\n- Description: test {n}\n\n"
        ));
    }
    fs::write(&p, content).unwrap();
}

fn write_reproduce_state(root: &Path, entries: &[(&str, &str)]) {
    let p = root.join("docs/bugs/reproduce-state.json");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    let mut map = serde_json::Map::new();
    for (key, status) in entries {
        map.insert(
            (*key).to_string(),
            serde_json::json!({
                "status": status,
            }),
        );
    }
    fs::write(&p, serde_json::to_string_pretty(&map).unwrap()).unwrap();
}

fn captured_phase_count(record_dir: &Path) -> usize {
    fs::read_dir(record_dir)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("phase_")
        })
        .count()
}

struct Fixture {
    root: TempDir,
    record_dir: TempDir,
    stub: PathBuf,
}

fn make_stub(record_dir: &Path, fail_phase: Option<usize>) -> PathBuf {
    let fail_expr = match fail_phase {
        Some(n) => format!("{n}"),
        None => "0".to_string(),
    };
    let script = format!(
        r#"#!/usr/bin/env bash
set -eu
RECORD_DIR="{record}"
count=$(ls "$RECORD_DIR"/phase_*.txt 2>/dev/null | wc -l)
n=$((count + 1))
printf '%s' "${{@: -1}}" > "$RECORD_DIR/phase_${{n}}.txt"
fail_phase={fail}
if [ "$fail_phase" -ne 0 ] && [ "$n" -eq "$fail_phase" ]; then
  echo "stub failing on phase $n" >&2
  exit 1
fi
exit 0
"#,
        record = record_dir.display(),
        fail = fail_expr,
    );
    let stub = record_dir.join("stub.sh");
    fs::write(&stub, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&stub, perms).unwrap();
    }
    stub
}

fn make_fixture(fail_phase: Option<usize>) -> Fixture {
    let root = tempdir().unwrap();
    let src_dir = root.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("lib.rs"), "pub fn example() -> usize { 1 }\n").unwrap();
    write_reproduce_targets_registry(root.path());
    let record_dir = tempdir().unwrap();
    let stub = make_stub(record_dir.path(), fail_phase);
    Fixture {
        root,
        record_dir,
        stub,
    }
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_agents"))
}

#[test]
fn bug_bash_runs_search_then_reproduce_in_order() {
    let fx = make_fixture(None);
    let output = bin()
        .args(["bug-bash", "--cli", "claude", "--root"])
        .arg(fx.root.path())
        .env("AGENTS_CLAUDE_BIN", &fx.stub)
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let search = fs::read_to_string(fx.record_dir.path().join("phase_1.txt")).unwrap();
    assert!(search.contains("Read and follow the following prompt @.bug-hunt-prompt.tmp.md"));

    let reproduce = fs::read_to_string(fx.record_dir.path().join("phase_2.txt")).unwrap();
    assert_eq!(
        reproduce.trim_end(),
        expected_reproduce_prompt("docs/bugs/extra-for-repro.md#BUG-001", false).trim_end(),
        "phase 2 captured stdin did not match embedded reproduce prompt",
    );
}

#[test]
fn bug_bash_stops_on_phase_failure() {
    let fx = make_fixture(Some(2));
    let output = bin()
        .args(["bug-bash", "--cli", "claude", "--root"])
        .arg(fx.root.path())
        .env("AGENTS_CLAUDE_BIN", &fx.stub)
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("phase 2 (reproduce) failed"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("resume with --phase reproduce"),
        "stderr was: {stderr}"
    );
    assert!(fx.record_dir.path().join("phase_1.txt").is_file());
    assert!(fx.record_dir.path().join("phase_2.txt").is_file());
    assert!(!fx.record_dir.path().join("phase_3.txt").exists());
}

#[test]
fn bug_bash_single_phase_flag() {
    let fx = make_fixture(None);
    let output = bin()
        .args([
            "bug-bash",
            "--cli",
            "claude",
            "--phase",
            "reproduce",
            "--root",
        ])
        .arg(fx.root.path())
        .env("AGENTS_CLAUDE_BIN", &fx.stub)
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let captured = fs::read_to_string(fx.record_dir.path().join("phase_1.txt")).unwrap();
    assert_eq!(
        captured.trim_end(),
        expected_reproduce_prompt("docs/bugs/extra-for-repro.md#BUG-001", false).trim_end(),
        "single-phase stdin did not match embedded reproduce prompt"
    );
    assert!(!fx.record_dir.path().join("phase_2.txt").exists());
}

#[test]
fn bug_bash_reproduce_prompt_includes_work_item_and_restart_mode() {
    let fx = make_fixture(None);
    let output = bin()
        .args([
            "bug-bash",
            "--cli",
            "claude",
            "--phase",
            "reproduce",
            "--jobs",
            "3",
            "--restart",
            "--root",
        ])
        .arg(fx.root.path())
        .env("AGENTS_CLAUDE_BIN", &fx.stub)
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let captured = fs::read_to_string(fx.record_dir.path().join("phase_1.txt")).unwrap();
    assert_eq!(
        captured.trim_end(),
        expected_reproduce_prompt("docs/bugs/extra-for-repro.md#BUG-001", true).trim_end(),
        "single-phase stdin did not include rendered reproduce settings"
    );
    assert!(captured.contains("docs/bugs/extra-for-repro.md#BUG-001"));
    assert!(captured.contains("Restart mode: the `agents` tool archived"));
    assert!(!captured.contains("{work_item}"));
    assert!(!captured.contains("{restart_mode}"));
    assert!(!captured.contains("{reproduce_state}"));
    assert!(!captured.contains("{search_state}"));
}

#[test]
fn bug_bash_dry_run_prints_plan_and_skips_agent() {
    let fx = make_fixture(None);
    let output = bin()
        .args(["bug-bash", "--cli", "claude", "--dry-run", "--root"])
        .arg(fx.root.path())
        .env("AGENTS_CLAUDE_BIN", &fx.stub)
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("search"));
    assert!(stdout.contains("reproduce"));
    assert!(stdout.contains("(per-file)"));
    assert!(stdout.contains("(per-bug)"));
    assert!(stdout.contains("(dry-run)"));
    assert!(!fx.record_dir.path().join("phase_1.txt").exists());
}

#[test]
fn bug_bash_search_skips_existing_outputs() {
    let fx = make_fixture(None);
    let out = fx.root.path().join("docs/bugs/src/lib.md");
    fs::create_dir_all(out.parent().unwrap()).unwrap();
    fs::write(&out, "already searched").unwrap();

    let output = bin()
        .args(["bug-bash", "--cli", "claude", "--phase", "search", "--root"])
        .arg(fx.root.path())
        .env("AGENTS_CLAUDE_BIN", &fx.stub)
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("skip (exists): src/lib.rs"));
    let state = fs::read_to_string(fx.root.path().join("docs/bugs/search-state.json")).unwrap();
    assert!(state.contains("\"src/lib.rs\""));
    assert!(state.contains("\"status\": \"skipped-existing\""));
    assert!(state.contains("\"temp_output\": \"docs/bugs/src/lib.md.tmp\""));
    assert!(!fx.record_dir.path().join("phase_1.txt").exists());
}

#[test]
fn bug_bash_reproduce_parallel_invokes_all_bugs() {
    let root = tempdir().unwrap();
    let src_dir = root.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("lib.rs"), "pub fn example() -> usize { 1 }\n").unwrap();
    write_reproduce_targets_registry_two_bugs(root.path());

    let record_dir = tempdir().unwrap();
    let stub = make_stub(record_dir.path(), None);

    let output = bin()
        .args([
            "bug-bash",
            "--cli",
            "claude",
            "--phase",
            "reproduce",
            "--jobs",
            "2",
            "--root",
        ])
        .arg(root.path())
        .env("AGENTS_CLAUDE_BIN", &stub)
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Both bugs must appear in the progress output — printed by our code before each agent
    // invocation, so they reflect what was actually dispatched regardless of stub timing.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("BUG-001"),
        "expected BUG-001 in stdout progress; got:\n{stdout}"
    );
    assert!(
        stdout.contains("BUG-002"),
        "expected BUG-002 in stdout progress; got:\n{stdout}"
    );
    assert!(
        stdout.contains("done: 2"),
        "expected done: 2 in stdout summary; got:\n{stdout}"
    );
}

#[test]
fn bug_bash_reproduce_skips_terminal_state_entries() {
    let root = tempdir().unwrap();
    let src_dir = root.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("lib.rs"), "pub fn example() -> usize { 1 }\n").unwrap();
    write_reproduce_targets_registry_two_bugs(root.path());
    write_reproduce_state(
        root.path(),
        &[("docs/bugs/extra-for-repro.md#BUG-001", "reproduced")],
    );

    let record_dir = tempdir().unwrap();
    let stub = make_stub(record_dir.path(), None);

    let output = bin()
        .args([
            "bug-bash",
            "--cli",
            "claude",
            "--phase",
            "reproduce",
            "--root",
        ])
        .arg(root.path())
        .env("AGENTS_CLAUDE_BIN", &stub)
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("reproduce resume: skipped 1 terminal work item(s), 1 remaining"),
        "stdout was:\n{stdout}"
    );
    assert!(
        !stdout.contains("reproduce: docs/bugs/extra-for-repro.md#BUG-001"),
        "BUG-001 should not have been dispatched; stdout was:\n{stdout}"
    );
    assert!(
        stdout.contains("reproduce: docs/bugs/extra-for-repro.md#BUG-002"),
        "BUG-002 should have been dispatched; stdout was:\n{stdout}"
    );
    let captured = fs::read_to_string(record_dir.path().join("phase_1.txt")).unwrap();
    assert_eq!(
        captured.trim_end(),
        expected_reproduce_prompt("docs/bugs/extra-for-repro.md#BUG-002", false).trim_end(),
    );
    assert_eq!(captured_phase_count(record_dir.path()), 1);
}

#[test]
fn bug_bash_reproduce_parallel_skips_terminal_state_entries() {
    let root = tempdir().unwrap();
    let src_dir = root.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("lib.rs"), "pub fn example() -> usize { 1 }\n").unwrap();
    write_reproduce_targets_registry_two_bugs(root.path());
    write_reproduce_state(
        root.path(),
        &[("docs/bugs/extra-for-repro.md#BUG-001", "withdrawn")],
    );

    let record_dir = tempdir().unwrap();
    let stub = make_stub(record_dir.path(), None);

    let output = bin()
        .args([
            "bug-bash",
            "--cli",
            "claude",
            "--phase",
            "reproduce",
            "--jobs",
            "2",
            "--root",
        ])
        .arg(root.path())
        .env("AGENTS_CLAUDE_BIN", &stub)
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("reproduce: docs/bugs/extra-for-repro.md#BUG-001"),
        "BUG-001 should not have been dispatched; stdout was:\n{stdout}"
    );
    assert!(
        stdout.contains("reproduce: docs/bugs/extra-for-repro.md#BUG-002"),
        "BUG-002 should have been dispatched; stdout was:\n{stdout}"
    );
    assert_eq!(captured_phase_count(record_dir.path()), 1);
}

#[test]
fn bug_bash_reproduce_status_filter_only_skips_terminal_statuses() {
    let root = tempdir().unwrap();
    let src_dir = root.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("lib.rs"), "pub fn example() -> usize { 1 }\n").unwrap();
    write_reproduce_targets_registry_many_bugs(root.path(), 9);
    write_reproduce_state(
        root.path(),
        &[
            ("docs/bugs/extra-for-repro.md#BUG-001", "reproduced"),
            ("docs/bugs/extra-for-repro.md#BUG-002", "withdrawn"),
            ("docs/bugs/extra-for-repro.md#BUG-003", "blocked"),
            ("docs/bugs/extra-for-repro.md#BUG-004", "failed"),
            ("docs/bugs/extra-for-repro.md#BUG-005", "pending"),
            ("docs/bugs/extra-for-repro.md#BUG-006", "in_progress"),
            ("docs/bugs/extra-for-repro.md#BUG-007", "needs-review"),
            ("docs/bugs/extra-for-repro.md#BUG-008", "unexpected"),
        ],
    );

    let record_dir = tempdir().unwrap();
    let stub = make_stub(record_dir.path(), None);

    let output = bin()
        .args([
            "bug-bash",
            "--cli",
            "claude",
            "--phase",
            "reproduce",
            "--root",
        ])
        .arg(root.path())
        .env("AGENTS_CLAUDE_BIN", &stub)
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for n in 1..=4 {
        assert!(
            !stdout.contains(&format!(
                "reproduce: docs/bugs/extra-for-repro.md#BUG-{n:03}"
            )),
            "terminal BUG-{n:03} should not have been dispatched; stdout was:\n{stdout}"
        );
    }
    for n in 5..=9 {
        assert!(
            stdout.contains(&format!(
                "reproduce: docs/bugs/extra-for-repro.md#BUG-{n:03}"
            )),
            "non-terminal BUG-{n:03} should have been dispatched; stdout was:\n{stdout}"
        );
    }
    assert_eq!(captured_phase_count(record_dir.path()), 5);
}

#[test]
fn bug_bash_reproduce_restart_ignores_existing_state() {
    let root = tempdir().unwrap();
    let src_dir = root.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("lib.rs"), "pub fn example() -> usize { 1 }\n").unwrap();
    write_reproduce_targets_registry_two_bugs(root.path());
    write_reproduce_state(
        root.path(),
        &[
            ("docs/bugs/extra-for-repro.md#BUG-001", "reproduced"),
            ("docs/bugs/extra-for-repro.md#BUG-002", "withdrawn"),
        ],
    );

    let record_dir = tempdir().unwrap();
    let stub = make_stub(record_dir.path(), None);

    let output = bin()
        .args([
            "bug-bash",
            "--cli",
            "claude",
            "--phase",
            "reproduce",
            "--restart",
            "--root",
        ])
        .arg(root.path())
        .env("AGENTS_CLAUDE_BIN", &stub)
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("BUG-001"), "stdout was:\n{stdout}");
    assert!(stdout.contains("BUG-002"), "stdout was:\n{stdout}");
    assert_eq!(captured_phase_count(record_dir.path()), 2);
    assert!(
        !root.path().join("docs/bugs/reproduce-state.json").exists(),
        "restart should archive the active state file"
    );
}

#[test]
fn bug_bash_reproduce_dry_run_reports_filtered_queue() {
    let root = tempdir().unwrap();
    let src_dir = root.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("lib.rs"), "pub fn example() -> usize { 1 }\n").unwrap();
    write_reproduce_targets_registry_two_bugs(root.path());
    write_reproduce_state(
        root.path(),
        &[("docs/bugs/extra-for-repro.md#BUG-001", "blocked")],
    );

    let record_dir = tempdir().unwrap();
    let stub = make_stub(record_dir.path(), None);

    let output = bin()
        .args([
            "bug-bash",
            "--cli",
            "claude",
            "--phase",
            "reproduce",
            "--dry-run",
            "--root",
        ])
        .arg(root.path())
        .env("AGENTS_CLAUDE_BIN", &stub)
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("reproduce resume: skipped 1 terminal work item(s), 1 remaining"),
        "stdout was:\n{stdout}"
    );
    assert!(
        !stdout.contains("--- reproduce 1/2 (docs/bugs/extra-for-repro.md#BUG-001)"),
        "BUG-001 should not appear in the dry-run queue; stdout was:\n{stdout}"
    );
    assert!(
        stdout.contains("--- reproduce 1/1 (docs/bugs/extra-for-repro.md#BUG-002)"),
        "BUG-002 should appear as the only dry-run work item; stdout was:\n{stdout}"
    );
    assert_eq!(captured_phase_count(record_dir.path()), 0);
}

#[test]
fn bug_bash_reproduce_parallel_continues_after_failure() {
    let root = tempdir().unwrap();
    let src_dir = root.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("lib.rs"), "pub fn example() -> usize { 1 }\n").unwrap();
    write_reproduce_targets_registry_two_bugs(root.path());

    // Stub always fails — both bugs fail, but both must be attempted.
    let record_dir = tempdir().unwrap();
    let stub = make_stub(record_dir.path(), Some(1));

    let output = bin()
        .args([
            "bug-bash",
            "--cli",
            "claude",
            "--phase",
            "reproduce",
            "--jobs",
            "2",
            "--root",
        ])
        .arg(root.path())
        .env("AGENTS_CLAUDE_BIN", &stub)
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();

    // Overall command should fail because jobs failed.
    assert!(
        !output.status.success(),
        "expected non-zero exit when reproduce jobs fail"
    );

    // Both bugs must appear in progress — parallel workers don't abort on peer failure.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("BUG-001"),
        "expected BUG-001 in stdout; got:\n{stdout}"
    );
    assert!(
        stdout.contains("BUG-002"),
        "expected BUG-002 in stdout — second bug was not dispatched despite --jobs 2; got:\n{stdout}"
    );
}
