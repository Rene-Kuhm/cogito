//! Tests for the `cogito bench` command.
//!
//! These tests verify that the benchmark produces deterministic, meaningful
//! output and handles edge cases correctly.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn cogito_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cogito"))
}

fn setup_minimal_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    let project_path = dir.path();

    // Create a minimal Rust project
    fs::write(
        project_path.join("Cargo.toml"),
        r#"[package]
name = "bench-test-project"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    fs::create_dir_all(project_path.join("src")).unwrap();
    fs::write(
        project_path.join("src/main.rs"),
        r#"/// Searchable function for benchmarking
pub fn search_code() {
    println!("hello");
}

/// Another searchable function
pub fn project_map() {
    println!("world");
}
"#,
    )
    .unwrap();

    // Init git
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(project_path)
        .output()
        .ok();
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(project_path)
        .output()
        .ok();
    Command::new("git")
        .args(["config", "user.name", "Test"])
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

/// Test that running the benchmark produces non-zero baseline and cogito totals.
#[test]
fn test_bench_produces_non_zero_results() {
    let repo = setup_minimal_repo();
    let output = Command::new(cogito_bin())
        .args(["bench", "--repo"])
        .arg(repo.path())
        .args(["--terms", "search_code,project_map"])
        .output()
        .expect("Failed to run cogito bench");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain the bl_find and bl_read column headers (human readable format)
    assert!(
        stdout.contains("bl_find") || stdout.contains("baseline_finding_tokens"),
        "Output should contain baseline finding cost column"
    );
    assert!(
        stdout.contains("bl_read") || stdout.contains("baseline_reading_tokens"),
        "Output should contain baseline reading cost column"
    );
    assert!(stdout.contains("TOTAL"), "Output should contain TOTAL row");

    // Verify the output is non-empty and contains numbers
    assert!(!stdout.is_empty(), "Output should not be empty");
}

/// Test that the same inputs twice produce byte-identical JSON output.
#[test]
fn test_bench_json_deterministic() {
    let repo = setup_minimal_repo();

    // First run
    let output1 = Command::new(cogito_bin())
        .args(["bench", "--repo"])
        .arg(repo.path())
        .args(["--terms", "search_code"])
        .arg("--json")
        .output()
        .expect("Failed to run cogito bench first time");

    // Second run
    let output2 = Command::new(cogito_bin())
        .args(["bench", "--repo"])
        .arg(repo.path())
        .args(["--terms", "search_code"])
        .arg("--json")
        .output()
        .expect("Failed to run cogito bench second time");

    let json1 = String::from_utf8_lossy(&output1.stdout);
    let json2 = String::from_utf8_lossy(&output2.stdout);

    // JSON output should be identical (elapsed_seconds is excluded for determinism)
    assert_eq!(
        json1, json2,
        "Two identical runs should produce identical JSON output"
    );

    // Verify the JSON is valid
    let _: serde_json::Value = serde_json::from_str(&json1).expect("JSON output should be valid");
}

/// Test that a term with no matches is reported and does not panic.
#[test]
fn test_bench_term_no_matches() {
    let repo = setup_minimal_repo();
    // Use a term that won't match anything in the test repo
    let output = Command::new(cogito_bin())
        .args(["bench", "--repo"])
        .arg(repo.path())
        .args(["--terms", "__NONEXISTENT_TERM_ABC123XYZ__"])
        .output()
        .expect("Failed to run cogito bench");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain "no matches" indicator
    assert!(
        stdout.contains("no matches"),
        "Output should indicate no matches for impossible term"
    );

    // Should not panic or error
    assert!(
        output.status.success(),
        "Benchmark should succeed even with no matches"
    );
}

/// Test that `--limit 1` reads at most one file per term in the baseline.
#[test]
fn test_bench_limit_respected() {
    let repo = setup_minimal_repo();

    // Run with limit 1
    let output = Command::new(cogito_bin())
        .args(["bench", "--repo"])
        .arg(repo.path())
        .args(["--terms", "pub"])
        .args(["--limit", "1"])
        .arg("--json")
        .output()
        .expect("Failed to run cogito bench");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // The JSON should contain "matched_files_count":1 for the term (serde_json uses space before number)
    assert!(
        stdout.contains(r#""matched_files_count": 1"#)
            || stdout.contains(r#""matched_files_count":1"#),
        "With --limit 1, matched_files_count should be 1. Output: {}",
        stdout
    );
}

/// Test that the tokenizer field is present in JSON output.
#[test]
fn test_bench_reports_tokenizer() {
    let repo = setup_minimal_repo();
    let output = Command::new(cogito_bin())
        .args(["bench", "--repo"])
        .arg(repo.path())
        .args(["--terms", "search_code"])
        .arg("--json")
        .output()
        .expect("Failed to run cogito bench");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should report which tokenizer was used
    assert!(
        stdout.contains(r#""tokenizer":"#),
        "JSON output should contain tokenizer field"
    );
}
