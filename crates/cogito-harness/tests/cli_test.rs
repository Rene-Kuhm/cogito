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
    // Use the legacy "key value" form which works on both old and new git
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
fn test_cli_doctor_in_project() {
    let project = setup_test_project();
    let output = Command::new(cogito_bin())
        .arg("doctor")
        .current_dir(project.path())
        // Keep the agent-integration section deterministic: no agents present.
        .env("COGITO_HOME", project.path().join("fake-home"))
        .output()
        .expect("Failed to run doctor");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("git"), "Should report git: {stdout}");
    assert!(
        stdout.contains("Rust"),
        "Should detect the language: {stdout}"
    );
    assert!(
        stdout.contains("Agent integrations"),
        "Should list agent integrations: {stdout}"
    );
}

#[test]
fn test_cli_stats_shows_persistence() {
    let project = setup_test_project();
    let bin = cogito_bin();

    // Check stats before any scan
    let output = Command::new(&bin)
        .args(["stats"])
        .current_dir(project.path())
        .output()
        .expect("Failed to run stats");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("not scanned yet"),
        "Should show not scanned yet: {stdout}"
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

// ============================================================
// Agent setup / uninstall (run against a throwaway HOME)
// ============================================================

/// Build a fake home with a Claude Code config that already has another MCP.
fn fake_home() -> TempDir {
    let home = TempDir::new().unwrap();
    fs::write(
        home.path().join(".claude.json"),
        r#"{"numStartups": 3, "mcpServers": {"other": {"command": "/x/other"}}}"#,
    )
    .unwrap();
    fs::create_dir_all(home.path().join(".codex")).unwrap();
    fs::write(
        home.path().join(".codex/config.toml"),
        "model = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"other\"\n",
    )
    .unwrap();
    home
}

#[test]
fn test_setup_configures_agents_and_preserves_existing_keys() {
    let home = fake_home();

    let output = Command::new(cogito_bin())
        .args(["setup", "claude-code", "codex"])
        .env("COGITO_HOME", home.path())
        .output()
        .expect("Failed to run setup");

    assert!(
        output.status.success(),
        "setup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Claude Code: cogito added, `other` and numStartups preserved
    let claude: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.path().join(".claude.json")).unwrap())
            .unwrap();
    assert_eq!(claude["numStartups"], 3);
    assert!(
        claude["mcpServers"]["other"].is_object(),
        "existing MCP kept"
    );
    assert_eq!(claude["mcpServers"]["cogito"]["args"][0], "mcp");

    // Codex: cogito added, model and `other` preserved
    let codex = fs::read_to_string(home.path().join(".codex/config.toml")).unwrap();
    assert!(codex.contains("model = \"gpt-5\""), "{codex}");
    assert!(codex.contains("[mcp_servers.other]"), "{codex}");
    assert!(codex.contains("mcp_servers.cogito"), "{codex}");

    // A backup of each touched file must exist
    let backups: Vec<_> = fs::read_dir(home.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("cogito-backup"))
        .collect();
    assert!(!backups.is_empty(), "expected a backup of the touched file");
}

#[test]
fn test_setup_is_idempotent() {
    let home = fake_home();
    let run = || {
        Command::new(cogito_bin())
            .args(["setup", "claude-code"])
            .env("COGITO_HOME", home.path())
            .output()
            .unwrap()
    };

    run();
    let after_first = fs::read_to_string(home.path().join(".claude.json")).unwrap();
    let second = run();
    assert!(second.status.success());

    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout.contains("already up to date"),
        "second run should be a no-op: {stdout}"
    );
    assert_eq!(
        after_first,
        fs::read_to_string(home.path().join(".claude.json")).unwrap(),
        "file must not change on a repeat run"
    );
}

#[test]
fn test_setup_dry_run_writes_nothing() {
    let home = fake_home();
    let before = fs::read_to_string(home.path().join(".claude.json")).unwrap();

    let output = Command::new(cogito_bin())
        .args(["setup", "claude-code", "--dry-run"])
        .env("COGITO_HOME", home.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        before,
        fs::read_to_string(home.path().join(".claude.json")).unwrap(),
        "dry run must not modify the file"
    );
    assert!(!home.path().join(".claude.json.cogito-backup-1").exists());
}

#[test]
fn test_uninstall_removes_only_cogito() {
    let home = fake_home();

    Command::new(cogito_bin())
        .args(["setup", "claude-code"])
        .env("COGITO_HOME", home.path())
        .output()
        .unwrap();

    let output = Command::new(cogito_bin())
        .args(["uninstall", "claude-code", "--yes"])
        .env("COGITO_HOME", home.path())
        .output()
        .unwrap();
    assert!(output.status.success());

    let claude: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.path().join(".claude.json")).unwrap())
            .unwrap();
    assert!(claude["mcpServers"]["cogito"].is_null(), "cogito removed");
    assert!(claude["mcpServers"]["other"].is_object(), "other kept");
    assert_eq!(claude["numStartups"], 3);
}

#[test]
fn test_doctor_reports_agent_status() {
    let home = fake_home();
    Command::new(cogito_bin())
        .args(["setup", "claude-code"])
        .env("COGITO_HOME", home.path())
        .output()
        .unwrap();

    let output = Command::new(cogito_bin())
        .arg("doctor")
        .env("COGITO_HOME", home.path())
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Claude Code"), "{stdout}");
    assert!(stdout.contains("cogito configured"), "{stdout}");
    // Codex is present but was never configured.
    assert!(stdout.contains("installed, cogito missing"), "{stdout}");
}

#[test]
fn test_stale_entry_is_reported_and_repaired_by_setup() {
    let home = TempDir::new().unwrap();
    // An entry as an older Cogito would have written it: stale path, missing
    // `exposure`, and the Engram-only `lifecycle` field.
    fs::create_dir_all(home.path().join(".gentle-shell/agent")).unwrap();
    fs::write(
        home.path().join(".gentle-shell/agent/settings.json"),
        r#"{"packages":["npm:cogito-harness"]}"#,
    )
    .unwrap();
    fs::write(
        home.path().join(".gentle-shell/agent/mcp.json"),
        r#"{"mcpServers":{"cogito":{"command":"/old/cogito","args":["mcp"],"lifecycle":"lazy"}}}"#,
    )
    .unwrap();

    // doctor must not claim a stale entry is fine
    let output = Command::new(cogito_bin())
        .arg("doctor")
        .env("COGITO_HOME", home.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("configured but outdated"),
        "stale entry must be flagged: {stdout}"
    );
    assert!(
        stdout.contains("Re-run `cogito setup`"),
        "doctor must say how to fix it: {stdout}"
    );

    // setup repairs it
    let output = Command::new(cogito_bin())
        .args(["setup", "gentle-shell"])
        .env("COGITO_HOME", home.path())
        .output()
        .unwrap();
    assert!(output.status.success());

    let mcp: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(home.path().join(".gentle-shell/agent/mcp.json")).unwrap(),
    )
    .unwrap();
    let entry = &mcp["mcpServers"]["cogito"];
    assert_eq!(entry["exposure"], "direct", "field added: {entry}");
    assert!(
        entry.get("lifecycle").is_none(),
        "stale field removed: {entry}"
    );
    assert_ne!(entry["command"], "/old/cogito", "path refreshed: {entry}");

    // and doctor is happy afterwards
    let output = Command::new(cogito_bin())
        .arg("doctor")
        .env("COGITO_HOME", home.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cogito configured"), "{stdout}");
    assert!(!stdout.contains("configured but outdated"), "{stdout}");
}

#[test]
fn test_setup_accepts_yes_flag() {
    let home = fake_home();
    let output = Command::new(cogito_bin())
        .args(["setup", "-y"])
        .env("COGITO_HOME", home.path())
        .output()
        .expect("setup -y must be accepted");
    assert!(
        output.status.success(),
        "setup -y failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // It must actually configure, not silently no-op.
    let claude: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.path().join(".claude.json")).unwrap())
            .unwrap();
    assert!(claude["mcpServers"]["cogito"].is_object());
}

#[test]
#[cfg(unix)]
fn test_claude_hook_prints_the_project_card() {
    // What Claude Code runs at session start: the installed hook script, in the
    // project directory, with `cogito` on PATH. Its output becomes context.
    let home = fake_home();
    let project = setup_test_project();
    let setup = Command::new(cogito_bin())
        .args(["setup", "claude-code"])
        .env("COGITO_HOME", home.path())
        .output()
        .expect("Failed to run setup");
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );

    let script = home.path().join(".claude/hooks/cogito-session-start.sh");
    let bin_dir = cogito_bin().parent().unwrap().to_path_buf();
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = Command::new("sh")
        .arg(&script)
        .env("PATH", path)
        .env("CLAUDE_PROJECT_DIR", project.path())
        .current_dir(home.path())
        .output()
        .expect("Failed to run the hook");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("<cogito_project>\n"), "{stdout}");
    assert!(stdout.contains("code files"), "{stdout}");
    assert!(stdout.contains("search_code"), "{stdout}");
    assert!(stdout.trim_end().ends_with("</cogito_project>"), "{stdout}");

    // Outside a project the hook prints nothing.
    let elsewhere = TempDir::new().unwrap();
    let output = Command::new("sh")
        .arg(&script)
        .env("CLAUDE_PROJECT_DIR", elsewhere.path())
        .output()
        .expect("Failed to run the hook");
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}

/// Run `cogito hook claude-pre-tool-use` with `event` on stdin.
fn claude_guard(project: &std::path::Path, event: &serde_json::Value) -> (i32, String) {
    use std::io::Write;
    let mut child = Command::new(cogito_bin())
        .args(["hook", "claude-pre-tool-use"])
        .env("CLAUDE_PROJECT_DIR", project)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn hook");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(event.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn test_claude_guardrail_outlines_a_large_read_once() {
    let project = setup_test_project();
    let big: String = (0..300).map(|i| format!("pub fn f{i}() {{}}\n")).collect();
    fs::write(project.path().join("src/big.rs"), big).unwrap();
    let file = project.path().join("src/big.rs");

    let whole = serde_json::json!({
        "session_id": "t1",
        "tool_name": "Read",
        "tool_input": { "file_path": file },
    });
    let (code, stderr) = claude_guard(project.path(), &whole);
    assert_eq!(code, 2, "first whole read is blocked: {stderr}");
    assert!(stderr.contains("src/big.rs has 300 lines"), "{stderr}");
    assert!(
        stderr.contains("1 pub fn f0()"),
        "outline in the message: {stderr}"
    );

    let (code, _) = claude_guard(project.path(), &whole);
    assert_eq!(code, 0, "the same read again goes through");

    let ranged = serde_json::json!({
        "session_id": "t2",
        "tool_name": "Read",
        "tool_input": { "file_path": file, "offset": 10, "limit": 20 },
    });
    assert_eq!(
        claude_guard(project.path(), &ranged).0,
        0,
        "ranged reads are free"
    );

    let regex = serde_json::json!({
        "session_id": "t2",
        "tool_name": "Grep",
        "tool_input": { "pattern": "fn\\s+f1" },
    });
    assert_eq!(claude_guard(project.path(), &regex).0, 0, "regex greps run");

    // `.cogito/` never shows up in git.
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(project.path())
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&status.stdout).contains(".cogito"),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
}
