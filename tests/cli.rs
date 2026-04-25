use std::fs;
use std::process::Command;

use tempfile::tempdir;

#[test]
fn prints_help_when_no_subcommand_is_given() {
    let output = Command::new(env!("CARGO_BIN_EXE_agents"))
        .output()
        .expect("binary should run");

    assert!(!output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");
    assert!(stderr.contains("Manage AI agent instruction files."));
    assert!(stderr.contains("Usage: agents [COMMAND]"));
    assert!(stderr.contains("Commands:"));
}

#[test]
fn save_prompt_saves_file_with_default_name() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let source = home.join("my-instructions.md");
    fs::write(&source, "# Instructions\nDo the thing.\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_agents"))
        .args(["save-prompt", source.to_str().unwrap()])
        .env("HOME", home)
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    let saved = home.join(".config/agents/prompts/my-instructions.md");
    assert!(saved.exists());
    assert_eq!(fs::read_to_string(&saved).unwrap(), "# Instructions\nDo the thing.\n");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Saved prompt 'my-instructions'"));
}

#[test]
fn save_prompt_uses_custom_name() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let source = home.join("file.md");
    fs::write(&source, "content").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_agents"))
        .args(["save-prompt", source.to_str().unwrap(), "--name", "custom"])
        .env("HOME", home)
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    let saved = home.join(".config/agents/prompts/custom.md");
    assert!(saved.exists());
    assert_eq!(fs::read_to_string(&saved).unwrap(), "content");
}

#[test]
fn save_prompt_force_overwrites_existing() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let prompts_dir = home.join(".config/agents/prompts");
    fs::create_dir_all(&prompts_dir).unwrap();
    fs::write(prompts_dir.join("existing.md"), "old").unwrap();

    let source = home.join("source.md");
    fs::write(&source, "new").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_agents"))
        .args(["save-prompt", source.to_str().unwrap(), "--name", "existing", "--force"])
        .env("HOME", home)
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read_to_string(prompts_dir.join("existing.md")).unwrap(), "new");
}

#[test]
fn save_prompt_errors_on_missing_file() {
    let temp = tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_agents"))
        .args(["save-prompt", "/nonexistent/file.md"])
        .env("HOME", temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot read file"));
}

#[test]
fn prompt_prints_saved_prompt() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let prompts_dir = home.join(".config/agents/prompts");
    fs::create_dir_all(&prompts_dir).unwrap();
    fs::write(prompts_dir.join("mypr.md"), "prompt body here").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_agents"))
        .args(["prompt", "mypr"])
        .env("HOME", home)
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "prompt body here");
}

#[test]
fn prompt_errors_on_missing_prompt() {
    let temp = tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_agents"))
        .args(["prompt", "nonexistent"])
        .env("HOME", temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("prompt 'nonexistent' not found"));
}

#[test]
fn prompt_list_shows_saved_prompts() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let prompts_dir = home.join(".config/agents/prompts");
    fs::create_dir_all(&prompts_dir).unwrap();
    fs::write(prompts_dir.join("beta.md"), "b").unwrap();
    fs::write(prompts_dir.join("alpha.md"), "a").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_agents"))
        .args(["prompt", "--list"])
        .env("HOME", home)
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "alpha\nbeta");
}

#[test]
fn prompt_list_empty_shows_message() {
    let temp = tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_agents"))
        .args(["prompt", "--list"])
        .env("HOME", temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("No prompts saved yet."));
}
