//! Regression tests for the multi-project isolation bug.
//!
//! Before this was fixed, the SQLite connection lived in a process-global
//! `OnceCell`: the first project opened by a process won, and every later
//! read/write silently targeted that first project's database. As an MCP
//! server, one process serves many projects, so this was a data-corruption
//! bug rather than a theoretical one.

use cogito_harness_core::{Cogito, decision};
use std::path::PathBuf;
use tempfile::TempDir;

fn project_with(marker: &str) -> (TempDir, PathBuf) {
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

#[test]
fn two_projects_do_not_share_state() {
    let (_a_dir, a) = project_with("a");
    let (_b_dir, b) = project_with("b");

    // Project A
    let cogito_a = Cogito::for_project(&a).unwrap();
    let project_a = cogito_a.load_project(a.clone()).unwrap();
    cogito_a
        .add_decision(
            decision::builder(project_a.id)
                .title("Decision in A")
                .why("only A should see this")
                .build(),
        )
        .unwrap();

    // Project B — a separate instance bound to a different directory
    let cogito_b = Cogito::for_project(&b).unwrap();
    let project_b = cogito_b.load_project(b.clone()).unwrap();

    // A and B must be distinct projects
    assert_ne!(project_a.id, project_b.id, "paths must map to distinct ids");

    // B must not see A's decision
    let b_decisions = cogito_b.search_decisions(project_b.id, "").unwrap();
    assert!(
        b_decisions.is_empty(),
        "project B must not see project A's decisions, got {b_decisions:?}"
    );

    // A must still see its own decision
    let a_decisions = cogito_a.search_decisions(project_a.id, "").unwrap();
    assert_eq!(a_decisions.len(), 1);
    assert_eq!(a_decisions[0].title, "Decision in A");

    // Now write to B and confirm A is unaffected
    cogito_b
        .add_decision(
            decision::builder(project_b.id)
                .title("Decision in B")
                .why("only B should see this")
                .build(),
        )
        .unwrap();

    let a_decisions = cogito_a.search_decisions(project_a.id, "").unwrap();
    assert_eq!(
        a_decisions.len(),
        1,
        "A must still have exactly one decision"
    );
    assert_eq!(a_decisions[0].title, "Decision in A");

    let b_decisions = cogito_b.search_decisions(project_b.id, "").unwrap();
    assert_eq!(b_decisions.len(), 1);
    assert_eq!(b_decisions[0].title, "Decision in B");
}

#[test]
fn tags_are_isolated_per_project() {
    let (_a_dir, a) = project_with("a");
    let (_b_dir, b) = project_with("b");

    let cogito_a = Cogito::for_project(&a).unwrap();
    let project_a = cogito_a.load_project(a.clone()).unwrap();
    cogito_a
        .add_tag(cogito_harness_core::types::SemanticTag {
            id: uuid::Uuid::new_v4(),
            project_id: project_a.id,
            tag: "only-in-a".to_string(),
            domain: "test".to_string(),
            paths: vec![PathBuf::from("src/a/")],
            confidence: 1.0,
            created_at: chrono::Utc::now(),
        })
        .unwrap();

    let cogito_b = Cogito::for_project(&b).unwrap();
    let project_b = cogito_b.load_project(b.clone()).unwrap();

    assert!(
        cogito_b.search_tags(project_b.id, "").unwrap().is_empty(),
        "project B must not see project A's tags"
    );
    assert_eq!(cogito_a.search_tags(project_a.id, "").unwrap().len(), 1);
}

#[test]
fn state_lives_under_the_project_directory() {
    let (_dir, path) = project_with("x");
    let cogito = Cogito::for_project(&path).unwrap();

    // The instance must be bound to this project's .cogito directory,
    // independent of the process working directory.
    assert_eq!(cogito.cogito_dir(), path.join(".cogito"));

    let project = cogito.load_project(path.clone()).unwrap();
    cogito
        .add_decision(decision::builder(project.id).title("t").why("w").build())
        .unwrap();

    // The database file must exist inside the project
    assert!(
        path.join(".cogito/state.db").exists(),
        "state.db should be created under the project directory"
    );
}
