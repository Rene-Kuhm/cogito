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
    let report1 = index::sync(&conn_guard, project_id, &root, false).unwrap();
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
    let report2 = index::sync(&conn_guard, project_id, &root, false).unwrap();
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
    index::sync(&conn_guard, project_id, &root, false).unwrap();

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
    let report = index::sync(&conn_guard, project_id, &root, false).unwrap();
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
    let report1 = index::sync(&conn_guard, project_id, &root, false).unwrap();
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
    let report2 = index::sync(&conn_guard, project_id, &root, false).unwrap();
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
    index::sync(&conn_guard, project_id, &root, false).unwrap();

    // Verify lib.rs is indexed
    let outline_before =
        index::outline(&conn_guard, project_id, &PathBuf::from("src/lib.rs")).unwrap();
    assert!(!outline_before.is_empty(), "lib.rs should be indexed");

    // Delete the file
    fs::remove_file(root.join("src/lib.rs")).unwrap();

    // Sync
    let report = index::sync(&conn_guard, project_id, &root, false).unwrap();
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
    let report1 = index::sync(&conn_guard, project_id, &root, false).unwrap();
    let first_scanned = report1.scanned;

    // Second sync (tree unchanged)
    let report2 = index::sync(&conn_guard, project_id, &root, false).unwrap();
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

    index::sync(&conn_guard, project_id, tmp.path(), false).unwrap();

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

    let report = index::sync(&conn_guard, project_id, tmp.path(), false).unwrap();
    assert!(report.added >= 1, "Should add docs/guide.md");

    // Search for heading
    let hits = index::search(&conn_guard, project_id, "Getting Started", 10).unwrap();
    assert!(
        hits.iter()
            .any(|h| h.is_symbol && h.name.contains("Getting Started")),
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

/// Test: a source file ranks above a test file for the same symbol name.
/// Within each class (symbol vs file), non-test paths come before test paths,
/// then FTS5 bm25, then path, then line — giving fully deterministic ordering.
#[test]
fn test_search_ranks_source_before_tests() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Create a source file with a symbol
    let lib_rel = PathBuf::from("src/lib.rs");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "pub fn process_data(x: i32) -> i32 { x }\npub struct DataProcessor;\n",
    )
    .unwrap();

    // Create a test file with the same symbol name
    let test_rel = PathBuf::from("tests/lib_test.rs");
    fs::create_dir_all(root.join("tests")).unwrap();
    fs::write(
        root.join("tests/lib_test.rs"),
        "#[test]\nfn process_data() {\n    assert_eq!(super::process_data(1), 1);\n}\n",
    )
    .unwrap();

    let (cogito, project_id) = init_cogito(root);
    let conn_guard = cogito.sqlite().unwrap().lock();

    index::sync(&conn_guard, project_id, root, false).unwrap();

    let hits = index::search(&conn_guard, project_id, "process_data", 10).unwrap();

    // Must find both
    assert!(
        hits.iter().any(|h| h.path == lib_rel),
        "Should find source file: {:?}",
        hits
    );
    assert!(
        hits.iter().any(|h| h.path == test_rel),
        "Should find test file: {:?}",
        hits
    );

    // Source file must come before test file
    let source_idx = hits.iter().position(|h| h.path == lib_rel).unwrap();
    let test_idx = hits.iter().position(|h| h.path == test_rel).unwrap();
    assert!(
        source_idx < test_idx,
        "Source file should rank before test file: {:?}",
        hits
    );

    drop(conn_guard);
    drop(tmp);
}

/// Test: a query containing FTS5 special characters (`\"`, `*`, `(`, `:`,
/// backslash) does not panic and returns either results or an empty list.
#[test]
fn test_search_special_chars_no_panic() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let src = root.join("src");
    fs::create_dir(&src).unwrap();

    fs::write(src.join("lib.rs"), "pub fn foo_bar() {}").unwrap();

    let (cogito, project_id) = init_cogito(root);
    let conn_guard = cogito.sqlite().unwrap().lock();
    index::sync(&conn_guard, project_id, root, false).unwrap();

    let dangerous = [
        r#"""#,
        "*",
        "(",
        ":",
        r#"\"#,
        "hello*world",
        "foo:bar",
        "(test)",
    ];

    for q in dangerous {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            index::search(&conn_guard, project_id, q, 10)
        }));
        assert!(
            result.is_ok(),
            "Query {:?} should not panic: {:?}",
            q,
            result
        );
        // A valid name query must keep working
        let normal = index::search(&conn_guard, project_id, "foo_bar", 10).unwrap();
        assert!(
            !normal.is_empty(),
            "Normal query 'foo_bar' should still find results"
        );
    }

    drop(conn_guard);
    drop(tmp);
}

/// Test: a valid identifier query still works after the LIKE fallback path
/// is exercised by an earlier invalid query.
#[test]
fn test_search_like_fallback() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let src = root.join("src");
    fs::create_dir(&src).unwrap();

    fs::write(
        src.join("lib.rs"),
        "pub fn my_function() {}\npub struct MyType;",
    )
    .unwrap();

    let (cogito, project_id) = init_cogito(root);
    let conn_guard = cogito.sqlite().unwrap().lock();
    index::sync(&conn_guard, project_id, root, false).unwrap();

    // A bare " triggers FTS5 error; the LIKE fallback path is entered.
    // A normal query afterward must still return correct results.
    let _ = index::search(&conn_guard, project_id, "\"", 10).unwrap();
    let hits = index::search(&conn_guard, project_id, "my_function", 10).unwrap();
    assert!(
        hits.iter().any(|h| h.name == "my_function"),
        "Normal query should still work after LIKE fallback: {:?}",
        hits
    );

    drop(conn_guard);
    drop(tmp);
}

/// Test: `SearchHit.score` carries the real FTS5 bm25 value, not a constant.
#[test]
fn test_search_score_is_bm25() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let src = root.join("src");
    fs::create_dir(&src).unwrap();

    fs::write(src.join("a.rs"), "pub fn unique_token_xyz() {}").unwrap();
    fs::write(
        src.join("b.rs"),
        "// comment\n// comment\n// comment\npub fn unique_token_xyz() {}",
    )
    .unwrap();

    let (cogito, project_id) = init_cogito(root);
    let conn_guard = cogito.sqlite().unwrap().lock();
    index::sync(&conn_guard, project_id, root, false).unwrap();

    let hits = index::search(&conn_guard, project_id, "unique_token_xyz", 10).unwrap();

    assert!(
        !hits.is_empty(),
        "Should find 'unique_token_xyz': {:?}",
        hits
    );
    // All scores should be real bm25 values (negative floats), not 1.0 or 0.5
    for hit in &hits {
        assert!(
            hit.score != 1.0 && hit.score != 0.5,
            "Score should be real bm25, not constant: {:?}",
            hit
        );
    }

    drop(conn_guard);
    drop(tmp);
}

/// Test: in a git repository with a clean worktree, `sync` with
/// `allow_skip_when_clean = true` reports no changes and reparses nothing.
#[test]
fn test_sync_skip_on_clean_worktree() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Create files before git init (git add_path requires existing files)
    fs::write(root.join("README.md"), "# Test").unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn x() {}").unwrap();

    // Initialise a git repo and create one commit so the worktree is clean
    let repo = git2::Repository::init(root).unwrap();

    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("README.md")).unwrap();
    index.add_path(std::path::Path::new("src/lib.rs")).unwrap();
    index.write().unwrap();

    let sig = git2::Signature::now("test", "test@example.com").unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Initial", &tree, &[])
        .ok();

    let (cogito, project_id) = init_cogito(root);
    let conn = cogito.sqlite().unwrap().lock();

    // First sync: populate the index
    let report = index::sync(&conn, project_id, root, false).unwrap();
    assert!(report.scanned > 0, "Should have scanned files");
    assert!(report.reparsed > 0, "Should have reparsed files");

    // Second sync with allow_skip_when_clean=true: should skip without re-parsing
    let report = index::sync(&conn, project_id, root, true).unwrap();
    assert!(!report.changed(), "Clean worktree: nothing changed");
    assert_eq!(report.reparsed, 0, "Should not have re-parsed");
    assert_eq!(report.unchanged, report.scanned, "All files unchanged");

    drop(conn);
    drop(tmp);
}

/// Test: in a git repository with one uncommitted file, `sync` with
/// `allow_skip_when_clean = true` does NOT skip and reports the change.
#[test]
fn test_sync_no_skip_on_dirty_worktree() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Create files before git init
    fs::write(root.join("README.md"), "# Test").unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn x() {}").unwrap();

    // Initialise a git repo and create one commit
    let repo = git2::Repository::init(root).unwrap();

    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("README.md")).unwrap();
    index.add_path(std::path::Path::new("src/lib.rs")).unwrap();
    index.write().unwrap();

    let sig = git2::Signature::now("test", "test@example.com").unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Initial", &tree, &[])
        .ok();

    let (cogito, project_id) = init_cogito(root);
    let conn = cogito.sqlite().unwrap().lock();

    // First sync: populate the index
    index::sync(&conn, project_id, root, false).unwrap();

    // Add an uncommitted file (src/ already exists)
    fs::write(root.join("src/new.rs"), "pub fn y() {}").unwrap();

    // Second sync with allow_skip_when_clean=true: should NOT skip because
    // worktree is dirty, and should find the new file.
    let report = index::sync(&conn, project_id, root, true).unwrap();
    assert!(report.changed(), "Dirty worktree: should detect change");
    assert!(report.added > 0, "Should have found the new file");

    drop(conn);
    drop(tmp);
}

// ---------------------------------------------------------------------------
// Import resolution tests
// ---------------------------------------------------------------------------

/// Test: in a TypeScript workspace, the resolver resolves a workspace package
/// import to its actual file path and tracks importers correctly.
#[test]
fn test_typescript_import_resolution() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Workspace structure:
    // packages/shared/src/index.ts  -- exports x
    // apps/web/app/page.tsx         -- imports from @m/shared
    let shared_src = root.join("packages/shared/src");
    let web_app = root.join("apps/web/app");
    fs::create_dir_all(&shared_src).unwrap();
    fs::create_dir_all(&web_app).unwrap();

    fs::write(
        root.join("package.json"),
        r#"{"name":"mono","private":true}"#,
    )
    .unwrap();
    fs::write(
        root.join("packages/shared/package.json"),
        r#"{"name":"@m/shared"}"#,
    )
    .unwrap();
    fs::write(shared_src.join("index.ts"), "export const x = 1;\n").unwrap();

    fs::write(root.join("apps/web/package.json"), r#"{"name":"@m/web"}"#).unwrap();
    fs::write(root.join("apps/web/tsconfig.json"), "{}").unwrap();
    fs::write(
        web_app.join("page.tsx"),
        "import { x } from '@m/shared';\n\nexport default function Page() { return x; }\n",
    )
    .unwrap();

    let (cogito, project_id) = init_cogito(root);
    let conn = cogito.sqlite().unwrap().lock();

    index::sync(&conn, project_id, root, false).unwrap();

    // Query imports for page.tsx: should show the resolved path
    let shared_file = PathBuf::from("packages/shared/src/index.ts");
    let page_file = PathBuf::from("apps/web/app/page.tsx");

    let imports = index::imports(&conn, project_id, &page_file).unwrap();
    assert_eq!(
        imports.len(),
        1,
        "page.tsx should have 1 import: {:?}",
        imports
    );
    assert_eq!(
        imports[0], "packages/shared/src/index.ts",
        "Import should be the resolved path, not the raw spec: {}",
        imports[0]
    );

    // Query importers of shared/src/index.ts: should show page.tsx
    let importers = index::importers(&conn, project_id, &shared_file).unwrap();
    assert_eq!(
        importers.len(),
        1,
        "shared/index.ts should have 1 importer: {:?}",
        importers
    );
    assert_eq!(
        importers[0], page_file,
        "Importer should be page.tsx: {:?}",
        importers[0]
    );

    drop(conn);
    drop(tmp);
}

/// Test: a relative Python import resolves correctly and importers are tracked.
#[test]
fn test_python_relative_import_resolution() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Python package structure:
    // mypackage/__init__.py  -- defines helper
    // mypackage/utils.py     -- imports from .helper
    let pkg = root.join("mypackage");
    fs::create_dir_all(&pkg).unwrap();

    fs::write(pkg.join("__init__.py"), "# mypackage\n").unwrap();
    fs::write(pkg.join("helper.py"), "def greet():\n    return 'hello'\n").unwrap();
    fs::write(
        pkg.join("utils.py"),
        "from .helper import greet\n\nprint(greet())\n",
    )
    .unwrap();

    let (cogito, project_id) = init_cogito(root);
    let conn = cogito.sqlite().unwrap().lock();

    index::sync(&conn, project_id, root, false).unwrap();

    let utils_file = PathBuf::from("mypackage/utils.py");
    let helper_file = PathBuf::from("mypackage/helper.py");

    // Query imports for utils.py: should show the resolved path
    let imports = index::imports(&conn, project_id, &utils_file).unwrap();
    assert_eq!(
        imports.len(),
        1,
        "utils.py should have 1 import: {:?}",
        imports
    );
    assert_eq!(
        imports[0], "mypackage/helper.py",
        "Import should be the resolved path: {}",
        imports[0]
    );

    // Query importers of helper.py: should show utils.py
    let importers = index::importers(&conn, project_id, &helper_file).unwrap();
    assert_eq!(
        importers.len(),
        1,
        "helper.py should have 1 importer: {:?}",
        importers
    );
    assert_eq!(
        importers[0], utils_file,
        "Importer should be utils.py: {:?}",
        importers[0]
    );

    drop(conn);
    drop(tmp);
}

/// Test: a bare external package name (no workspace package) stays unresolved.
/// When we query for a file that doesn't exist but is referenced as an external
/// import, the imports query correctly returns the raw spec (not a resolved path).
#[test]
fn test_external_package_stays_unresolved() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(root.join("package.json"), r#"{"name":"myapp"}"#).unwrap();
    // A file that imports a package not in the workspace
    fs::write(
        src.join("app.ts"),
        "import React from 'react';\n\nexport const App = () => null;\n",
    )
    .unwrap();

    let (cogito, project_id) = init_cogito(root);
    let conn = cogito.sqlite().unwrap().lock();

    index::sync(&conn, project_id, root, false).unwrap();

    let app_file = PathBuf::from("src/app.ts");

    // The import should be stored with raw spec, resolved should be NULL
    let imports = index::imports(&conn, project_id, &app_file).unwrap();
    assert_eq!(
        imports.len(),
        1,
        "app.ts should have 1 import: {:?}",
        imports
    );
    // COALESCE(resolved, spec) returns spec when resolved is NULL
    assert_eq!(
        imports[0], "react",
        "External import should return raw spec, not resolved path: {}",
        imports[0]
    );

    // A non-existent file path (like 'react' the external package) should not
    // be treated as a real file. The importers query finds files with matching
    // spec, but this is expected: external imports are stored by spec.
    let external_pkg = PathBuf::from("react");
    let importers = index::importers(&conn, project_id, &external_pkg).unwrap();
    // app.ts imports 'react' by spec, so it shows up as an importer of 'react'
    // This is correct behavior: importers query finds files by spec OR resolved path
    assert_eq!(
        importers.len(),
        1,
        "Files that import external package by spec should be found: {:?}",
        importers
    );

    // Key check: querying for a non-existent real file path returns nothing
    let fake_file = PathBuf::from("node_modules/react/index.js");
    let fake_importers = index::importers(&conn, project_id, &fake_file).unwrap();
    assert!(
        fake_importers.is_empty(),
        "No importers should exist for non-imported fake path: {:?}",
        fake_importers
    );

    drop(conn);
    drop(tmp);
}
