//! Benchmarks for the Code Fingerprint generator
//!
//! Run with: `cargo bench --bench fingerprint`
//!
//! Measures end-to-end fingerprint generation on synthetic codebases of
//! varying sizes, plus real-world measurement against an actual project.

use cogito_core::Cogito;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::fs;
use tempfile::TempDir;

/// Create a synthetic project with N Rust source files.
fn synth_project(num_files: usize) -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "synth"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    fs::create_dir_all(root.join("src")).unwrap();

    // Always one main.rs as entry point
    fs::write(
        root.join("src/main.rs"),
        "fn main() { println!(\"hello\"); }\n",
    )
    .unwrap();

    // N other files with realistic-looking content
    for i in 0..num_files {
        let content = format!(
            r#"//! Module {i}
use crate::types::*;

/// Function {i}
pub fn process_{i}(input: &str) -> Result<String, Error> {{
    if input.is_empty() {{
        return Err(Error::Empty);
    }}
    let parsed = parse(input)?;
    Ok(transform(parsed))
}}

fn parse(s: &str) -> Result<Parsed, Error> {{
    Ok(Parsed {{ value: s.to_string() }})
}}

fn transform(p: Parsed) -> String {{
    p.value.to_uppercase()
}}

pub struct Parsed {{
    pub value: String,
}}

#[derive(Debug)]
pub enum Error {{
    Empty,
    Parse,
}}
"#
        );
        fs::write(root.join(format!("src/module_{i}.rs")), content).unwrap();
    }

    // Init git so fingerprint has history to work with
    let _ = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(root)
        .output();
    // Make a first commit on whatever default branch git uses
    let _ = std::process::Command::new("git")
        .args(["config", "user.email=bench@bench"])
        .current_dir(root)
        .output();
    let _ = std::process::Command::new("git")
        .args(["config", "user.name=Bench"])
        .current_dir(root)
        .output();
    let _ = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(root)
        .output();
    let _ = std::process::Command::new("git")
        .args([
            "commit",
            "-m",
            "init",
            "--no-gpg-sign",
            "-q",
            "--allow-empty",
        ])
        .current_dir(root)
        .output();
    // Rename whatever default branch git chose to 'main'
    let _ = std::process::Command::new("git")
        .args(["branch", "-m", "main"])
        .current_dir(root)
        .output();

    // Make a few more commits for zone detection
    for i in 0..3 {
        let _ = std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(root)
            .output();
        let _ = std::process::Command::new("git")
            .args([
                "commit",
                "--allow-empty",
                "-m",
                &format!("commit {i}"),
                "--no-gpg-sign",
                "-q",
            ])
            .current_dir(root)
            .output();
    }

    dir
}

fn bench_fingerprint_synth(c: &mut Criterion) {
    let mut group = c.benchmark_group("fingerprint_synth");
    let cogito = Cogito::new();

    for num_files in [10, 50, 200, 500].iter() {
        // Create the project ONCE per size, outside the timed loop.
        // The git operations and fs work would dominate otherwise.
        let project = synth_project(*num_files);
        let total_bytes: u64 = walkdir::WalkDir::new(project.path())
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok().map(|m| m.len()))
            .sum();

        let p = cogito
            .init_project(project.path().to_path_buf())
            .expect("Failed to init synth project");

        // Sanity check before benchmarking
        match futures::executor::block_on(cogito.generate_fingerprint(&p)) {
            Ok(_) => {}
            Err(e) => {
                eprintln!(
                    "Skipping {num_files} files: synth project did not produce fingerprint: {e}"
                );
                continue;
            }
        }

        group.throughput(Throughput::Bytes(total_bytes));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_files),
            num_files,
            |b, _n| {
                b.iter(|| {
                    let _fp = futures::executor::block_on(cogito.generate_fingerprint(&p)).unwrap();
                });
            },
        );
    }

    group.finish();
}

fn bench_fingerprint_real_world(c: &mut Criterion) {
    // Walk up from CARGO_MANIFEST_DIR to find the cogito root
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    let cogito_root = manifest_dir
        .ancestors()
        .find(|p| p.join("Cargo.toml").exists() && p.join("crates").exists())
        .map(|p| p.to_path_buf());

    let Some(root) = cogito_root else {
        eprintln!("Could not find cogito root, skipping real-world bench");
        return;
    };

    // Try to find a default branch: main, master, or HEAD
    let branch_exists = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "refs/heads/main"])
        .current_dir(&root)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        || std::process::Command::new("git")
            .args(["rev-parse", "--verify", "refs/heads/master"])
            .current_dir(&root)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

    if !branch_exists {
        eprintln!("No main or master branch, skipping real-world bench");
        return;
    }

    if !std::path::Path::new(&root).join(".git").exists() {
        eprintln!("No git repo at {}, skipping", root.display());
        return;
    }

    let total_files = walkdir::WalkDir::new(&root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .map(|x| x == "rs" || x == "md" || x == "toml")
                .unwrap_or(false)
        })
        .count();

    println!(
        "Real-world benchmark: scanning cogito itself ({} Rust/MD/TOML files at {})",
        total_files,
        root.display()
    );

    let mut group = c.benchmark_group("fingerprint_real_world");
    let cogito = Cogito::new();

    let project = cogito
        .init_project(root.clone())
        .expect("Failed to init real-world project");

    group.bench_function("cogito_self_scan", |b| {
        b.iter(|| {
            let _fp = futures::executor::block_on(cogito.generate_fingerprint(&project)).unwrap();
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_fingerprint_synth,
    bench_fingerprint_real_world
);
criterion_main!(benches);
