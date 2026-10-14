//! End-to-end integration tests
//!
//! These tests verify the full cogito workflow:
//! - init project → scan → fingerprint
//!
//! They use a temp directory to avoid polluting the real .cogito/.

use cogito_harness_core::Cogito;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

fn create_test_project() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let project_path = dir.path().to_path_buf();

    // Create a minimal Rust project structure
    fs::write(
        project_path.join("Cargo.toml"),
        r#"[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    fs::create_dir_all(project_path.join("src")).unwrap();
    fs::write(
        project_path.join("src/main.rs"),
        r#"fn main() {
    println!("Hello, world!");
}

fn auth_check() -> bool {
    true
}

mod billing {
    pub fn calculate() -> u64 {
        100
    }
}
"#,
    )
    .unwrap();

    // Make it look like a real git repo so fingerprint works
    std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(&project_path)
        .output()
        .expect("git init failed");
    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&project_path)
        .output()
        .ok();
    std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&project_path)
        .output()
        .ok();
    std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(&project_path)
        .output()
        .expect("git add failed");
    std::process::Command::new("git")
        .args(["commit", "-m", "initial", "--no-gpg-sign", "-q"])
        .current_dir(&project_path)
        .output()
        .ok();

    (dir, project_path)
}

#[tokio::test]
async fn test_full_workflow_init_scan_fingerprint() {
    let (_dir, project_path) = create_test_project();
    let cogito = Cogito::new();

    // 1. Init project
    let project = cogito.init_project(project_path.clone()).unwrap();
    assert_eq!(
        project.name,
        project_path.file_name().unwrap().to_str().unwrap()
    );
    assert_eq!(project.language, cogito_harness_core::Language::Rust);

    // 2. Generate fingerprint
    let fingerprint = cogito.generate_fingerprint(&project).await.unwrap();
    assert!(
        !fingerprint.entry_points.is_empty(),
        "Should find entry points"
    );
    assert!(
        !fingerprint.entry_points.is_empty(),
        "Fingerprint must include entry points"
    );
}

#[tokio::test]
async fn test_fingerprint_with_multiple_commits() {
    let (_dir, project_path) = create_test_project();

    // Make a few more commits to test hot zone detection
    for i in 1..=3 {
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&project_path)
            .output()
            .ok();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&project_path)
            .output()
            .ok();
        fs::write(
            project_path.join("src/lib.rs"),
            format!("// commit {i}\npub fn f() {{}}\n"),
        )
        .unwrap();
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&project_path)
            .output()
            .ok();
        std::process::Command::new("git")
            .args([
                "commit",
                "-m",
                &format!("commit {i}"),
                "--no-gpg-sign",
                "-q",
            ])
            .current_dir(&project_path)
            .output()
            .ok();
    }

    let cogito = Cogito::new();
    let project = cogito.init_project(project_path).unwrap();
    let fingerprint = cogito.generate_fingerprint(&project).await.unwrap();

    // We have commits, so the fingerprint must include some zone
    let total_zones =
        fingerprint.hot_zones.len() + fingerprint.cold_zones.len() + fingerprint.frozen_zones.len();
    assert!(
        total_zones > 0,
        "Should have at least one zone from git history"
    );
}
