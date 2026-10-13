//! A scan must leave its fingerprint in the project's database.
//!
//! Before this, nothing wrote the `fingerprints` table: `project_stats`
//! reported zero hot zones after every scan, and the session-start hooks that
//! scan in the background had no lasting effect.

use cogito_harness_core::Cogito;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A Rust project with a few recent commits, so it has hot zones.
fn project() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
    git(root, &["init", "-q"]);
    for n in 0..12 {
        std::fs::write(root.join("src/main.rs"), format!("fn main() {{ {n}; }}\n")).unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", &format!("change {n}")]);
    }
    dir
}

#[tokio::test]
async fn scan_stores_the_fingerprint() {
    let dir = project();
    let root = dir.path();

    let cogito = Cogito::for_project(root).unwrap();
    let project = cogito.load_project(root.to_path_buf()).unwrap();

    let before = cogito.stats(project.id).unwrap();
    assert!(before.fingerprint_generated_at.is_none());
    assert!(cogito.stored_fingerprint(project.id).unwrap().is_none());

    let fingerprint = cogito.generate_fingerprint(&project).await.unwrap();
    assert!(
        !fingerprint.hot_zones.is_empty(),
        "the fixture must have hot zones"
    );

    let after = cogito.stats(project.id).unwrap();
    assert_eq!(after.hot_zones_count, fingerprint.hot_zones.len() as i64);
    assert!(after.fingerprint_generated_at.is_some());

    let stored = cogito.stored_fingerprint(project.id).unwrap().unwrap();
    assert_eq!(stored.generated_at, fingerprint.generated_at);
    assert_eq!(stored.hot_zones.len(), fingerprint.hot_zones.len());
}

#[tokio::test]
async fn stored_fingerprint_survives_a_new_process_and_is_replaced_by_a_rescan() {
    let dir = project();
    let root = dir.path();

    let first = {
        let cogito = Cogito::for_project(root).unwrap();
        let project = cogito.load_project(root.to_path_buf()).unwrap();
        cogito.generate_fingerprint(&project).await.unwrap()
    };

    // A fresh instance is what the next session (or the MCP server) opens.
    let cogito = Cogito::for_project(root).unwrap();
    let project = cogito.load_project(root.to_path_buf()).unwrap();
    let stored = cogito.stored_fingerprint(project.id).unwrap().unwrap();
    assert_eq!(stored.generated_at, first.generated_at);

    let second = cogito.generate_fingerprint(&project).await.unwrap();
    let stored = cogito.stored_fingerprint(project.id).unwrap().unwrap();
    assert_eq!(
        stored.generated_at, second.generated_at,
        "one row per project"
    );
}

#[tokio::test]
async fn zones_are_most_changed_first() {
    let dir = project();
    let root = dir.path();
    // A second file, changed once: it must rank below src/main.rs.
    std::fs::write(root.join("src/lib.rs"), "\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "lib"]);

    let cogito = Cogito::for_project(root).unwrap();
    let project = cogito.load_project(root.to_path_buf()).unwrap();
    let fingerprint = cogito.generate_fingerprint(&project).await.unwrap();

    let all: Vec<_> = fingerprint
        .hot_zones
        .iter()
        .chain(&fingerprint.cold_zones)
        .collect();
    for zones in [&fingerprint.hot_zones, &fingerprint.cold_zones] {
        assert!(
            zones
                .windows(2)
                .all(|w| w[0].change_count >= w[1].change_count),
            "zones must be sorted by change count: {all:?}"
        );
    }
}
