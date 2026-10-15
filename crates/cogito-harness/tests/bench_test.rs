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

// ─────────────────────────────────────────────────────────────────────────────
// Track A3: quality, latency and cost reported separately, plus raw artifacts
// ─────────────────────────────────────────────────────────────────────────────

/// The fixture used by the report tests, with a fixed repository basename.
///
/// The project card names the repository, so a random temp directory name would
/// change the token counts and no golden could be compared against it.
fn setup_fixture() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = dir.path().join("bench-fixture");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"bench-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        repo.join("src/main.rs"),
        "/// Searchable function for benchmarking\npub fn search_code() {\n    println!(\"hello\");\n}\n\n/// Another searchable function\npub fn project_map() {\n    println!(\"world\");\n}\n",
    )
    .unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "test@test.com"],
        vec!["config", "user.name", "Test"],
        vec!["add", "-A"],
    ] {
        let status = Command::new("git")
            .args(&args)
            .current_dir(&repo)
            .output()
            .expect("git");
        assert!(status.status.success(), "git {args:?}");
    }
    // The project card reports `indexed at <short sha>`, so the token counts
    // depend on the commit. Pinning the dates and the identity makes the commit
    // — and therefore the golden file — reproducible.
    let status = Command::new("git")
        .args(["commit", "-m", "init", "--no-gpg-sign", "-q"])
        .current_dir(&repo)
        .env("GIT_AUTHOR_DATE", "2020-01-01T00:00:00+0000")
        .env("GIT_COMMITTER_DATE", "2020-01-01T00:00:00+0000")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .expect("git commit");
    assert!(status.status.success(), "git commit");
    (dir, repo)
}

/// A labelled gold set whose single repository is the fixture just created, so
/// the quality column can be exercised without the network or the pinned clones.
fn write_gold(dir: &std::path::Path, repo: &std::path::Path) -> PathBuf {
    let gold = serde_json::json!({
        "schema": "cogito.context_pack_gold/v1",
        "repos": [{"name": "fixture", "root": repo.display().to_string()}],
        "tasks": [
            {
                "id": "fx-01",
                "repo": "fixture",
                "language": "en",
                "budget": 2000,
                "task": "Where is search_code defined and what does it print?",
                "expect": {"symbols": ["search_code"], "files": ["src/main.rs"]}
            },
            {
                "id": "fx-02",
                "repo": "fixture",
                "language": "en",
                "budget": 2000,
                "task": "Where is project_map defined and what does it print?",
                "expect": {"symbols": ["project_map"]}
            }
        ]
    });
    let path = dir.join("gold.json");
    fs::write(&path, serde_json::to_string_pretty(&gold).unwrap()).unwrap();
    path
}

/// The one run directory a single invocation created below `root`.
fn run_dir(root: &std::path::Path) -> PathBuf {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root)
        .expect("artifact root")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert_eq!(
        dirs.len(),
        1,
        "one run should write exactly one dir: {dirs:?}"
    );
    dirs.pop().unwrap()
}

fn bench(repo: &std::path::Path, out: &std::path::Path, extra: &[&str]) -> std::process::Output {
    let mut command = Command::new(cogito_bin());
    command.args(["bench", "--repo"]).arg(repo);
    command.arg("--out-dir").arg(out);
    command.args(extra);
    command.output().expect("cogito bench")
}

/// Without the flag the output is exactly what it was before the report
/// existed, including the machine-readable form. The fixture is the one the
/// golden file was captured from, with the checkout path and its commit
/// normalised because both change per run.
#[test]
fn test_legacy_output_is_unchanged_without_the_flag() {
    let (_fixture, repo) = setup_fixture();
    let out = tempfile::TempDir::new().unwrap();
    let output = bench(
        &repo,
        out.path(),
        &["--terms", "search_code,project_map", "--json"],
    );
    assert!(output.status.success(), "bench failed");

    let mut produced: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("legacy JSON must stay valid");
    for entry in produced["repos"].as_array_mut().unwrap() {
        entry["repo"] = serde_json::json!("<REPO>");
        entry["commit"] = serde_json::json!("<COMMIT>");
    }
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/bench_legacy_output.json")).unwrap();
    assert_eq!(produced, expected, "the legacy JSON output changed");

    // The report sections must not appear without the flag either.
    let human = bench(&repo, out.path(), &["--terms", "search_code"]);
    let stdout = String::from_utf8_lossy(&human.stdout);
    for section in ["QUALITY", "LATENCY", "COST", "PROVENANCE"] {
        assert!(
            !stdout.contains(section),
            "{section} should only appear with --report:\n{stdout}"
        );
    }
    assert!(
        stdout.contains("bl_find"),
        "the old table should be unchanged"
    );
}

/// The report keeps the three classes apart: one section each, never one score.
#[test]
fn test_report_prints_quality_latency_and_cost_separately() {
    let (_fixture, repo) = setup_fixture();
    let out = tempfile::TempDir::new().unwrap();
    let gold = write_gold(out.path(), &repo);
    let output = bench(
        &repo,
        out.path(),
        &[
            "--terms",
            "search_code",
            "--report",
            "--gold",
            gold.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "bench failed");
    let stdout = String::from_utf8_lossy(&output.stdout);

    let quality = stdout.find("QUALITY").expect("a QUALITY section");
    let latency = stdout.find("LATENCY").expect("a LATENCY section");
    let cost = stdout.find("COST").expect("a COST section");
    assert!(
        quality < latency && latency < cost,
        "the three sections must be separate and ordered:\n{stdout}"
    );

    // Quality: recall@budget per task, with the expected columns.
    assert!(stdout.contains("recall@budget"), "{stdout}");
    assert!(
        stdout.contains("hits") && stdout.contains("expected"),
        "{stdout}"
    );
    assert!(
        stdout.contains("fx-01"),
        "the task id should be a row: {stdout}"
    );
    assert!(
        stdout.contains("100.0%"),
        "recall should be a percentage: {stdout}"
    );

    // Latency: index sync and query, separately.
    assert!(stdout.contains("index_sync_ms"), "{stdout}");
    assert!(stdout.contains("query_ms"), "{stdout}");

    // Cost: pack tokens and tool calls.
    assert!(stdout.contains("pack_tokens"), "{stdout}");
    assert!(stdout.contains("tool_calls"), "{stdout}");

    // Provenance: SHA, clean state, repetitions and tokenizer.
    assert!(stdout.contains("repetitions: 1"), "{stdout}");
    assert!(stdout.contains("tokenizer: cl100k_base"), "{stdout}");
    assert!(stdout.contains("worktree"), "{stdout}");
    assert!(
        stdout.contains("clean"),
        "the fixture worktree is clean: {stdout}"
    );

    // No combined score: the protocol is explicit that the three must not be
    // mixed into one number.
    assert!(
        !stdout.to_lowercase().contains("score"),
        "the report must not invent a combined score:\n{stdout}"
    );
}

/// A worktree with uncommitted work is marked, with the count, so the numbers
/// above it can be attributed to a revision plus a working tree.
#[test]
fn test_dirty_worktree_is_marked_in_the_report() {
    let (_fixture, repo) = setup_fixture();
    let out = tempfile::TempDir::new().unwrap();

    // One run first: it writes `.cogito/.gitignore`, which must stay invisible
    // to git, so the only change the report sees is the one added here.
    assert!(
        bench(&repo, out.path(), &["--terms", "search_code"])
            .status
            .success()
    );
    fs::write(repo.join("src/untracked_extra.rs"), "fn extra() {}\n").unwrap();

    let output = bench(&repo, out.path(), &["--terms", "search_code", "--report"]);
    assert!(output.status.success(), "bench failed");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The header row also carries the word `dirty`, so the data row is the one
    // that does not mention `worktree`.
    let row = stdout
        .lines()
        .find(|l| l.contains("dirty") && !l.contains("worktree"))
        .unwrap_or_else(|| panic!("an untracked file must mark the worktree dirty:\n{stdout}"));
    assert!(
        row.split_whitespace().any(|f| f == "1"),
        "the dirty row should count the one changed path: {row:?}\n{stdout}"
    );
}

/// Every run leaves an artifact outside the measured worktree, and a term that
/// matched nothing is in it. A benchmark that drops its zero results reports
/// only its good days.
#[test]
fn test_every_run_writes_a_raw_artifact_including_zero_results() {
    let (_fixture, repo) = setup_fixture();
    let out = tempfile::TempDir::new().unwrap();
    let output = bench(
        &repo,
        out.path(),
        &["--terms", "__NONEXISTENT_TERM_ABC123XYZ__", "--json"],
    );
    assert!(output.status.success(), "bench failed");

    let dir = run_dir(out.path());
    assert!(
        !dir.starts_with(&repo),
        "artifacts must live outside the measured worktree: {dir:?}"
    );
    let raw = fs::read_to_string(dir.join("run.json")).expect("run.json");
    let artifact: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(artifact["schema"], "cogito.bench-run/v1");
    assert_eq!(artifact["repetitions"], 1);
    let term = &artifact["runs"][0]["repos"][0]["terms"][0];
    assert_eq!(term["term"], "__NONEXISTENT_TERM_ABC123XYZ__");
    assert_eq!(
        term["no_matches"], true,
        "a zero-result term stays in the artifact: {raw}"
    );
    assert!(
        dir.join("report.txt").is_file(),
        "the rendered text is kept too"
    );
}

/// Repetitions are recorded, and every one of them is kept.
#[test]
fn test_repetitions_are_recorded_and_all_kept() {
    let (_fixture, repo) = setup_fixture();
    let out = tempfile::TempDir::new().unwrap();
    let output = bench(
        &repo,
        out.path(),
        &["--terms", "search_code", "--repeat", "2", "--json"],
    );
    assert!(output.status.success(), "bench failed");

    let raw = fs::read_to_string(run_dir(out.path()).join("run.json")).unwrap();
    let artifact: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(artifact["repetitions"], 2);
    assert_eq!(
        artifact["runs"].as_array().unwrap().len(),
        2,
        "both repetitions are kept: {raw}"
    );
}
