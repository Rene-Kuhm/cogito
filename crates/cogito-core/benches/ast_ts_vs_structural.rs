//! Benchmark: structural vs AST-aware TypeScript compression
//!
//! Run with: `cargo bench --bench ast_ts_vs_structural -- --quick`

use cogito_core::Language;
use cogito_core::compressor::compress_with_language;
use cogito_core::types::CompressionOptions;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

/// Generate realistic TypeScript code with implementation bodies.
fn synth_typescript(approx_lines: usize) -> String {
    let mut out = String::new();
    out.push_str("import { Injectable } from '@nestjs/common';\n\n");

    let mut i = 0;
    while out.lines().count() < approx_lines {
        // Interface
        out.push_str(&format!(
            "export interface Entity{i} {{\n\
             \x20\x20\x20\x20id: number;\n\
             \x20\x20\x20\x20name: string;\n\
             \x20\x20\x20\x20createdAt: Date;\n\
             }}\n\n"
        ));
        // Class with methods
        out.push_str(&format!(
            "@Injectable()\n\
             export class Service{i} {{\n\
             \x20\x20\x20\x20constructor(private readonly repo: Repository) {{}}\n\n\
             \x20\x20\x20\x20async findById(id: number): Promise<Entity{i} | null> {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (id <= 0) {{ return null; }}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return this.repo.findOne({{ where: {{ id }} }});\n\
             \x20\x20\x20\x20}}\n\n\
             \x20\x20\x20\x20async create(data: Partial<Entity{i}>): Promise<Entity{i}> {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20const existing = await this.repo.findOne({{ where: {{ id: data.id }} }});\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (existing) {{ throw new Error('exists'); }}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return this.repo.save(data);\n\
             \x20\x20\x20\x20}}\n\n\
             \x20\x20\x20\x20async update(id: number, data: Partial<Entity{i}>): Promise<Entity{i}> {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20const entity = await this.findById(id);\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (!entity) {{ throw new Error('not found'); }}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return this.repo.save({{ ...entity, ...data }});\n\
             \x20\x20\x20\x20}}\n\n\
             \x20\x20\x20\x20async delete(id: number): Promise<void> {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20await this.repo.delete({{ id }});\n\
             \x20\x20\x20\x20}}\n\
             }}\n\n"
        ));
        i += 1;
    }
    out
}

fn run_bench(c: &mut Criterion, name: &str, use_ast: bool) {
    let mut group = c.benchmark_group(name);
    for lines in [100, 500, 2000].iter() {
        let code = synth_typescript(*lines);
        let bytes = code.len() as u64;
        group.throughput(Throughput::Bytes(bytes));

        group.bench_with_input(BenchmarkId::from_parameter(lines), lines, |b, &n| {
            b.iter(|| {
                let code = synth_typescript(n);
                let opts = CompressionOptions::default();
                let lang = if use_ast {
                    Some(Language::TypeScript)
                } else {
                    None
                };
                let _ = compress_with_language(&code, opts, lang);
            });
        });
    }
    group.finish();
}

fn bench_structural(c: &mut Criterion) {
    run_bench(c, "compress_structural_ts", false);
}

fn bench_ast(c: &mut Criterion) {
    run_bench(c, "compress_ast_ts", true);
}

fn print_savings() {
    println!("\n=== Savings comparison (synthetic TypeScript) ===");
    for lines in [100, 500, 2000].iter() {
        let code = synth_typescript(*lines);

        let struct_opts = CompressionOptions::default();
        let struct_result = compress_with_language(&code, struct_opts, None).unwrap();

        let ast_opts = CompressionOptions::default();
        let ast_result =
            compress_with_language(&code, ast_opts, Some(Language::TypeScript)).unwrap();

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
