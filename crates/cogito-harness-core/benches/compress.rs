//! Benchmarks for the Context Compressor
//!
//! Run with: `cargo bench --bench compress`
//!
//! Measures compression performance and savings on synthetic code
//! of varying sizes.

use cogito_harness_core::{Cogito, CompressionOptions};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

/// Generate synthetic code with realistic comment density.
fn synth_code(approx_lines: usize) -> String {
    let mut out = String::new();
    out.push_str("// Auto-generated synthetic code for benchmarking\n\n");

    let mut i = 0;
    while out.lines().count() < approx_lines {
        out.push_str(&format!(
            "// Function {i}: handles case {i} of the synthetic workload\n"
        ));
        out.push_str(&format!("/// Doc comment for fn_{i}\n"));
        out.push_str(&format!(
            "pub fn fn_{i}(x: i32) -> i32 {{\n    if x > {i} {{\n        return x * 2;\n    }}\n    x + {i}\n}}\n\n"
        ));
        i += 1;
    }
    out
}

fn default_options() -> CompressionOptions {
    CompressionOptions {
        max_tokens: 1_000_000,
        preserve_imports: true,
        preserve_types: true,
        hot_zones_priority: false,
    }
}

fn bench_compress_synth(c: &mut Criterion) {
    let mut group = c.benchmark_group("compress_synth");
    let cogito = Cogito::new();

    for lines in [100, 500, 2_000, 10_000].iter() {
        let code = synth_code(*lines);
        let bytes = code.len() as u64;
        group.throughput(Throughput::Bytes(bytes));

        group.bench_with_input(BenchmarkId::from_parameter(lines), lines, |b, &n| {
            b.iter(|| {
                let code = synth_code(n);
                let _ =
                    futures::executor::block_on(cogito.compress_context(&code, default_options()));
            });
        });
    }

    group.finish();
}

fn bench_compress_savings(_c: &mut Criterion) {
    let cogito = Cogito::new();

    for lines in [100, 500, 2_000, 10_000].iter() {
        let code = synth_code(*lines);
        let result =
            futures::executor::block_on(cogito.compress_context(&code, default_options())).unwrap();
        println!(
            "compress_savings: {lines} lines -> {:.1}% saved ({} -> {} tokens)",
            result.savings_percent, result.original_tokens, result.compressed_tokens
        );
    }
}

fn bench_compress_real_world(c: &mut Criterion) {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    let candidate = manifest_dir.join("src/fingerprint/mod.rs");
    if !candidate.exists() {
        eprintln!("Cannot find real-world file, skipping");
        return;
    }
    let code = std::fs::read_to_string(&candidate).expect("read source");
    println!(
        "Real-world compress: {} ({} bytes)",
        candidate.display(),
        code.len()
    );

    let cogito = Cogito::new();
    let result =
        futures::executor::block_on(cogito.compress_context(&code, default_options())).unwrap();
    println!(
        "compress_real_world: {} -> {} tokens ({:.1}% savings)",
        result.original_tokens, result.compressed_tokens, result.savings_percent
    );

    let mut group = c.benchmark_group("compress_real_world");
    group.throughput(Throughput::Bytes(code.len() as u64));
    group.bench_function("fingerprint_mod", |b| {
        b.iter(|| {
            let _ = futures::executor::block_on(cogito.compress_context(&code, default_options()));
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_compress_synth,
    bench_compress_savings,
    bench_compress_real_world
);
criterion_main!(benches);
