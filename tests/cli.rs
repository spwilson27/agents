use std::process::Command;

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
