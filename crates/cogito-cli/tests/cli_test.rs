//! End-to-end CLI tests
//!
//! These tests invoke the compiled `cogito` binary and verify the full workflow
//! from the command line.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn setup_test_project() -> TempDir {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let project_path = dir.path();

    // Create minimal Rust project
    fs::write(
        project_path.join("Cargo.toml"),
        r#"[package]
name = "cli-test-project"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    fs::create_dir_all(project_path.join("src")).unwrap();
    fs::write(
        project_path.join("src/main.rs"),
        "fn main() { println!(\"hi\"); }",
    )
    .unwrap();

    // Init git
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(project_path)
        .output()
        .ok();
    Command::new("git")
        .args(["config", "user.email=test@test.com"])
        .current_dir(project_path)
        .output()
        .ok();
    Command::new("git")
        .args(["config", "user.name=Test"])
        .current_dir(project_path)
        .output()
        .ok();
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(project_path)
        .output()
        .ok();
    Command::new("git")
        .args(["commit", "-m", "init", "--no-gpg-sign", "-q"])
        .current_dir(project_path)
        .output()
        .ok();

    dir
}

fn cogito_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cogito"))
}

#[test]
fn test_cli_scan_in_real_project() {
    let project = setup_test_project();
    let output = Command::new(cogito_bin())
        .arg("scan")
        .current_dir(project.path())
        .output()
        .expect("Failed to run cogito scan");

    assert!(output.status.success(), "scan should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Entry Points") || stdout.contains("Entry points"),
        "Should show entry points. Got: {stdout}"
    );
    assert!(
        stdout.contains("Hot Zones") || stdout.contains("Hot zones"),
        "Should show hot zones. Got: {stdout}"
    );
}

#[test]
fn test_cli_decision_add_and_search() {
    let project = setup_test_project();
    let bin = cogito_bin();

    // Add a decision
    let output = Command::new(&bin)
        .args([
            "decision",
            "add",
            "--title",
            "Use JWT",
            "--why",
            "stateless auth",
            "--tags",
            "auth",
            "--paths",
            "src/auth.rs",
        ])
        .current_dir(project.path())
        .output()
        .expect("Failed to add decision");

    assert!(output.status.success(), "decision add should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Decision added"));

    // Search for the decision
    let output = Command::new(&bin)
        .args(["decision", "search", "JWT"])
        .current_dir(project.path())
        .output()
        .expect("Failed to search decisions");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Use JWT"), "Should find the decision");
    assert!(stdout.contains("stateless auth"));
}

#[test]
fn test_cli_tag_add_and_list() {
    let project = setup_test_project();
    let bin = cogito_bin();

    // Add a tag
    let output = Command::new(&bin)
        .args(["tag", "add", "billing", "src/billing/"])
        .current_dir(project.path())
        .output()
        .expect("Failed to add tag");

    assert!(output.status.success(), "tag add should succeed");

    // List tags
    let output = Command::new(&bin)
        .args(["tag", "list"])
        .current_dir(project.path())
        .output()
        .expect("Failed to list tags");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("billing"), "Should list the tag");
}

#[test]
fn test_cli_compress_via_stdin() {
    let bin = cogito_bin();
    let input = "fn main() { // comment\n    println!(\"hi\");\n}";

    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(&bin)
        .args(["compress", "-", "--max-tokens", "100"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to spawn cogito");

    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    stdin
        .write_all(input.as_bytes())
        .expect("Failed to write to stdin");
    drop(stdin);

    let output = child.wait_with_output().expect("Failed to read stdout");
    assert!(output.status.success(), "compress should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Original tokens"));
    assert!(stdout.contains("Compressed tokens"));
}

#[test]
fn test_cli_doctor_in_project() {
    let project = setup_test_project();
    let output = Command::new(cogito_bin())
        .arg("doctor")
        .current_dir(project.path())
        .output()
        .expect("Failed to run doctor");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Git:"), "Should detect git");
    assert!(stdout.contains("Cargo.toml"), "Should detect project file");
}

#[test]
fn test_cli_stats_shows_persistence() {
    let project = setup_test_project();
    let bin = cogito_bin();

    // Add a decision
    Command::new(&bin)
        .args(["decision", "add", "--title", "Test", "--why", "reason"])
        .current_dir(project.path())
        .output()
        .expect("Failed to add decision");

    // Add a tag
    Command::new(&bin)
        .args(["tag", "add", "test", "src/"])
        .current_dir(project.path())
        .output()
        .expect("Failed to add tag");

    // Check stats
    let output = Command::new(&bin)
        .args(["stats"])
        .current_dir(project.path())
        .output()
        .expect("Failed to run stats");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Tags:"));
    assert!(stdout.contains("Decisions:"));
    assert!(
        stdout.contains("1"),
        "Should show at least 1 tag and 1 decision"
    );
}

#[test]
fn test_cli_init() {
    let project = setup_test_project();
    let output = Command::new(cogito_bin())
        .arg("init")
        .current_dir(project.path())
        .output()
        .expect("Failed to init");

    assert!(output.status.success(), "init should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Project created") || stdout.contains("Project"));
}
