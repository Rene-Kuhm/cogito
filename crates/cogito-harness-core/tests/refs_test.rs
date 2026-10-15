//! Reference edges: which files use a name a changed file declares.
//!
//! A reference is a *use* reached through an import edge the index already
//! stores, never a same-name declaration somewhere else. The fixtures pin the
//! exact set for TypeScript and Rust, the two languages v1 resolves, and the
//! explicit "not supported" answer every other language gets.

use cogito_harness_core::Cogito;
use cogito_harness_core::impact::{self, Reference};
use cogito_harness_core::index;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Run `git` in `dir` with an explicit identity; the identity matters because a
/// CI runner has none and a failed commit would leave no HEAD.
fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git failed to start");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn commit_all(root: &Path, message: &str) {
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", message]);
}

/// Index the working tree and return the engine with its project id.
fn indexed(root: &Path) -> (Cogito, uuid::Uuid) {
    let cogito = Cogito::for_project(root).unwrap();
    let project = cogito.init_project(root.to_path_buf()).unwrap();
    cogito.register_project(&project).unwrap();
    let conn = cogito.connection().unwrap();
    index::sync(&conn, project.id, root, false).unwrap();
    drop(conn);
    (cogito, project.id)
}

/// Build the impact of `base..HEAD` for an indexed project.
fn impact_at(root: &Path, base: &str) -> impact::Impact {
    let (cogito, project_id) = indexed(root);
    let conn = cogito.connection().unwrap();
    impact::build(&conn, project_id, root, Some(base)).unwrap()
}

/// A TypeScript project where `computeValue` is imported by two files, imported
/// but unused in a third, declared without importing anywhere in a fourth, and
/// mentioned only in a comment and a string in a fifth.
fn ts_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(root, "package.json", r#"{"name":"refsws","private":true}"#);
    write(
        root,
        "src/math.ts",
        "export function computeValue(input: number): number {\n  return input + 1;\n}\n\nexport const SCALE = 2;\n",
    );
    write(
        root,
        "src/consumer.ts",
        "import { computeValue } from './math';\n\nexport function useValue(): number {\n  return computeValue(1) + computeValue(2);\n}\n",
    );
    write(
        root,
        "src/other.ts",
        "import { computeValue, SCALE } from './math';\n\nexport function otherValue(): number {\n  return computeValue(SCALE);\n}\n",
    );
    // Declares the same name and imports nothing: not a reference.
    write(
        root,
        "src/copy.ts",
        "export function computeValue(): number {\n  return 0;\n}\n",
    );
    // Imports the changed file but never uses the changed name.
    write(
        root,
        "src/nosy.ts",
        "import { SCALE } from './math';\n\nexport function scale(): number {\n  return SCALE;\n}\n",
    );
    // Mentions the name in a comment and a string only.
    write(
        root,
        "src/docs.ts",
        "import { SCALE } from './math';\n\n// computeValue lives in math.ts\nexport const NOTE = 'computeValue';\n",
    );
    write(
        root,
        "src/unrelated.ts",
        "export function untouched(): number {\n  return 1;\n}\n",
    );
    git(root, &["init", "-q"]);
    commit_all(root, "initial");
    dir
}

#[test]
fn test_typescript_references_are_exactly_the_importing_users() {
    let dir = ts_project();
    let root = dir.path();

    write(
        root,
        "src/math.ts",
        "export function computeValue(input: number): number {\n  return input + 2;\n}\n\nexport const SCALE = 2;\n",
    );
    commit_all(root, "change");

    let impact = impact_at(root, "HEAD~1");

    assert_eq!(
        impact.references,
        vec![
            Reference {
                file: PathBuf::from("src/consumer.ts"),
                declared_in: PathBuf::from("src/math.ts"),
                name: "computeValue".to_string(),
            },
            Reference {
                file: PathBuf::from("src/other.ts"),
                declared_in: PathBuf::from("src/math.ts"),
                name: "computeValue".to_string(),
            },
        ],
        "only the files that import math.ts and use the name reference it"
    );
    assert!(
        impact.unsupported.is_empty(),
        "TypeScript is supported: {:?}",
        impact.unsupported
    );
}

#[test]
fn test_rust_references_include_a_use_in_the_declaring_file() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(
        root,
        "Cargo.toml",
        "[package]\nname = \"refsws\"\nversion = \"0.1.0\"\n",
    );
    write(
        root,
        "src/lib.rs",
        "pub mod app;\npub mod homonym;\npub mod math;\npub mod other;\npub mod unused;\n",
    );
    // `twice` uses `compute`, so the declaring file references the name itself.
    write(
        root,
        "src/math.rs",
        "pub fn compute(input: i32) -> i32 { input + 1 }\npub fn twice(input: i32) -> i32 { compute(input) + compute(input) }\n",
    );
    write(
        root,
        "src/app.rs",
        "use crate::math::compute;\n\npub fn run() -> i32 { compute(1) }\n",
    );
    write(
        root,
        "src/other.rs",
        "use crate::math::compute;\n\npub fn other() -> i32 { compute(2) }\n",
    );
    // Same name, declared here and never imported from the changed file.
    write(root, "src/homonym.rs", "pub fn compute() -> i32 { 0 }\n");
    // Imports the changed file but never uses the changed name.
    write(
        root,
        "src/unused.rs",
        "use crate::math::compute;\n\npub fn nothing() -> i32 { 0 }\n",
    );
    git(root, &["init", "-q"]);
    commit_all(root, "initial");

    write(
        root,
        "src/math.rs",
        "pub fn compute(input: i32) -> i32 { input + 7 }\npub fn twice(input: i32) -> i32 { compute(input) + compute(input) }\n",
    );
    commit_all(root, "change");

    let impact = impact_at(root, "HEAD~1");

    assert_eq!(
        impact.references,
        vec![
            Reference {
                file: PathBuf::from("src/app.rs"),
                declared_in: PathBuf::from("src/math.rs"),
                name: "compute".to_string(),
            },
            Reference {
                file: PathBuf::from("src/math.rs"),
                declared_in: PathBuf::from("src/math.rs"),
                name: "compute".to_string(),
            },
            Reference {
                file: PathBuf::from("src/other.rs"),
                declared_in: PathBuf::from("src/math.rs"),
                name: "compute".to_string(),
            },
        ],
        "Rust references are the two users plus the declaring file's own use"
    );
}

#[test]
fn test_a_language_v1_does_not_resolve_says_not_supported() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(root, "src/helper.py", "def thing():\n    return 1\n");
    write(
        root,
        "src/app.py",
        "from helper import thing\n\ndef run():\n    return thing()\n",
    );
    git(root, &["init", "-q"]);
    commit_all(root, "initial");

    write(
        root,
        "src/app.py",
        "from helper import thing\n\ndef run():\n    return thing() + thing()\n",
    );
    commit_all(root, "change");

    let impact = impact_at(root, "HEAD~1");

    assert_eq!(
        impact.unsupported,
        vec![PathBuf::from("src/app.py")],
        "a changed Python file is named, not guessed at"
    );
    assert!(
        impact.references.is_empty(),
        "no reference edge is invented for Python: {:?}",
        impact.references
    );

    let text = impact::render(&impact);
    assert!(
        text.contains("not supported"),
        "the rendering says so in those words:\n{text}"
    );
    assert!(
        text.contains("src/app.py"),
        "and names the file it did not resolve:\n{text}"
    );
    assert!(
        !text.contains("uses thing"),
        "no name-based guess is printed for Python:\n{text}"
    );
}

#[test]
fn test_references_are_resolved_only_for_typescript_and_rust() {
    for ext in ["ts", "tsx", "mts", "cts", "rs"] {
        assert!(
            impact::references_supported(ext),
            "{ext} is one of the v1 languages"
        );
    }
    for ext in ["js", "jsx", "mjs", "cjs", "py", "go", "java", "rb", ""] {
        assert!(
            !impact::references_supported(ext),
            "{ext} must answer not supported, never guess"
        );
    }
    assert!(
        impact::references_supported("TS"),
        "extensions are case-folded"
    );
}
