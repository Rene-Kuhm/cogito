//! Opt-in scale benchmarks for the file and symbol index.
//!
//! Run with: `COGITO_SCALE=1 cargo test --release -p cogito-harness-core --test scale_test -- --ignored --nocapture`
//!
//! This harness generates a synthetic tree of 100,000 files (Rust,
//! TypeScript, Python, and Markdown) and prints a table of cold/warm sync
//! and search latencies. It is not a functional test — its whole purpose is
//! to produce numbers.

use cogito_harness_core::index;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use uuid::Uuid;

/// Generate a synthetic project tree with `n` files spread across Rust,
/// TypeScript/TSX, Python, and Markdown in nested directories.
/// Does NOT initialise a git repository.
fn generate_project(n: usize) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();

    let langs: &[(&str, &str, &str)] = &[
        ("rs", "src", "// Rust source"),
        ("ts", "src", "// TypeScript source"),
        ("py", "src", "# Python source"),
        ("md", "docs", "# Markdown doc"),
    ];

    // Target files per language
    let per_lang = n / langs.len();
    let files_per_dir = 50;

    for (ext, base_dir, comment) in langs {
        let dir = root.join(base_dir);
        fs::create_dir_all(&dir).unwrap();

        for i in 0..per_lang {
            let subdir_num = i / files_per_dir;
            let subdir = dir.join(format!("sub_{:05}", subdir_num));
            fs::create_dir_all(&subdir).unwrap();

            let filename = format!("file_{:06}.{}", i, ext);
            let content =
                format!("{comment}\n// File {i}\npub fn fn_{i}() {{}}\npub struct S{i};\n");
            fs::write(subdir.join(&filename), content).unwrap();
        }
    }

    // Ensure at least the requested count
    let actual = count_files(&root);
    assert!(
        actual >= n,
        "Generated {actual} files, expected at least {n}"
    );

    (tmp, root)
}

/// Initialise a git repository in `root`, add every generated file, and
/// commit — so the worktree is clean for the `allow_skip_when_clean` test.
fn init_git_repo(root: &Path) {
    let repo = git2::Repository::init(root).unwrap();

    // Stage everything
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();

    let sig = git2::Signature::now("cogito-scale", "scale@cogito.test").unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
        .unwrap();
}

fn count_files(root: &Path) -> usize {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .count()
}

fn init_cogito(root: &Path) -> (cogito_harness_core::Cogito, TempDir, Uuid) {
    let tmp = TempDir::new().unwrap();
    let cogito_dir = tmp.path().join(".cogito");
    let cogito = cogito_harness_core::Cogito::with_project_sqlite(&cogito_dir).unwrap();
    let project = cogito.init_project(root.to_path_buf()).unwrap();
    cogito.register_project(&project).unwrap();
    (cogito, tmp, project.id)
}

/// Percentile calculation on a sorted slice.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len() as f64;
    if n == 0.0 {
        return 0.0;
    }
    let idx = (p / 100.0) * (n - 1.0);
    let lo = idx.floor() as usize;
    let hi = (lo + 1).min(sorted.len() - 1);
    let frac = idx.fract();
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

#[test]
#[ignore]
fn scale_100k() {
    if std::env::var("COGITO_SCALE").as_deref() != Ok("1") {
        eprintln!("Set COGITO_SCALE=1 to run scale benchmarks.");
        return;
    }

    let n = 100_000;
    eprintln!("\n=== Scale test: {n} files ===");
    eprintln!("Generating project tree...");

    let start_gen = std::time::Instant::now();
    let (_project_tmp, project_root) = generate_project(n);
    eprintln!("  Generated in {:?}", start_gen.elapsed());
    let file_count = count_files(&project_root);
    eprintln!("  File count: {file_count}");

    // Initialise a git repo so worktree_changed() sees a clean tree
    eprintln!("Initialising git repository...");
    init_git_repo(&project_root);
    eprintln!("  Git repo ready.");

    let (cogito, _cogito_tmp, project_id) = init_cogito(&project_root);

    // ── worktree_changed cost ─────────────────────────────────────────────
    let wt_start = std::time::Instant::now();
    let _changed = index::worktree_changed(&project_root);
    let wt_ms = ms(wt_start.elapsed());
    eprintln!("  worktree_changed: {:.2} ms", wt_ms);

    // ── Cold sync ─────────────────────────────────────────────────────────
    let cold_start = std::time::Instant::now();
    {
        let conn = cogito.sqlite().unwrap().lock();
        index::sync(&conn, project_id, &project_root, false).unwrap();
    }
    let cold_ms = ms(cold_start.elapsed());
    eprintln!("  cold sync: {:.2} ms", cold_ms);

    // ── Warm sync, no skip ───────────────────────────────────────────────
    let warm_no_skip_start = std::time::Instant::now();
    {
        let conn = cogito.sqlite().unwrap().lock();
        index::sync(&conn, project_id, &project_root, false).unwrap();
    }
    let warm_no_skip_ms = ms(warm_no_skip_start.elapsed());
    eprintln!("  warm sync (no skip): {:.2} ms", warm_no_skip_ms);

    // ── Warm sync, skip when clean ───────────────────────────────────────
    let warm_skip_start = std::time::Instant::now();
    {
        let conn = cogito.sqlite().unwrap().lock();
        index::sync(&conn, project_id, &project_root, true).unwrap();
    }
    let warm_skip_ms = ms(warm_skip_start.elapsed());
    eprintln!("  warm sync (skip=true): {:.2} ms", warm_skip_ms);

    // ── Selective search queries (unique identifiers) ─────────────────────
    let selective_queries = [
        "fn_0", "fn_100", "fn_1000", "fn_9999", "fn_50000", "S0", "S100", "S1000", "S9999",
        "S50000",
    ];
    let selective_times: Vec<f64> = selective_queries
        .iter()
        .cycle()
        .take(100)
        .map(|q| {
            let conn = cogito.sqlite().unwrap().lock();
            let start = std::time::Instant::now();
            let _ = index::search(&conn, project_id, q, 100);
            ms(start.elapsed())
        })
        .collect();

    let mut sel_sorted = selective_times.clone();
    sel_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let sel_p50 = percentile(&sel_sorted, 50.0);
    let sel_p95 = percentile(&sel_sorted, 95.0);
    eprintln!(
        "  selective search p50: {:.3} ms, p95: {:.3} ms",
        sel_p50, sel_p95
    );

    // ── Common-token search queries (appear in thousands of symbols) ───────
    // "fn" matches every Rust function; "pub" matches every public item
    let common_queries = ["fn", "pub", "fn_", "S"];
    let common_times: Vec<f64> = common_queries
        .iter()
        .cycle()
        .take(100)
        .map(|q| {
            let conn = cogito.sqlite().unwrap().lock();
            let start = std::time::Instant::now();
            let _ = index::search(&conn, project_id, q, 100);
            ms(start.elapsed())
        })
        .collect();

    let mut com_sorted = common_times.clone();
    com_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let com_p50 = percentile(&com_sorted, 50.0);
    let com_p95 = percentile(&com_sorted, 95.0);
    eprintln!(
        "  common-token search p50: {:.3} ms, p95: {:.3} ms",
        com_p50, com_p95
    );

    // ── Outline latency ────────────────────────────────────────────────────
    let outline_path = project_root.join("src/sub_00000/file_000000.rs");
    let outline_times: Vec<f64> = (0..100)
        .map(|_| {
            let conn = cogito.sqlite().unwrap().lock();
            let start = std::time::Instant::now();
            let _ = index::outline(&conn, project_id, &outline_path);
            ms(start.elapsed())
        })
        .collect();
    let outline_p50 = percentile(&outline_times, 50.0);

    // ── Print table ────────────────────────────────────────────────────────
    println!();
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!(
        "║               Scale measurement: {n:>6} files                 ║",
        n = n
    );
    println!("╠════════════════════════════════════════════════════════════════╣");
    println!(
        "║  worktree_changed          {:>8.3} ms                        ║",
        wt_ms
    );
    println!(
        "║  cold sync                 {:>8.3} ms                        ║",
        cold_ms
    );
    println!(
        "║  warm sync (no skip)       {:>8.3} ms                        ║",
        warm_no_skip_ms
    );
    println!(
        "║  warm sync (skip=true)      {:>8.3} ms                        ║",
        warm_skip_ms
    );
    println!("╠════════════════════════════════════════════════════════════════╣");
    println!("║  Search — selective (unique identifiers)                    ║");
    println!(
        "║    p50                     {:>8.3} ms                        ║",
        sel_p50
    );
    println!(
        "║    p95                     {:>8.3} ms                        ║",
        sel_p95
    );
    println!("║  Search — common token (e.g. \"fn\", \"pub\")                 ║");
    println!(
        "║    p50                     {:>8.3} ms                        ║",
        com_p50
    );
    println!(
        "║    p95                     {:>8.3} ms                        ║",
        com_p95
    );
    println!("╠════════════════════════════════════════════════════════════════╣");
    println!(
        "║  outline p50                {:>8.3} ms                        ║",
        outline_p50
    );
    println!("╠════════════════════════════════════════════════════════════════╣");
    println!("║  Budgets:                                                 ║");
    let search_budget = if com_p95 <= 50.0 { "PASS" } else { "FAIL" };
    println!(
        "║    search p95 ≤ 50 ms      [{search_budget}]  got {:>7.3} ms              ║",
        com_p95
    );
    let warm_budget = if warm_skip_ms <= 150.0 {
        "PASS"
    } else {
        "FAIL"
    };
    println!(
        "║    warm sync ≤ 150 ms      [{warm_budget}]  got {:>7.3} ms              ║",
        warm_skip_ms
    );
    println!("╚════════════════════════════════════════════════════════════════╝");
    println!();

    // CSV: selective and common p95 separately
    println!(
        "scale,files,cold_ms,warm_no_skip_ms,warm_skip_ms,sel_p50_ms,sel_p95_ms,com_p50_ms,com_p95_ms,outline_p50_ms,worktree_changed_ms"
    );
    println!(
        "{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}",
        n,
        file_count,
        cold_ms,
        warm_no_skip_ms,
        warm_skip_ms,
        sel_p50,
        sel_p95,
        com_p50,
        com_p95,
        outline_p50,
        wt_ms
    );
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
