//! A fingerprint can be current while the index it describes was never built:
//! a project scanned before the SQLite index existed, for example. The sync
//! fast path must not trust the fingerprint alone, or search stays empty until
//! the next commit.

use cogito_harness_core::Cogito;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[tokio::test]
async fn test_current_fingerprint_with_an_empty_index_is_rebuilt() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/session.ts"),
        "export function requireSession() {}\n",
    )
    .unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "init"]);

    let cogito = Cogito::for_project(root).unwrap();
    let project = cogito.load_project(root.to_path_buf()).unwrap();
    // Scan stores a current fingerprint. It does not build the SQLite index.
    cogito.generate_fingerprint(&project).await.unwrap();
    let stored = cogito.stored_fingerprint(project.id).unwrap();
    assert!(stored.is_some(), "the scan stored a fingerprint");

    // The fingerprint matches HEAD and the tree is clean, so the fast path
    // would skip. It must still build the index, because it is empty.
    cogito.sync_index(&project).await.unwrap();

    let conn = cogito.sqlite().unwrap().lock();
    let files: i64 = conn
        .query_row("SELECT COUNT(*) FROM indexed_files", [], |r| r.get(0))
        .unwrap();
    assert!(
        files >= 1,
        "index populated by sync_index, got {files} files"
    );
}
