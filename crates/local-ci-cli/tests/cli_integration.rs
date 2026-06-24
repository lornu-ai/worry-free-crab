use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_version_flag() {
    let mut cmd = Command::cargo_bin("local-ci").unwrap();
    cmd.arg("--version");
    cmd.assert().success();
}

#[test]
fn test_help_flag() {
    let mut cmd = Command::cargo_bin("local-ci").unwrap();
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("local-ci"));
}

#[test]
fn test_init_config() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join(".local-ci.toml");

    let mut cmd = Command::cargo_bin("local-ci").unwrap();
    cmd.current_dir(temp_dir.path())
        .arg("init")
        .assert()
        .success();

    assert!(config_path.exists(), "Config file should be created");
}

#[test]
fn test_list_stages_without_config() {
    let temp_dir = TempDir::new().unwrap();

    let mut cmd = Command::cargo_bin("local-ci").unwrap();
    cmd.current_dir(temp_dir.path())
        .arg("--list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("config"));
}

#[test]
fn test_dry_run_mode() {
    let temp_dir = TempDir::new().unwrap();
    let config_content = r#"
[cache]
skip_dirs = [".git"]

[stages.test]
command = ["echo", "hello"]
timeout = 10
enabled = true
"#;

    fs::write(temp_dir.path().join(".local-ci.toml"), config_content).unwrap();

    let mut cmd = Command::cargo_bin("local-ci").unwrap();
    cmd.current_dir(temp_dir.path())
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("test"));
}

#[test]
fn test_json_output() {
    let temp_dir = TempDir::new().unwrap();
    let config_content = r#"
[cache]
skip_dirs = [".git"]

[stages.test]
command = ["echo", "hello"]
timeout = 10
enabled = true
"#;

    fs::write(temp_dir.path().join(".local-ci.toml"), config_content).unwrap();

    let mut cmd = Command::cargo_bin("local-ci").unwrap();
    cmd.current_dir(temp_dir.path())
        .arg("--json")
        .arg("--list")
        .assert()
        .success()
        .stdout(predicate::str::contains("test"));
}

#[test]
fn test_unknown_stage() {
    let temp_dir = TempDir::new().unwrap();
    let config_content = r#"
[cache]
skip_dirs = [".git"]

[stages.test]
command = ["echo", "hello"]
timeout = 10
enabled = true
"#;

    fs::write(temp_dir.path().join(".local-ci.toml"), config_content).unwrap();

    let mut cmd = Command::cargo_bin("local-ci").unwrap();
    cmd.current_dir(temp_dir.path())
        .arg("nonexistent-stage")
        .assert()
        .failure();
}
