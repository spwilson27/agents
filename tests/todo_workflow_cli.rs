use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::{TempDir, tempdir};

const PROMPT_BODIES: &[(&str, &str)] = &[
    ("prompt_01.md", "PLAN-PROMPT-BODY-01"),
    ("prompt_02.md", "IMPL-PROMPT-BODY-02"),
    ("prompt_03.md", "LAND-PROMPT-BODY-03"),
];

struct Fixture {
    root: TempDir,
    record_dir: TempDir,
    stub: PathBuf,
}

fn write_prompts(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    for (name, body) in PROMPT_BODIES {
        fs::write(dir.join(name), body).unwrap();
    }
}

// Writes a shell-script stub at <record_dir>/stub.sh that:
// - reads stdin into a file named phase_N.txt where N is the number of existing
//   phase_*.txt entries + 1,
// - exits with `exit_phase_map` if the current N matches, else 0.
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
cat > "$RECORD_DIR/phase_${{n}}.txt"
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
    let record_dir = tempdir().unwrap();
    write_prompts(&root.path().join("prompts").join("todo-workflow"));
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
fn todo_workflow_runs_three_phases_in_order() {
    let fx = make_fixture(None);
    let output = bin()
        .args(["todo-workflow", "--cli", "claude", "--root"])
        .arg(fx.root.path())
        .env("AGENTS_CLAUDE_BIN", &fx.stub)
        .env_remove("AGENTS_PROMPTS_DIR")
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for (n, (_, body)) in PROMPT_BODIES.iter().enumerate() {
        let path = fx.record_dir.path().join(format!("phase_{}.txt", n + 1));
        let captured = fs::read_to_string(&path).unwrap();
        assert!(
            captured.contains(body),
            "phase {} missing body; got: {captured}",
            n + 1
        );
    }
}

#[test]
fn todo_workflow_stops_on_phase_failure() {
    let fx = make_fixture(Some(2));
    let output = bin()
        .args(["todo-workflow", "--cli", "claude", "--root"])
        .arg(fx.root.path())
        .env("AGENTS_CLAUDE_BIN", &fx.stub)
        .env_remove("AGENTS_PROMPTS_DIR")
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("phase 2") || stderr.contains("implement"),
        "stderr was: {stderr}"
    );
    assert!(fx.record_dir.path().join("phase_1.txt").is_file());
    assert!(fx.record_dir.path().join("phase_2.txt").is_file());
    assert!(!fx.record_dir.path().join("phase_3.txt").exists());
}

#[test]
fn todo_workflow_single_phase_flag() {
    let fx = make_fixture(None);
    let output = bin()
        .args(["todo-workflow", "--cli", "claude", "--phase", "land", "--root"])
        .arg(fx.root.path())
        .env("AGENTS_CLAUDE_BIN", &fx.stub)
        .env_remove("AGENTS_PROMPTS_DIR")
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let captured = fs::read_to_string(fx.record_dir.path().join("phase_1.txt")).unwrap();
    assert!(captured.contains("LAND-PROMPT-BODY-03"));
    assert!(!fx.record_dir.path().join("phase_2.txt").exists());
}

#[test]
fn todo_workflow_dry_run_prints_plan_and_skips_agent() {
    let fx = make_fixture(None);
    let output = bin()
        .args(["todo-workflow", "--cli", "claude", "--dry-run", "--root"])
        .arg(fx.root.path())
        .env("AGENTS_CLAUDE_BIN", &fx.stub)
        .env_remove("AGENTS_PROMPTS_DIR")
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("plan"));
    assert!(stdout.contains("implement"));
    assert!(stdout.contains("land"));
    assert!(stdout.contains("prompt_01.md"));
    assert!(stdout.contains("prompt_02.md"));
    assert!(stdout.contains("prompt_03.md"));
    assert!(!fx.record_dir.path().join("phase_1.txt").exists());
}

#[test]
fn todo_workflow_missing_prompt_errors_cleanly() {
    let fx = make_fixture(None);
    fs::remove_file(
        fx.root
            .path()
            .join("prompts")
            .join("todo-workflow")
            .join("prompt_02.md"),
    )
    .unwrap();
    let output = bin()
        .args(["todo-workflow", "--cli", "claude", "--root"])
        .arg(fx.root.path())
        .env("AGENTS_CLAUDE_BIN", &fx.stub)
        .env_remove("AGENTS_PROMPTS_DIR")
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("prompt_02.md"), "stderr was: {stderr}");
    assert!(!fx.record_dir.path().join("phase_1.txt").exists());
}

#[test]
fn todo_workflow_respects_prompts_dir_env() {
    let fx = make_fixture(None);
    let alt = tempdir().unwrap();
    write_prompts(alt.path());
    fs::write(alt.path().join("prompt_01.md"), "ALT-PLAN-BODY").unwrap();
    let output = bin()
        .args([
            "todo-workflow",
            "--cli",
            "claude",
            "--phase",
            "plan",
            "--root",
        ])
        .arg(fx.root.path())
        .env("AGENTS_CLAUDE_BIN", &fx.stub)
        .env("AGENTS_PROMPTS_DIR", alt.path())
        .env_remove("AGENTS_WORKFLOW_TIMEOUT_SECS")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let captured = fs::read_to_string(fx.record_dir.path().join("phase_1.txt")).unwrap();
    assert!(captured.contains("ALT-PLAN-BODY"), "got: {captured}");
}
