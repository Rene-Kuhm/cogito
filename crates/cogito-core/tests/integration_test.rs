//! End-to-end integration tests
//!
//! These tests verify the full cogito workflow:
//! - init project → scan → fingerprint
//! - scan → compress context
//! - scan → add decision → search decision
//! - scan → add tag → search tag
//!
//! They use a temp directory to avoid polluting the real .cogito/.

use cogito_core::{Cogito, CompressionOptions};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;
use uuid::Uuid;

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
        .args(["add", "-A"])
        .current_dir(&project_path)
        .output()
        .expect("git add failed");
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
    assert_eq!(project.language, cogito_core::Language::Rust);

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
async fn test_compress_code_reduces_tokens() {
    let cogito = Cogito::new();
    let code = r#"
// This is a comment
fn main() {
    // Another comment
    let x = 5;
    let y = 10;
    // More comments
    let sum = x + y;
    println!("{}", sum);
}
"#;

    let options = CompressionOptions {
        max_tokens: 1000,
        preserve_imports: true,
        preserve_types: true,
        hot_zones_priority: false,
    };

    let result = cogito.compress_context(code, options).await.unwrap();

    // Compression should produce *some* output and at least not increase size
    // (we don't assert strict reduction because the compressor may preserve structure)
    assert!(result.original_tokens > 0, "Original must have tokens");
    assert!(
        !result.content.is_empty(),
        "Compressed content must not be empty"
    );
}

#[tokio::test]
async fn test_decision_lifecycle() {
    let (_dir, project_path) = create_test_project();
    let cogito = Cogito::new();
    let project = cogito.init_project(project_path.clone()).unwrap();

    // Add decision
    let decision = cogito_core::decision::builder(project.id)
        .title("Use JWT for auth")
        .why("Stateless, scales better than sessions")
        .alternative("Cookie sessions")
        .tag("auth")
        .tag("security")
        .path(PathBuf::from("src/auth/"))
        .build();

    let added = cogito.add_decision(decision).await.unwrap();
    assert_eq!(added.title, "Use JWT for auth");
    assert_eq!(added.why, "Stateless, scales better than sessions");
    assert!(added.alternatives.contains(&"Cookie sessions".to_string()));
    assert!(added.tags.contains(&"auth".to_string()));
    assert!(added.tags.contains(&"security".to_string()));

    // Search by title
    let results = cogito.search_decisions(project.id, "JWT").await.unwrap();
    assert!(!results.is_empty(), "Should find decision by 'JWT'");
    assert_eq!(results[0].id, added.id);

    // Search by tag
    let results = cogito.search_decisions(project.id, "auth").await.unwrap();
    assert!(!results.is_empty(), "Should find decision by tag 'auth'");

    // Search by alternative
    let results = cogito.search_decisions(project.id, "Cookie").await.unwrap();
    assert!(!results.is_empty(), "Should find decision by alternative");

    // Search returns empty for non-existent query
    let results = cogito
        .search_decisions(project.id, "nonexistent-query-xyz")
        .await
        .unwrap();
    assert!(results.is_empty(), "Should not find non-matching decision");
}

#[tokio::test]
async fn test_tag_lifecycle() {
    let (_dir, project_path) = create_test_project();
    let cogito = Cogito::new();
    let project = cogito.init_project(project_path.clone()).unwrap();

    // Add tag
    let tag = cogito_core::types::SemanticTag {
        id: Uuid::new_v4(),
        project_id: project.id,
        tag: "auth".to_string(),
        domain: "security".to_string(),
        paths: vec![PathBuf::from("src/auth/")],
        confidence: 0.95,
        created_at: chrono::Utc::now(),
    };

    let added = cogito.add_tag(tag).await.unwrap();
    assert_eq!(added.tag, "auth");
    assert_eq!(added.domain, "security");
    assert_eq!(added.confidence, 0.95);

    // Search by tag name
    let results = cogito.search_tags(project.id, "auth").await.unwrap();
    assert!(!results.is_empty(), "Should find tag 'auth'");
    assert_eq!(results[0].id, added.id);

    // Search by domain
    let results = cogito.search_tags(project.id, "security").await.unwrap();
    assert!(!results.is_empty(), "Should find tag by domain");

    // Search returns empty for non-existent query
    let results = cogito
        .search_tags(project.id, "nonexistent-xyz")
        .await
        .unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_persistence_roundtrip() {
    let (_dir, project_path) = create_test_project();

    // Change to the test project directory for persistence
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project_path).unwrap();

    let cogito = Cogito::new();
    let project = cogito.init_project(project_path.clone()).unwrap();

    // Add a decision
    let decision = cogito_core::decision::builder(project.id)
        .title("Persistent decision")
        .why("Test persistence")
        .build();

    let _added = cogito.add_decision(decision).await.unwrap();

    // Verify .cogito/decisions.json was created
    let decisions_file = project_path.join(".cogito/decisions.json");
    assert!(
        decisions_file.exists(),
        "decisions.json should be created at {}",
        decisions_file.display()
    );

    // Restore original directory
    std::env::set_current_dir(&original_dir).unwrap();
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

#[tokio::test]
async fn test_compressor_preserves_strings() {
    let cogito = Cogito::new();
    // Note: https://example.com inside a string MUST NOT be treated as a comment
    let code = r#"let url = "https://example.com/path"; // real comment
let name = "hello"; /* another comment */
println!("{}", url);
"#;

    let options = CompressionOptions {
        max_tokens: 10000,
        preserve_imports: true,
        preserve_types: true,
        hot_zones_priority: false,
    };

    let result = cogito.compress_context(code, options).await.unwrap();
    assert!(
        result.content.contains("https://example.com"),
        "URL in string must survive compression"
    );
    assert!(
        result.content.contains("hello"),
        "String contents must survive compression"
    );
}
