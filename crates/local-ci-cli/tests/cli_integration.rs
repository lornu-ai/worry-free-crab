//! Integration tests for local-ci CLI

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_cli_version() {
    let mut cmd = Command::cargo_bin("local-ci").expect("failed to find binary");
    cmd.arg("--version");
    cmd.assert().success();
}

#[test]
fn test_cli_help() {
    let mut cmd = Command::cargo_bin("local-ci").expect("failed to find binary");
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("local-ci"));
}

#[test]
fn test_cli_list_stages() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let config_path = temp_dir.path().join("wfc.toml");

    fs::write(
        &config_path,
        r#"
[stages.fmt]
command = ["rustfmt", "--check", "src/"]
timeout = 30

[stages.clippy]
command = ["cargo", "clippy"]
timeout = 60
depends_on = ["fmt"]
"#,
    )
    .expect("failed to write config");

    let mut cmd = Command::cargo_bin("local-ci").expect("failed to find binary");
    cmd.current_dir(temp_dir.path());
    cmd.arg("--list");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("fmt").and(predicate::str::contains("clippy")));
}

#[test]
fn test_cli_init() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");

    let mut cmd = Command::cargo_bin("local-ci").expect("failed to find binary");
    cmd.current_dir(temp_dir.path());
    cmd.arg("init");
    cmd.assert().success();

    // Verify wfc.toml was created
    let config_path = temp_dir.path().join("wfc.toml");
    assert!(config_path.exists(), "wfc.toml not created");

    let content = fs::read_to_string(&config_path).expect("failed to read config");
    assert!(
        content.contains("[stages"),
        "config missing [stages] section"
    );
}

#[test]
fn test_cli_dry_run() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let config_path = temp_dir.path().join("wfc.toml");

    fs::write(
        &config_path,
        r#"
[stages.echo]
command = ["echo", "hello"]
timeout = 10
"#,
    )
    .expect("failed to write config");

    let mut cmd = Command::cargo_bin("local-ci").expect("failed to find binary");
    cmd.current_dir(temp_dir.path());
    cmd.arg("--dry-run");
    cmd.arg("echo");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("echo").or(predicate::str::contains("Would run")));
}

#[test]
fn test_cli_json_output() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let config_path = temp_dir.path().join("wfc.toml");

    fs::write(
        &config_path,
        r#"
[stages.true]
command = ["true"]
timeout = 10
"#,
    )
    .expect("failed to write config");

    let mut cmd = Command::cargo_bin("local-ci").expect("failed to find binary");
    cmd.current_dir(temp_dir.path());
    cmd.arg("--json");
    cmd.arg("true");
    cmd.assert().success().stdout(
        predicate::str::contains("schema_version").and(predicate::str::contains("local-ci.result")),
    );
}

#[test]
fn test_cli_stage_selection() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let config_path = temp_dir.path().join("wfc.toml");

    fs::write(
        &config_path,
        r#"
[stages.fmt]
command = ["echo", "fmt"]
timeout = 10

[stages.test]
command = ["echo", "test"]
timeout = 10
"#,
    )
    .expect("failed to write config");

    // Test selecting specific stages
    let mut cmd = Command::cargo_bin("local-ci").expect("failed to find binary");
    cmd.current_dir(temp_dir.path());
    cmd.arg("--dry-run");
    cmd.arg("fmt");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("fmt"));
}

#[test]
fn test_cli_missing_config() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");

    let mut cmd = Command::cargo_bin("local-ci").expect("failed to find binary");
    cmd.current_dir(temp_dir.path());
    cmd.arg("--list");
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("config").or(predicate::str::contains("not found")));
}

#[test]
fn test_cli_unknown_stage() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let config_path = temp_dir.path().join("wfc.toml");

    fs::write(
        &config_path,
        r#"
[stages.fmt]
command = ["echo", "fmt"]
timeout = 10
"#,
    )
    .expect("failed to write config");

    let mut cmd = Command::cargo_bin("local-ci").expect("failed to find binary");
    cmd.current_dir(temp_dir.path());
    cmd.arg("unknown-stage");
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("unknown").or(predicate::str::contains("not found")));
}

#[test]
fn test_cli_deprecation_warning() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let config_path = temp_dir.path().join(".local-ci.toml");

    fs::write(
        &config_path,
        r#"
[stages.fmt]
command = ["echo", "fmt"]
timeout = 10
"#,
    )
    .expect("failed to write config");

    let mut cmd = Command::cargo_bin("local-ci").expect("failed to find binary");
    cmd.current_dir(temp_dir.path());
    cmd.arg("--list");
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("is deprecated"));
}

#[test]
fn test_cli_preference() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let wfc_config_path = temp_dir.path().join("wfc.toml");
    let local_config_path = temp_dir.path().join(".local-ci.toml");

    fs::write(
        &wfc_config_path,
        r#"
[stages.wfc-stage]
command = ["echo", "wfc"]
timeout = 10
"#,
    )
    .expect("failed to write wfc config");

    fs::write(
        &local_config_path,
        r#"
[stages.local-stage]
command = ["echo", "local"]
timeout = 10
"#,
    )
    .expect("failed to write local config");

    let mut cmd = Command::cargo_bin("local-ci").expect("failed to find binary");
    cmd.current_dir(temp_dir.path());
    cmd.arg("--list");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("wfc-stage"))
        .stdout(predicate::str::contains("local-stage").not())
        .stderr(predicate::str::contains("is deprecated").not());
}
