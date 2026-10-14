//! Guards on publish metadata.
//!
//! Everything published to crates.io is permanent, so a wrong homepage, a
//! security address on a domain we do not own, or a crate missing its
//! keywords and categories cannot be fixed by publishing again. These tests
//! read the manifests directly and fail before anything is uploaded.

use std::path::{Path, PathBuf};

/// Repository root, from this crate's manifest directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("Cargo.toml").exists() && p.join("crates").exists())
        .expect("workspace root")
        .to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The publishable crates.
const PUBLISHABLE: &[&str] = &["cogito-harness-core", "cogito-harness"];

#[test]
fn no_metadata_references_a_domain_we_do_not_own() {
    // cogito.ai is a parked domain for sale. A homepage there points at
    // someone else's page and a security address there delivers reports to
    // whoever buys it.
    //
    // Only *values* are checked, not comments: the manifests deliberately
    // document why the domain was dropped, and that prose is not published.
    let root = workspace_root();
    for relative in [
        "Cargo.toml",
        "SECURITY.md",
        "crates/cogito-harness/Cargo.toml",
        "crates/cogito-harness-core/Cargo.toml",
    ] {
        let path = root.join(relative);
        let text = read(&path);
        for line in text.lines() {
            let trimmed = line.trim_start();
            // Skip comments in Rust-style and Markdown/TOML prose.
            if trimmed.starts_with('#') || trimmed.starts_with("//") {
                continue;
            }
            assert!(
                !line.contains("cogito.ai"),
                "{relative} still references the cogito.ai domain in a value: {line}"
            );
        }
    }
}

#[test]
fn workspace_homepage_points_at_the_repository() {
    let manifest = read(&workspace_root().join("Cargo.toml"));
    assert!(
        manifest.contains("homepage = \"https://github.com/Rene-Kuhm/cogito-harness\""),
        "workspace homepage must be the repository URL"
    );
    assert!(
        manifest.contains("repository = \"https://github.com/Rene-Kuhm/cogito-harness\""),
        "workspace repository must be the repository URL"
    );
}

#[test]
fn every_publishable_crate_inherits_keywords_and_categories() {
    let root = workspace_root();

    // The workspace has to define them in the first place.
    let workspace = read(&root.join("Cargo.toml"));
    let package_section = workspace
        .split("[workspace.package]")
        .nth(1)
        .expect("[workspace.package] section");
    let package_section = package_section.split("\n[").next().unwrap();
    assert!(
        package_section.contains("keywords = ["),
        "workspace must define keywords"
    );
    assert!(
        package_section.contains("categories = ["),
        "workspace must define categories"
    );

    for crate_name in PUBLISHABLE {
        let manifest = read(&root.join(format!("crates/{crate_name}/Cargo.toml")));
        assert!(
            manifest.contains("keywords.workspace = true"),
            "{crate_name} does not inherit keywords, so it will not be \
             searchable by keyword on crates.io"
        );
        assert!(
            manifest.contains("categories.workspace = true"),
            "{crate_name} does not inherit categories, so it will not appear \
             in crates.io category listings"
        );
    }
}

#[test]
fn every_publishable_crate_has_a_specific_description() {
    let root = workspace_root();
    for crate_name in PUBLISHABLE {
        let manifest = read(&root.join(format!("crates/{crate_name}/Cargo.toml")));
        let description = manifest
            .lines()
            .find_map(|l| l.trim().strip_prefix("description = "))
            .unwrap_or_else(|| panic!("{crate_name} has no description"));

        // Not the workspace default, and long enough to say something useful.
        assert!(
            !description.contains("Intelligent context harness"),
            "{crate_name} still uses the generic workspace description"
        );
        assert!(
            description.len() > 60,
            "{crate_name} description is too thin: {description}"
        );
    }
}

#[test]
fn npm_package_ships_the_license_text() {
    // Apache-2.0 requires redistributing the license, and npm only includes
    // files listed in `files`.
    let root = workspace_root().join("packages/cogito-harness");
    assert!(
        root.join("LICENSE").exists(),
        "packages/cogito-harness/LICENSE is missing"
    );

    let package = read(&root.join("package.json"));
    assert!(
        package.contains("\"LICENSE\""),
        "LICENSE must be listed in the npm package's `files`"
    );
    assert!(
        package.contains("\"license\": \"Apache-2.0\""),
        "the npm package must declare its license"
    );

    // The bundled text has to be the real license, not a stub.
    let text = read(&root.join("LICENSE"));
    assert!(
        text.contains("Apache License") && text.contains("Version 2.0"),
        "bundled LICENSE does not look like Apache-2.0"
    );
}
