//! Tests for the file and symbol index.
//!
//! Uses tempfile to build real project directories and asserts through the
//! public `index` API.

use cogito_harness_core::Cogito;
use cogito_harness_core::index;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use uuid::Uuid;

/// Create a minimal Rust project in temp dir.
fn rust_project() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(
        src.join("lib.rs"),
        r#"
/// Adds two numbers.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// Subtracts two numbers.
pub fn subtract(a: i32, b: i32) -> i32 {
    a - b
}
"#,
    )
    .unwrap();
    // Add Cargo.toml
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let root = tmp.path().to_path_buf();
    (tmp, root)
}

fn init_cogito(root: &Path) -> (Cogito, Uuid) {
    let cogito = Cogito::with_project_sqlite(&root.join(".cogito")).unwrap();
    let project = cogito.init_project(root.to_path_buf()).unwrap();
    cogito.register_project(&project).unwrap();
    (cogito, project.id)
}

/// Test: a file created after a first sync is found by search after a second
/// sync, with no git commit anywhere in the test.
#[test]
fn test_new_file_appears_after_sync() {
    let (tmp, root) = rust_project();
    let (cogito, project_id) = init_cogito(&root);

    let conn_guard = cogito.sqlite().unwrap().lock();

    // First sync
    let report1 = index::sync(&conn_guard, project_id, &root).unwrap();
    assert!(
        report1.added >= 1,
        "Should add at least Cargo.toml and lib.rs"
    );
    assert_eq!(report1.removed, 0);

    // Create a new file (no git commit)
    let new_file = root.join("src").join("math.rs");
    fs::write(
        &new_file,
        r#"
/// Multiplies two numbers.
pub fn multiply(a: i32, b: i32) -> i32 {
    a * b
}
"#,
    )
    .unwrap();

    // Search before second sync - should not find the new file's symbol
    let hits_before = index::search(&conn_guard, project_id, "multiply", 10).unwrap();
    assert!(
        !hits_before.iter().any(|h| h.name == "multiply"),
        "Should not find 'multiply' before second sync"
    );

    // Second sync
    let report2 = index::sync(&conn_guard, project_id, &root).unwrap();
    assert_eq!(report2.added, 1, "Should add the new file");
    assert_eq!(report2.removed, 0);

    // Search after second sync - should find the new symbol
    let hits_after = index::search(&conn_guard, project_id, "multiply", 10).unwrap();
    assert!(
        hits_after.iter().any(|h| h.name == "multiply"),
        "Should find 'multiply' after second sync: {:?}",
        hits_after
    );

    drop(conn_guard);
    drop(tmp);
}

/// Test: editing a file so a symbol changes triggers reparse, and the new
/// declaration is found while the old one is gone.
#[test]
fn test_symbol_change_triggers_reparse() {
    let (tmp, root) = rust_project();
    let (cogito, project_id) = init_cogito(&root);

    let conn_guard = cogito.sqlite().unwrap().lock();

    // Initial sync
    index::sync(&conn_guard, project_id, &root).unwrap();

    // Verify initial symbol exists
    let outline_before =
        index::outline(&conn_guard, project_id, &PathBuf::from("src/lib.rs")).unwrap();
    assert!(
        outline_before.iter().any(|d| d.name == "add"),
        "Should have 'add' before edit"
    );

    // Edit the file to rename add -> add_numbers
    fs::write(
        root.join("src/lib.rs"),
        r#"
/// Adds two numbers.
pub fn add_numbers(a: i32, b: i32) -> i32 {
    a + b
}

/// Subtracts two numbers.
pub fn subtract(a: i32, b: i32) -> i32 {
    a - b
}
"#,
    )
    .unwrap();

    // Sync again
    let report = index::sync(&conn_guard, project_id, &root).unwrap();
    assert_eq!(report.modified, 1, "Should modify src/lib.rs");

    // Verify new symbol exists and old one is gone
    let outline_after =
        index::outline(&conn_guard, project_id, &PathBuf::from("src/lib.rs")).unwrap();
    assert!(
        !outline_after.iter().any(|d| d.name == "add"),
        "'add' should be gone after rename"
    );
    assert!(
        outline_after.iter().any(|d| d.name == "add_numbers"),
        "'add_numbers' should exist after rename"
    );

    // Verify no duplicate rows
    let count = outline_after
        .iter()
        .filter(|d| d.name == "add_numbers")
        .count();
    assert_eq!(count, 1, "Should have exactly one 'add_numbers'");

    drop(conn_guard);
    drop(tmp);
}

/// Test: a file rewritten with identical bytes but a newer mtime is reported
/// as unchanged content and produces no symbol churn.
#[test]
fn test_same_bytes_newer_mtime_unchanged() {
    let (tmp, root) = rust_project();
    let (cogito, project_id) = init_cogito(&root);

    let conn_guard = cogito.sqlite().unwrap().lock();

    // Initial sync
    let report1 = index::sync(&conn_guard, project_id, &root).unwrap();
    let _initial_reparsed = report1.reparsed;

    // Wait a tiny bit, then rewrite with same content
    std::thread::sleep(std::time::Duration::from_millis(10));
    let content = fs::read(root.join("src/lib.rs")).unwrap();
    fs::write(root.join("src/lib.rs"), &content).unwrap();

    // Touch the Cargo.toml too
    fs::write(
        root.join("Cargo.toml"),
        fs::read(root.join("Cargo.toml")).unwrap(),
    )
    .unwrap();

    // Sync again
    let report2 = index::sync(&conn_guard, project_id, &root).unwrap();
    assert_eq!(
        report2.reparsed, 0,
        "Should not reparse files with identical content"
    );
    assert_eq!(
        report2.unchanged, report1.scanned,
        "All files should be unchanged"
    );

    drop(conn_guard);
    drop(tmp);
}

/// Test: deleting a file removes it from search, outline, and imported_files.
#[test]
fn test_deleted_file_removed() {
    let (tmp, root) = rust_project();
    let (cogito, project_id) = init_cogito(&root);

    let conn_guard = cogito.sqlite().unwrap().lock();

    // Initial sync
    index::sync(&conn_guard, project_id, &root).unwrap();

    // Verify lib.rs is indexed
    let outline_before =
        index::outline(&conn_guard, project_id, &PathBuf::from("src/lib.rs")).unwrap();
    assert!(!outline_before.is_empty(), "lib.rs should be indexed");

    // Delete the file
    fs::remove_file(root.join("src/lib.rs")).unwrap();

    // Sync
    let report = index::sync(&conn_guard, project_id, &root).unwrap();
    assert_eq!(report.removed, 1, "Should remove deleted file");

    // Verify it's gone from search
    let hits = index::search(&conn_guard, project_id, "add", 10).unwrap();
    assert!(
        !hits
            .iter()
            .any(|h| h.path.to_string_lossy().contains("lib.rs")),
        "'add' from lib.rs should not be found"
    );

    // Verify outline returns empty
    let outline_after =
        index::outline(&conn_guard, project_id, &PathBuf::from("src/lib.rs")).unwrap();
    assert!(
        outline_after.is_empty(),
        "Deleted file should have no outline"
    );

    drop(conn_guard);
    drop(tmp);
}

/// Test: a second sync on an untouched tree reports 0 for all change counts.
#[test]
fn test_idempotent_sync() {
    let (tmp, root) = rust_project();
    let (cogito, project_id) = init_cogito(&root);

    let conn_guard = cogito.sqlite().unwrap().lock();

    // First sync
    let report1 = index::sync(&conn_guard, project_id, &root).unwrap();
    let first_scanned = report1.scanned;

    // Second sync (tree unchanged)
    let report2 = index::sync(&conn_guard, project_id, &root).unwrap();
    assert_eq!(report2.added, 0, "Nothing added");
    assert_eq!(report2.modified, 0, "Nothing modified");
    assert_eq!(report2.removed, 0, "Nothing removed");
    assert_eq!(report2.reparsed, 0, "Nothing reparsed");
    assert_eq!(
        report2.scanned, first_scanned,
        "Same number of files scanned"
    );

    drop(conn_guard);
    drop(tmp);
}

/// Test: FTS5 query finds camelCase and snake_case identifiers.
#[test]
fn test_fts5_multilingual_query() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();

    // Create files with different naming conventions
    fs::write(
        src.join("camel_case.rs"),
        r#"
pub struct CamelCaseName {
    pub first_field: i32,
}

pub fn getCamelCaseValue() -> i32 {
    42
}
"#,
    )
    .unwrap();

    fs::write(
        src.join("snake_case.py"),
        r#"
def get_snake_case_value():
    return 42

class SnakeCaseName:
    pass
"#,
    )
    .unwrap();

    let (cogito, project_id) = init_cogito(tmp.path());
    let conn_guard = cogito.sqlite().unwrap().lock();

    index::sync(&conn_guard, project_id, tmp.path()).unwrap();

    // Search for partial identifier
    let hits = index::search(&conn_guard, project_id, "CamelCase", 10).unwrap();
    assert!(
        hits.iter().any(|h| h.name.contains("CamelCase")),
        "Should find CamelCase: {:?}",
        hits
    );

    let hits = index::search(&conn_guard, project_id, "snake_case", 10).unwrap();
    assert!(
        hits.iter().any(|h| h.name.contains("snake_case")),
        "Should find snake_case: {:?}",
        hits
    );

    // Query with no match returns empty
    let hits = index::search(&conn_guard, project_id, "nonexistent_xyz_123", 10).unwrap();
    assert!(hits.is_empty(), "No match should return empty: {:?}", hits);

    drop(conn_guard);
    drop(tmp);
}

/// Test: a documentation file is indexed and its heading is findable by search.
#[test]
fn test_doc_file_indexed() {
    let tmp = TempDir::new().unwrap();
    let docs = tmp.path().join("docs");
    fs::create_dir(&docs).unwrap();

    fs::write(
        docs.join("guide.md"),
        "# User Guide\n\n## Getting Started\n\nWelcome to the guide.\n\n## Configuration\n\nConfigure your settings here.",
    )
    .unwrap();

    let (cogito, project_id) = init_cogito(tmp.path());
    let conn_guard = cogito.sqlite().unwrap().lock();

    let report = index::sync(&conn_guard, project_id, tmp.path()).unwrap();
    assert!(report.added >= 1, "Should add docs/guide.md");

    // Search for heading
    let hits = index::search(&conn_guard, project_id, "Getting Started", 10).unwrap();
    assert!(
        hits.iter()
            .any(|h| h.kind == "symbol" && h.name.contains("Getting Started")),
        "Should find heading 'Getting Started': {:?}",
        hits
    );

    // Search for filename
    let hits = index::search(&conn_guard, project_id, "guide", 10).unwrap();
    assert!(
        hits.iter().any(|h| h.kind == "doc" && h.name == "guide.md"),
        "Should find doc file 'guide.md': {:?}",
        hits
    );

    drop(conn_guard);
    drop(tmp);
}

/// Integration test: on a git repository, create a file, run
/// Cogito::fingerprint(project, false) and assert the new file is present
/// without refresh.
#[tokio::test]
async fn test_fingerprint_sees_working_tree_change() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Initialize a git repo and create initial commit
    let repo = git2::Repository::init(root).unwrap();
    fs::write(root.join("README.md"), "# Test Project").unwrap();

    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("README.md")).unwrap();
    index.write().unwrap();

    let sig = git2::Signature::now("test", "test@example.com").unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();

    // Create an empty commit (no parents for initial commit)
    repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
        .ok();

    let cogito = Cogito::with_project_sqlite(&root.join(".cogito")).unwrap();
    let project = cogito.init_project(root.to_path_buf()).unwrap();
    cogito.register_project(&project).ok();

    // Generate initial fingerprint
    let (_fp1, scanned1) = cogito.fingerprint(&project, false).await.unwrap();
    assert!(scanned1, "First call should scan");

    // Create a new file (not committed). The directory has to exist first:
    // `fs::write` does not create parents.
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        r#"
/// New module.
pub fn new_function() {}
"#,
    )
    .unwrap();

    // Get fingerprint again without refresh - should see the new file
    let (fp2, scanned2) = cogito.fingerprint(&project, false).await.unwrap();
    assert!(
        scanned2,
        "Should re-scan after working-tree change (no refresh flag)"
    );

    // Verify the new file's symbol is in the fingerprint
    assert!(
        fp2.symbols.iter().any(|s| s.name == "new_function"),
        "Should have new_function in fingerprint symbols"
    );

    drop(tmp);
}
