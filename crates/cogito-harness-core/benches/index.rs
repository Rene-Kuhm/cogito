//! Benchmarks for the file and symbol index.
//!
//! Generates synthetic trees of N files and measures cold/warm sync and search.

use cogito_harness_core::Cogito;
use cogito_harness_core::index;
use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use uuid::Uuid;

/// Generate a synthetic project with N files.
fn generate_project(n: usize) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();

    // Generate N Rust files with various declarations
    for i in 0..n {
        let content = format!(
            r#"
/// Module {0} documentation.
pub mod module_{0} {{
    /// Type {0}A definition.
    pub struct Type{0}A {{
        pub field_a: i32,
        pub field_b: String,
    }}

    /// Type {0}B definition.
    pub struct Type{0}B {{
        pub value: Vec<i32>,
    }}

    /// Function {0}A implementation.
    pub fn function{0}A(a: i32, b: i32) -> i32 {{
        a + b
    }}

    /// Function {0}B implementation.
    pub fn function{0}B(x: f64) -> f64 {{
        x * 2.0
    }}

    /// Constant {0}A.
    pub const CONSTANT_{0}A: i32 = 42;

    /// Trait {0}A definition.
    pub trait Trait{0}A {{
        fn method_a(&self) -> i32;
        fn method_b(&self, x: i32) -> i32;
    }}

    impl Trait{0}A for Type{0}A {{
        fn method_a(&self) -> i32 {{
            self.field_a
        }}
        fn method_b(&self, x: i32) -> i32 {{
            self.field_a + x
        }}
    }}
}}
"#,
            i
        );
        fs::write(src.join(format!("mod_{}.rs", i)), content).unwrap();
    }

    // Add Cargo.toml
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"bench\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let root = tmp.path().to_path_buf();
    (tmp, root)
}

/// Initialize cogito for benchmarking.
fn init_cogito(root: &Path) -> (Cogito, TempDir, Uuid) {
    let tmp = TempDir::new().unwrap();
    let cogito_dir = tmp.path().join(".cogito");
    let cogito = Cogito::with_project_sqlite(&cogito_dir).unwrap();
    let project = cogito.init_project(root.to_path_buf()).unwrap();
    cogito.register_project(&project).unwrap();
    (cogito, tmp, project.id)
}

fn bench_index(c: &mut Criterion) {
    // Parameters: N files
    let params = vec![1000, 10000];
    // Set to 100000 for scale testing when needed
    // let params = vec![1000, 10000, 100000];

    let mut group = c.benchmark_group("index");

    for &n in &params {
        let (project_tmp, project_root) = generate_project(n);
        let (cogito, _cogito_tmp, project_id) = init_cogito(&project_root);

        // Measure cold sync (first run, empty index)
        let conn = cogito.sqlite().unwrap().lock();
        group.throughput(Throughput::Elements(n as u64));

        group.bench_function(BenchmarkId::new("sync_cold", n), |b| {
            // Use a fresh connection for cold measurement
            let (cg, _ct, pid) = {
                let (_p_tmp, p_root) = generate_project(n);
                let (cg, ct, pid) = init_cogito(&p_root);
                (cg, ct, pid)
            };
            let conn = cg.sqlite().unwrap().lock();
            let project_root = project_root.clone();
            b.iter(|| {
                index::sync(black_box(&conn), black_box(pid), black_box(&project_root)).unwrap();
            });
        });

        // Measure warm sync (index already populated, no changes)
        group.bench_function(BenchmarkId::new("sync_warm", n), |b| {
            // First, populate the index
            index::sync(&conn, project_id, &project_root).unwrap();
            // Now measure warm sync
            b.iter(|| {
                index::sync(
                    black_box(&conn),
                    black_box(project_id),
                    black_box(&project_root),
                )
                .unwrap();
            });
        });

        // Measure search
        group.bench_function(BenchmarkId::new("search", n), |b| {
            // Ensure index is populated
            index::sync(&conn, project_id, &project_root).unwrap();
            b.iter(|| {
                index::search(
                    black_box(&conn),
                    black_box(project_id),
                    black_box("function"),
                    black_box(100),
                )
                .unwrap();
            });
        });

        drop(conn);
        drop(project_tmp);
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10).warm_up_time(std::time::Duration::from_secs(1));
    targets = bench_index
}
criterion_main!(benches);
