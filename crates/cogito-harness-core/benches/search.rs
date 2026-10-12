//! Benchmarks for search operations (tags + decisions)
//!
//! Run with: `cargo bench --bench search`

use cogito_harness_core::{Cogito, decision};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::path::PathBuf;
use tempfile::TempDir;
use uuid::Uuid;

fn setup_with_n_decisions(n: usize) -> (TempDir, Uuid, Cogito) {
    let dir = TempDir::new().unwrap();
    let original = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    let cogito = Cogito::new();
    let project = cogito.init_project(dir.path().to_path_buf()).unwrap();
    let pid = project.id;

    for i in 0..n {
        let d = decision::builder(pid)
            .title(&format!("Decision #{i}: use pattern {}", i % 5))
            .why(&format!(
                "Rationale for decision {i} - scalability, performance"
            ))
            .tag(&format!("tag-{}", i % 10))
            .tag(&format!("domain-{}", i % 3))
            .path(PathBuf::from(format!("src/module_{i}/")))
            .build();
        let _ = cogito.add_decision(d);
    }

    std::env::set_current_dir(&original).unwrap();
    (dir, pid, cogito)
}

fn bench_search_decisions(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_decisions");

    for n in [10, 100, 1000].iter() {
        let (_dir, pid, cogito) = setup_with_n_decisions(*n);

        group.bench_with_input(BenchmarkId::from_parameter(n), n, |b, _| {
            b.iter(|| {
                let _ = cogito.search_decisions(pid, "scalability");
            });
        });
    }

    group.finish();
}

fn bench_search_tags(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_tags");

    for n in [10, 100, 1000].iter() {
        let (_dir, pid, cogito) = setup_with_n_decisions(*n);

        group.bench_with_input(BenchmarkId::from_parameter(n), n, |b, _| {
            b.iter(|| {
                let _ = cogito.search_tags(pid, "tag-5");
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_search_decisions, bench_search_tags);
criterion_main!(benches);
