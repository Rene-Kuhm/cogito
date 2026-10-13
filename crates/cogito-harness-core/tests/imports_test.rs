//! Integration tests for import extraction and resolution across languages.
//!
//! Tests use real temporary project directories and exercise the public
//! surface: `cogito_harness_core::fingerprint::generate`.

use cogito_harness_core::fingerprint::generate;
use cogito_harness_core::types::{DataFlow, Language, Project};
use pretty_assertions::assert_eq;
use std::fs;
use tempfile::TempDir;

/// Initialize a git repo in `root` so the fingerprint scanner has a working tree.
fn git_init(root: &std::path::Path) {
    use std::process::Command;
    let _ = Command::new("git")
        .args(["init", "-q"])
        .current_dir(root)
        .output()
        .expect("git init failed");
    let _ = Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(root)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(root)
        .output();
    let _ = Command::new("git")
        .args(["add", "-A"])
        .current_dir(root)
        .output();
    let _ = Command::new("git")
        .args(["commit", "-q", "-m", "init", "--no-gpg-sign"])
        .current_dir(root)
        .output();
}

/// A minimal project that carries an id and path for the scanner.
/// Uses Cogito::init_project pattern without requiring the full Cogito instance.
fn make_project(root: &std::path::Path, language: Language) -> Project {
    use chrono::Utc;
    Project {
        id: uuid::Uuid::new_v4(),
        name: root.file_name().unwrap().to_string_lossy().into_owned(),
        path: root.to_path_buf(),
        language,
        fingerprint: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// Find the data flow in `flows` whose `from` ends with `from_suffix` and `to` equals `to_str`.
fn find_flow<'a>(flows: &'a [DataFlow], from_suffix: &str, to_str: &str) -> Option<&'a DataFlow> {
    flows
        .iter()
        .find(|f| f.from.ends_with(from_suffix) && f.to.to_string_lossy() == to_str)
}

fn assert_flow_imports_to(
    flows: &[DataFlow],
    from_suffix: &str,
    expected_to: &str,
    expected_resolved: Option<&str>,
) {
    let flow = find_flow(flows, from_suffix, expected_to)
        .unwrap_or_else(|| panic!("No flow found from {from_suffix} with to {expected_to}"));
    let resolved_str = flow
        .resolved
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    assert_eq!(
        resolved_str.as_deref(),
        expected_resolved,
        "resolved path mismatch"
    );
}

/// External packages must never be resolved to a project file.
fn assert_unresolved(flows: &[DataFlow], from_suffix: &str, to: &str) {
    let flow = find_flow(flows, from_suffix, to)
        .unwrap_or_else(|| panic!("No flow found from {from_suffix} with to {to}"));
    assert_eq!(flow.resolved, None, "external package must not resolve");
}

// ─────────────────────────────────────────────────────────────────────────────
// Python
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn python_import_resolves_to_module() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("pkg")).unwrap();
    fs::write(root.join("pkg/__init__.py"), "").unwrap();
    fs::write(root.join("pkg/auth.py"), "def require_session(): pass").unwrap();
    fs::create_dir_all(root.join("app")).unwrap();
    fs::write(
        root.join("app/main.py"),
        "from pkg.auth import require_session\n",
    )
    .unwrap();
    git_init(root);

    let project = make_project(root, Language::Python);
    let fp = generate(&project).await.unwrap();

    assert_flow_imports_to(
        &fp.data_flows,
        "app/main.py",
        "pkg.auth",
        Some("pkg/auth.py"),
    );
}

#[tokio::test]
async fn python_relative_import_resolves() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("pkg")).unwrap();
    fs::write(root.join("pkg/a.py"), "from .b import thing\n").unwrap();
    fs::write(root.join("pkg/b.py"), "thing = 42\n").unwrap();
    git_init(root);

    let project = make_project(root, Language::Python);
    let fp = generate(&project).await.unwrap();

    assert_flow_imports_to(&fp.data_flows, "pkg/a.py", ".b", Some("pkg/b.py"));
}

#[tokio::test]
async fn python_stdlib_remains_unresolved() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    fs::write(root.join("main.py"), "import os\nimport pathlib\n").unwrap();
    git_init(root);

    let project = make_project(root, Language::Python);
    let fp = generate(&project).await.unwrap();

    assert_unresolved(&fp.data_flows, "main.py", "os");
    assert_unresolved(&fp.data_flows, "main.py", "pathlib");
}

// ─────────────────────────────────────────────────────────────────────────────
// Go
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn go_import_resolves_to_package() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    fs::write(root.join("go.mod"), "module example.com/app\n").unwrap();
    fs::create_dir_all(root.join("internal/auth")).unwrap();
    fs::write(root.join("internal/auth/auth.go"), "package auth\n").unwrap();
    fs::create_dir_all(root.join("cmd")).unwrap();
    fs::write(
        root.join("cmd/main.go"),
        r#"package main
import "example.com/app/internal/auth"
"#,
    )
    .unwrap();
    git_init(root);

    let project = make_project(root, Language::Go);
    let fp = generate(&project).await.unwrap();

    assert_flow_imports_to(
        &fp.data_flows,
        "cmd/main.go",
        "example.com/app/internal/auth",
        Some("internal/auth/auth.go"),
    );
}

#[tokio::test]
async fn go_relative_import_resolves() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    fs::write(root.join("go.mod"), "module example.com/app\n").unwrap();
    fs::create_dir_all(root.join("lib")).unwrap();
    fs::write(root.join("lib/utils.go"), "package lib\n").unwrap();
    fs::create_dir_all(root.join("internal/auth")).unwrap();
    fs::write(
        root.join("internal/auth/auth.go"),
        r#"package auth
import "../lib"
"#,
    )
    .unwrap();
    git_init(root);

    let project = make_project(root, Language::Go);
    let fp = generate(&project).await.unwrap();

    assert_flow_imports_to(
        &fp.data_flows,
        "internal/auth/auth.go",
        "../lib",
        Some("lib/utils.go"),
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Java
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn java_import_resolves_to_source_file() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("src/main/java/com/acme")).unwrap();
    fs::write(
        root.join("src/main/java/com/acme/Auth.java"),
        "package com.acme;\n",
    )
    .unwrap();
    fs::write(
        root.join("src/main/java/com/acme/App.java"),
        "package com.acme;\nimport com.acme.Auth;\n",
    )
    .unwrap();
    git_init(root);

    let project = make_project(root, Language::Java);
    let fp = generate(&project).await.unwrap();

    assert_flow_imports_to(
        &fp.data_flows,
        "src/main/java/com/acme/App.java",
        "com.acme.Auth",
        Some("src/main/java/com/acme/Auth.java"),
    );
}

#[tokio::test]
async fn java_static_import_resolves() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("src/main/java/com/acme")).unwrap();
    fs::write(
        root.join("src/main/java/com/acme/Auth.java"),
        "package com.acme;\npublic class Auth { public static boolean check() { return true; } }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/main/java/com/acme/App.java"),
        "package com.acme;\nimport static com.acme.Auth.check;\n",
    )
    .unwrap();
    git_init(root);

    let project = make_project(root, Language::Java);
    let fp = generate(&project).await.unwrap();

    // The extractor trims the `.check` member from the import, leaving the class.
    assert_flow_imports_to(
        &fp.data_flows,
        "src/main/java/com/acme/App.java",
        "com.acme.Auth",
        Some("src/main/java/com/acme/Auth.java"),
    );
}

#[tokio::test]
async fn java_wildcard_import_resolves() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("src/main/java/com/acme/util")).unwrap();
    fs::write(
        root.join("src/main/java/com/acme/util/Strings.java"),
        "package com.acme.util;\n",
    )
    .unwrap();
    fs::write(
        root.join("src/main/java/com/acme/App.java"),
        "package com.acme;\nimport com.acme.util.*;\n",
    )
    .unwrap();
    git_init(root);

    let project = make_project(root, Language::Java);
    let fp = generate(&project).await.unwrap();

    // Wildcard `import com.acme.util.*` is trimmed to the package path.
    assert_flow_imports_to(
        &fp.data_flows,
        "src/main/java/com/acme/App.java",
        "com.acme.util",
        Some("src/main/java/com/acme/util/Strings.java"),
    );
}

#[tokio::test]
async fn java_stdlib_remains_unresolved() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("src/main/java")).unwrap();
    fs::write(
        root.join("src/main/java/App.java"),
        "package;\nimport java.util.List;\nimport java.io.File;\n",
    )
    .unwrap();
    git_init(root);

    let project = make_project(root, Language::Java);
    let fp = generate(&project).await.unwrap();

    assert_unresolved(&fp.data_flows, "src/main/java/App.java", "java.util.List");
    assert_unresolved(&fp.data_flows, "src/main/java/App.java", "java.io.File");
}

// ─────────────────────────────────────────────────────────────────────────────
// Polyglot: Rust project containing Python files
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn polyglot_rust_project_extracts_and_resolves_python_imports() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    // Root Cargo.toml makes the project language Rust.
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"myproject\"\n").unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

    // Python files at the project root level (where the import path expects them).
    // This tests that Python imports are extracted and resolved even when the
    // project language is Rust (per-file language detection).
    fs::create_dir_all(root.join("pkg")).unwrap();
    fs::write(root.join("pkg/__init__.py"), "").unwrap();
    fs::write(root.join("pkg/auth.py"), "def require_session(): pass\n").unwrap();
    fs::create_dir_all(root.join("app")).unwrap();
    fs::write(
        root.join("app/main.py"),
        "from pkg.auth import require_session\n",
    )
    .unwrap();
    git_init(root);

    // Project language is Rust.
    let project = make_project(root, Language::Rust);
    let fp = generate(&project).await.unwrap();

    // Python import must be extracted despite the project being Rust.
    assert_flow_imports_to(
        &fp.data_flows,
        "app/main.py",
        "pkg.auth",
        Some("pkg/auth.py"),
    );
}
