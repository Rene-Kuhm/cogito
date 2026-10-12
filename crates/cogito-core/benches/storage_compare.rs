//! Storage backend comparison: JSON files vs SQLite
//!
//! Run with: `cargo bench --bench storage_compare -- --quick`
//!
//! Measures the same operations against both backends to quantify the
//! real-world impact of the SQLite migration.

use cogito_core::{Cogito, decision};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::path::PathBuf;
use tempfile::TempDir;
use uuid::Uuid;

const COUNTS: &[usize] = &[10, 100, 1000];

/// Build a Cogito instance backed by SQLite in a temp dir.
fn sqlite_cogito() -> (TempDir, Cogito) {
    let dir = TempDir::new().unwrap();
    let cogito = Cogito::with_project_sqlite(&dir.path().join(".cogito")).unwrap();
    (dir, cogito)
}

/// Build a Cogito instance using the legacy JSON backend (no SQLite).
fn json_cogito() -> (TempDir, Cogito) {
    let dir = TempDir::new().unwrap();
    std::env::set_current_dir(&dir).unwrap();
    let cogito = Cogito::new();
    (dir, cogito)
}

/// Build N decisions, return them so we can insert into both backends.
fn make_decisions(project_id: Uuid, n: usize) -> Vec<cogito_core::types::Decision> {
    (0..n)
        .map(|i| {
            decision::builder(project_id)
                .title(&format!("Decision #{i}: use pattern {}", i % 10))
                .why(&format!(
                    "Rationale for decision {i} - scalability, performance, maintainability"
                ))
                .tag(&format!("tag-{}", i % 5))
                .tag(&format!("domain-{}", i % 3))
                .path(PathBuf::from(format!("src/module_{i}/")))
                .alternative(&format!("alt-{}", i % 4))
                .build()
        })
        .collect()
}

// ============================================================
// WRITE throughput
// ============================================================

fn bench_write_json(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_decisions_json");
    for &n in COUNTS {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                let (_dir, cogito) = json_cogito();
                let project_id = Uuid::new_v4();
                let decisions = make_decisions(project_id, n);
                let count = decisions.len();
                futures::executor::block_on(async {
                    for d in decisions {
                        let _ = cogito.add_decision(d).await;
                    }
                });
                count
            });
        });
    }
    group.finish();
}

fn bench_write_sqlite(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_decisions_sqlite");
    for &n in COUNTS {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                let (_dir, cogito) = sqlite_cogito();
                let project_id = Uuid::new_v4();
                let decisions = make_decisions(project_id, n);
                let count = decisions.len();
                futures::executor::block_on(async {
                    for d in decisions {
                        let _ = cogito.add_decision(d).await;
                    }
                });
                count
            });
        });
    }
    group.finish();
}

// ============================================================
// READ throughput (exact match)
// ============================================================

fn bench_read_json(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_decisions_json");
    for &n in COUNTS {
        let (_dir, cogito) = json_cogito();
        let project_id = Uuid::new_v4();
        // Pre-populate
        let decisions = make_decisions(project_id, n);
        futures::executor::block_on(async {
            for d in decisions {
                let _ = cogito.add_decision(d).await;
            }
        });

        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _n| {
            b.iter(|| futures::executor::block_on(cogito.search_decisions(project_id, "")));
        });
    }
    group.finish();
}

fn bench_read_sqlite(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_decisions_sqlite");
    for &n in COUNTS {
        let (_dir, cogito) = sqlite_cogito();
        let project_id = Uuid::new_v4();
        // Pre-populate
        let decisions = make_decisions(project_id, n);
        futures::executor::block_on(async {
            for d in decisions {
                let _ = cogito.add_decision(d).await;
            }
        });

        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _n| {
            b.iter(|| futures::executor::block_on(cogito.search_decisions(project_id, "")));
        });
    }
    group.finish();
}

// ============================================================
// SEARCH with stemming (SQLite-only feature)
// ============================================================

fn bench_search_with_stemming(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_with_stemming_sqlite");
    let (_dir, cogito) = sqlite_cogito();
    let project_id = Uuid::new_v4();
    let decisions = make_decisions(project_id, 1000);
    futures::executor::block_on(async {
        for d in decisions {
            let _ = cogito.add_decision(d).await;
        }
    });

    // "running" should match "scalability" via porter stemming... actually no.
    // But "scalable" should match "scalability" via stemming.
    // "alternativ" should match "alternative" via stemming.
    // Let's use a search that demonstrably hits stemming.
    for query in &["scalable", "performance", "decision"] {
        group.bench_with_input(BenchmarkId::from_parameter(query), query, |b, q| {
            b.iter(|| futures::executor::block_on(cogito.search_decisions(project_id, q)));
        });
    }
    group.finish();
}

// ============================================================
// FILE SIZE on disk
// ============================================================

fn measure_file_sizes() {
    use cogito_core::persistence;
    use std::fs;

    let n = 1000;
    let project_id = Uuid::new_v4();

    // JSON — write directly via the persistence module to avoid the
    // global SQLITE static leaking between bench iterations.
    {
        let dir = TempDir::new().unwrap();
        let cogito_dir = dir.path().join(".cogito");
        fs::create_dir_all(&cogito_dir).unwrap();

        let decisions = make_decisions(project_id, n);
        let all: Vec<_> = decisions.iter().map(|d| (d.id, d.clone())).collect();
        let _ = persistence::save_decisions(&cogito_dir, &decisions);

        let json_path = cogito_dir.join("decisions.json");
        let size = fs::metadata(&json_path).map(|m| m.len()).unwrap_or(0);
        println!(
            "FILE_SIZE_JSON: {n} decisions -> {size} bytes ({:.1} bytes/decision)",
            size as f64 / n as f64
        );
        let _ = all; // suppress unused
    }

    // SQLite
    {
        let (dir, cogito) = sqlite_cogito();
        let project_id = Uuid::new_v4();
        let decisions = make_decisions(project_id, n);
        futures::executor::block_on(async {
            for d in decisions {
                let _ = cogito.add_decision(d).await;
            }
        });

        // Run a CHECKPOINT to consolidate WAL into main DB
        if let Some(sqlite) = cogito.sqlite() {
            let _ = sqlite
                .lock()
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }

        let db_path = dir.path().join(".cogito/state.db");
        let main = fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
        let wal = fs::metadata(format!("{}-wal", db_path.display()))
            .map(|m| m.len())
            .unwrap_or(0);
        let shm = fs::metadata(format!("{}-shm", db_path.display()))
            .map(|m| m.len())
            .unwrap_or(0);
        let total = main + wal + shm;
        println!(
            "FILE_SIZE_SQLITE: {n} decisions -> {total} bytes ({:.1} bytes/decision)",
            total as f64 / n as f64
        );
    }
}

fn bench_file_size(_c: &mut Criterion) {
    measure_file_sizes();
}

criterion_group!(
    benches,
    bench_write_json,
    bench_write_sqlite,
    bench_read_json,
    bench_read_sqlite,
    bench_search_with_stemming,
    bench_file_size,
);
criterion_main!(benches);
