//! Real-world measurement tests
//!
//! These tests download or use real OSS codebases to verify that Cogito
//! works on production-scale projects. They are gated by an env var
//! `COGITO_RUN_REAL_WORLD=1` so CI doesn't always clone large repos.
//!
//! To run: `COGITO_RUN_REAL_WORLD=1 cargo test --test real_world_test -- --ignored`

use std::env;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Skip this test unless COGITO_RUN_REAL_WORLD=1 is set.
fn should_run() -> bool {
    env::var("COGITO_RUN_REAL_WORLD").is_ok()
}

/// Clone a repo into a temp dir using `git clone --depth 100` (cheap, fast).
fn clone_shallow(url: &str) -> Option<(TempDir, PathBuf)> {
    let dir = TempDir::new().ok()?;
    let path = dir.path().to_path_buf();

    eprintln!("Cloning {url} into {}...", path.display());
    let status = Command::new("git")
        .args(["clone", "--depth", "100", "--quiet", url, path.to_str()?])
        .status()
        .ok()?;

    if !status.success() {
        eprintln!("Clone failed: {status}");
        return None;
    }

    Some((dir, path))
}

#[tokio::test]
#[ignore] // Run with --ignored
async fn real_world_ripgrep_small() {
    if !should_run() {
        eprintln!("Skipping: set COGITO_RUN_REAL_WORLD=1 to run");
        return;
    }

    // ripgrep is a medium-sized Rust project (~30k LOC, fast clone)
    let Some((_dir, path)) = clone_shallow("https://github.com/BurntSushi/ripgrep.git") else {
        eprintln!("Could not clone ripgrep - skipping");
        return;
    };

    eprintln!("=== ripgrep real-world test ===");
    eprintln!("Path: {}", path.display());

    let cogito = cogito_harness_core::Cogito::for_project(&path).unwrap();
    let project = cogito.load_project(path).unwrap();

    eprintln!("Project: {} ({})", project.name, project.language);

    let start = std::time::Instant::now();
    let fingerprint = cogito
        .generate_fingerprint(&project)
        .await
        .expect("fingerprint");
    let elapsed = start.elapsed();

    eprintln!("Fingerprint time: {:?}", elapsed);
    eprintln!("Entry points: {}", fingerprint.entry_points.len());
    eprintln!("Data flows: {}", fingerprint.data_flows.len());
    eprintln!("Hot zones: {}", fingerprint.hot_zones.len());
    eprintln!("Cold zones: {}", fingerprint.cold_zones.len());

    // Sanity: a real Rust project must have many entry points and zones
    assert!(
        fingerprint.entry_points.len() >= 3,
        "Expected many entry points"
    );
    assert!(
        !fingerprint.hot_zones.is_empty(),
        "Expected hot zones from real git history"
    );
}

#[tokio::test]
#[ignore] // Run with --ignored
async fn real_world_bat_medium() {
    if !should_run() {
        eprintln!("Skipping: set COGITO_RUN_REAL_WORLD=1 to run");
        return;
    }

    // bat is a larger project (~30k+ LOC, well-maintained, Rust)
    let Some((_dir, path)) = clone_shallow("https://github.com/sharkdp/bat.git") else {
        eprintln!("Could not clone bat - skipping");
        return;
    };

    eprintln!("=== bat real-world test ===");
    eprintln!("Path: {}", path.display());

    let cogito = cogito_harness_core::Cogito::for_project(&path).unwrap();
    let project = cogito.load_project(path).unwrap();

    let start = std::time::Instant::now();
    let fingerprint = cogito
        .generate_fingerprint(&project)
        .await
        .expect("fingerprint");
    let elapsed = start.elapsed();

    eprintln!("Project: {} ({})", project.name, project.language);
    eprintln!("Fingerprint time: {:?}", elapsed);
    eprintln!("Entry points: {}", fingerprint.entry_points.len());
    eprintln!("Data flows: {}", fingerprint.data_flows.len());
    eprintln!("Hot zones: {}", fingerprint.hot_zones.len());
    eprintln!("Cold zones: {}", fingerprint.cold_zones.len());

    assert!(fingerprint.entry_points.len() >= 3);
    assert!(!fingerprint.hot_zones.is_empty());
}
