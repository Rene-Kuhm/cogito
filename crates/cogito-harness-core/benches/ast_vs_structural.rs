//! Benchmark: structural vs AST-aware compression
//!
//! Run with: `cargo bench --bench ast_vs_structural -- --quick`

use cogito_harness_core::Language;
use cogito_harness_core::compressor::compress_with_language;
use cogito_harness_core::types::CompressionOptions;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

/// Generate realistic Rust code with implementation bodies.
fn synth_rust(approx_lines: usize) -> String {
    let mut out = String::new();
    out.push_str("use std::collections::HashMap;\n\n");

    let mut i = 0;
    while out.lines().count() < approx_lines {
        // Struct
        out.push_str(&format!(
            "pub struct Module{i} {{\n    pub id: u64,\n    pub name: String,\n    pub data: HashMap<String, String>,\n}}\n\n"
        ));
        // Impl with method bodies
        out.push_str(&format!(
            "impl Module{i} {{\n\
             \x20\x20\x20\x20pub fn new(id: u64, name: String) -> Self {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20Self {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20id,\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20name,\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20data: HashMap::new(),\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20}}\n\n\
             \x20\x20\x20\x20pub fn validate(&self) -> bool {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if self.name.is_empty() {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return false;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if self.id == 0 {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return false;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20true\n\
             \x20\x20\x20\x20}}\n\n\
             \x20\x20\x20\x20pub fn process(&mut self, key: String, value: String) {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20self.data.insert(key, value);\n\
             \x20\x20\x20\x20}}\n\
             }}\n\n"
        ));
        i += 1;
    }
    out
}

/// Run benchmark for a given compressor.
fn run_bench(c: &mut Criterion, name: &str, use_ast: bool) {
    let mut group = c.benchmark_group(name);
    for lines in [100, 500, 2000].iter() {
        let code = synth_rust(*lines);
        let bytes = code.len() as u64;
        group.throughput(Throughput::Bytes(bytes));

        group.bench_with_input(BenchmarkId::from_parameter(lines), lines, |b, &n| {
            b.iter(|| {
                let code = synth_rust(n);
                let opts = CompressionOptions::default();
                let lang = if use_ast { Some(Language::Rust) } else { None };
                let _ = compress_with_language(&code, opts, lang);
            });
        });
    }
    group.finish();
}

fn bench_structural(c: &mut Criterion) {
    run_bench(c, "compress_structural", false);
}

fn bench_ast(c: &mut Criterion) {
    run_bench(c, "compress_ast_rust", true);
}

fn print_savings() {
    println!("\n=== Savings comparison (synthetic Rust) ===");
    for lines in [100, 500, 2000].iter() {
        let code = synth_rust(*lines);

        let struct_opts = CompressionOptions::default();
        let struct_result = compress_with_language(&code, struct_opts, None).unwrap();

        let ast_opts = CompressionOptions::default();
        let ast_result = compress_with_language(&code, ast_opts, Some(Language::Rust)).unwrap();

        println!(
            "{lines:>5} lines: structural {s_struct:.1}% ({} -> {} bytes), AST {s_ast:.1}% ({} -> {} bytes)",
            struct_result.original_tokens,
            struct_result.compressed_tokens,
            ast_result.original_tokens,
            ast_result.compressed_tokens,
            s_struct = struct_result.savings_percent,
            s_ast = ast_result.savings_percent,
        );
    }
}

fn bench_savings(_c: &mut Criterion) {
    print_savings();
}

criterion_group!(benches, bench_structural, bench_ast, bench_savings,);
criterion_main!(benches);
