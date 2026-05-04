use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::{TempDir, tempdir};

// Exact bytes of the embedded reproduction prompt under prompts/bug-bash/.
const PROMPT_REPRODUCE: &str = include_str!("../prompts/bug-bash/prompt_02.md");

fn expected_reproduce_prompt(jobs: usize, restart: bool) -> String {
    let restart_mode = if restart {
        "Restart mode: archive any existing reproduce state before building a fresh queue."
    } else {
        "Resume mode: if reproduce state exists, reconcile it and continue from durable state."
    };
    PROMPT_REPRODUCE
        .replace("{jobs}", &jobs.to_string())
        .replace("{restart_mode}", restart_mode)
        .replace("{reproduce_state}", "docs/bugs/reproduce-state.json")
        .replace("{search_state}", "docs/bugs/search-state.json")
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
        expected_reproduce_prompt(1, false).trim_end(),
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
        expected_reproduce_prompt(1, false).trim_end(),
        "single-phase stdin did not match embedded reproduce prompt"
    );
    assert!(!fx.record_dir.path().join("phase_2.txt").exists());
}

#[test]
fn bug_bash_reproduce_prompt_includes_jobs_and_restart_mode() {
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
        expected_reproduce_prompt(3, true).trim_end(),
        "single-phase stdin did not include rendered reproduce settings"
    );
    assert!(captured.contains("Concurrency: keep at most 3 reproduce worker(s) active"));
    assert!(captured.contains("Restart mode: archive any existing reproduce state"));
    assert!(!captured.contains("{jobs}"));
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
