//! Regression tests for multi-project isolation.
//!
//! Before this was fixed, the SQLite connection lived in a process-global
//! `OnceCell`: the first project opened by a process won, and every later
//! read/write silently targeted that first project's database. As an MCP
//! server, one process serves many projects, so this was a data-corruption
//! bug rather than a theoretical one.

use std::path::PathBuf;
use tempfile::TempDir;

/// Create a minimal Rust project with one source file.
fn make_project(marker: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_path_buf();
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::write(path.join("Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
    std::fs::write(
        path.join("src/main.rs"),
        format!("fn main() {{ /* {marker} */ }}\n"),
    )
    .unwrap();
    (dir, path)
}

/// Verify each project gets its own `.cogito` directory under its own root.
#[tokio::test]
async fn state_lives_under_the_project_directory() {
    let (_dir, path) = make_project("x");

    let cogito = cogito_harness_core::Cogito::for_project(&path).unwrap();

    // The instance must be bound to this project's `.cogito` directory,
    // independent of the process working directory.
    assert_eq!(cogito.cogito_dir(), path.join(".cogito"));

    // Touch the project to ensure the directory is initialised.
    let project = cogito.load_project(path.clone()).unwrap();

    // Scan the project so fingerprints are written.
    let _fp = cogito.generate_fingerprint(&project).await.unwrap();

    // The SQLite database must exist inside the project directory.
    assert!(
        path.join(".cogito/state.db").exists(),
        "state.db should be created under the project directory"
    );
}

/// Verify two distinct project roots yield two distinct project ids.
#[tokio::test]
async fn two_projects_have_distinct_ids() {
    let (_a_dir, a) = make_project("a");
    let (_b_dir, b) = make_project("b");

    // Cogito instances bound to each project's directory.
    let cogito_a = cogito_harness_core::Cogito::for_project(&a).unwrap();
    let cogito_b = cogito_harness_core::Cogito::for_project(&b).unwrap();

    let project_a = cogito_a.load_project(a.clone()).unwrap();
    let project_b = cogito_b.load_project(b.clone()).unwrap();

    // The two project roots must map to distinct ids.
    assert_ne!(
        project_a.id, project_b.id,
        "distinct project roots must yield distinct ids"
    );
}

/// Verify that fingerprint storage is isolated between projects:
///
/// - before a scan, both stored fingerprints are `None`
/// - after scanning project A, its stored fingerprint is `Some(_)`
///   while project B's stored fingerprint is still `None`
/// - after scanning project B, both stored fingerprints are non-`None`
#[tokio::test]
async fn stored_fingerprint_isolated_between_projects() {
    let (_a_dir, a) = make_project("a");
    let (_b_dir, b) = make_project("b");

    let cogito_a = cogito_harness_core::Cogito::for_project(&a).unwrap();
    let cogito_b = cogito_harness_core::Cogito::for_project(&b).unwrap();

    let project_a = cogito_a.load_project(a.clone()).unwrap();
    let project_b = cogito_b.load_project(b.clone()).unwrap();

    // Neither has a fingerprint yet.
    assert!(
        cogito_a.stored_fingerprint(project_a.id).unwrap().is_none(),
        "project A has no stored fingerprint before scan"
    );
    assert!(
        cogito_b.stored_fingerprint(project_b.id).unwrap().is_none(),
        "project B has no stored fingerprint before scan"
    );

    // Scan project A only.
    let fp_a = cogito_a.generate_fingerprint(&project_a).await.unwrap();

    // A now has a fingerprint; B must still be `None`.
    assert!(
        cogito_a.stored_fingerprint(project_a.id).unwrap().is_some(),
        "project A must have a fingerprint after scan"
    );
    assert!(
        cogito_b.stored_fingerprint(project_b.id).unwrap().is_none(),
        "project B must still have no fingerprint (isolation)"
    );

    // Verify the stored fingerprint matches what was generated.
    let stored = cogito_a.stored_fingerprint(project_a.id).unwrap().unwrap();
    assert_eq!(stored.head, fp_a.head);
    assert_eq!(stored.format, fp_a.format);

    // Scan project B.
    let fp_b = cogito_b.generate_fingerprint(&project_b).await.unwrap();

    // Both are now non-`None`.
    assert!(
        cogito_a.stored_fingerprint(project_a.id).unwrap().is_some(),
        "project A still has its fingerprint"
    );
    assert!(
        cogito_b.stored_fingerprint(project_b.id).unwrap().is_some(),
        "project B must have a fingerprint after scan"
    );

    // Each project's stored fingerprint was independently generated.
    let stored_a = cogito_a.stored_fingerprint(project_a.id).unwrap().unwrap();
    let stored_b = cogito_b.stored_fingerprint(project_b.id).unwrap().unwrap();
    assert_eq!(stored_a.head, fp_a.head);
    assert_eq!(stored_b.head, fp_b.head);
    assert_eq!(stored_a.format, fp_a.format);
    assert_eq!(stored_b.format, fp_b.format);
}
