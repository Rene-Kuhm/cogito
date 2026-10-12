//! Benchmark: structural vs AST-aware compression for Ruby, PHP, Kotlin, Swift
//!
//! Run with: `cargo bench --bench ast_ruby_php_kotlin_swift -- --quick`

use cogito_core::Language;
use cogito_core::compressor::compress_with_language;
use cogito_core::types::CompressionOptions;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

fn synth_ruby(approx_lines: usize) -> String {
    let mut out = String::new();
    out.push_str("require 'json'\nrequire 'logger'\n\n");

    let mut i = 0;
    while out.lines().count() < approx_lines {
        out.push_str(&format!(
            "class Service{i}\n\
             \x20\x20def initialize(repository, logger)\n\
             \x20\x20\x20\x20@repository = repository\n\
             \x20\x20\x20\x20@logger = logger\n\
             \x20\x20end\n\n\
             \x20\x20def find(id)\n\
             \x20\x20\x20\x20return nil if id <= 0\n\
             \x20\x20\x20\x20user = @repository.find(id)\n\
             \x20\x20\x20\x20if user.nil?\n\
             \x20\x20\x20\x20\x20\x20@logger.warn(\"not found: #{{id}}\")\n\
             \x20\x20\x20\x20\x20\x20return nil\n\
             \x20\x20\x20\x20end\n\
             \x20\x20\x20\x20user\n\
             \x20\x20end\n\n\
             \x20\x20def delete(id)\n\
             \x20\x20\x20\x20user = find(id)\n\
             \x20\x20\x20\x20return false if user.nil?\n\
             \x20\x20\x20\x20@repository.delete(id)\n\
             \x20\x20\x20\x20true\n\
             \x20\x20end\n\
             end\n\n"
        ));
        i += 1;
    }
    out
}

fn synth_php(approx_lines: usize) -> String {
    let mut out = String::new();
    out.push_str("<?php\n\ndeclare(strict_types=1);\n\n");

    let mut i = 0;
    while out.lines().count() < approx_lines {
        out.push_str(&format!(
            "class Service{i}\n\
             {{\n\
             \x20\x20\x20\x20public function __construct(private readonly Repository $repository) {{}}\n\n\
             \x20\x20\x20\x20public function find(int $id): ?User\n\
             \x20\x20\x20\x20{{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if ($id <= 0) {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return null;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20$user = $this->repository->find($id);\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if ($user === null) {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return null;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return $user;\n\
             \x20\x20\x20\x20}}\n\n\
             \x20\x20\x20\x20public function delete(int $id): bool\n\
             \x20\x20\x20\x20{{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if ($this->find($id) === null) {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return false;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20$this->repository->delete($id);\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return true;\n\
             \x20\x20\x20\x20}}\n\
             }}\n\n"
        ));
        i += 1;
    }
    out
}

fn synth_kotlin(approx_lines: usize) -> String {
    let mut out = String::new();
    out.push_str("package com.example\n\nimport kotlin.collections.List\n\n");

    let mut i = 0;
    while out.lines().count() < approx_lines {
        out.push_str(&format!(
            "class Service{i}(private val repository: Repository) {{\n\
             \x20\x20\x20\x20fun find(id: Long): User? {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (id <= 0) {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return null\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20val user = repository.find(id)\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (user == null) {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return null\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return user\n\
             \x20\x20\x20\x20}}\n\n\
             \x20\x20\x20\x20fun delete(id: Long): Boolean {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20val user = find(id) ?: return false\n\
             \x20\x20\x20\x20\x20\x20\x20\x20repository.delete(id)\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return true\n\
             \x20\x20\x20\x20}}\n\n\
             \x20\x20\x20\x20fun listAll(): List<User> {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return repository.findAll()\n\
             \x20\x20\x20\x20}}\n\
             }}\n\n"
        ));
        i += 1;
    }
    out
}

fn synth_swift(approx_lines: usize) -> String {
    let mut out = String::new();
    out.push_str("import Foundation\nimport Combine\n\n");

    let mut i = 0;
    while out.lines().count() < approx_lines {
        out.push_str(&format!(
            "final class Service{i} {{\n\
             \x20\x20\x20\x20private let repository: Repository\n\n\
             \x20\x20\x20\x20init(repository: Repository) {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20self.repository = repository\n\
             \x20\x20\x20\x20}}\n\n\
             \x20\x20\x20\x20func find(_ id: Int) -> User? {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20guard id > 0 else {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return nil\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return repository.find(id)\n\
             \x20\x20\x20\x20}}\n\n\
             \x20\x20\x20\x20func delete(_ id: Int) -> Bool {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20guard find(id) != nil else {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return false\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20repository.delete(id)\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return true\n\
             \x20\x20\x20\x20}}\n\
             }}\n\n"
        ));
        i += 1;
    }
    out
}

fn synth(lang: Language, lines: usize) -> String {
    match lang {
        Language::Ruby => synth_ruby(lines),
        Language::Php => synth_php(lines),
        Language::Kotlin => synth_kotlin(lines),
        Language::Swift => synth_swift(lines),
        _ => String::new(),
    }
}

fn run_bench(c: &mut Criterion, name: &str, language: Language) {
    let mut group = c.benchmark_group(name);
    for lines in [100, 500, 2000].iter() {
        let code = synth(language, *lines);
        group.throughput(Throughput::Bytes(code.len() as u64));

        group.bench_with_input(BenchmarkId::from_parameter(lines), lines, |b, &n| {
            b.iter(|| {
                let code = synth(language, n);
                let opts = CompressionOptions::default();
                let _ = compress_with_language(&code, opts, Some(language));
            });
        });
    }
    group.finish();
}

fn bench_ruby(c: &mut Criterion) {
    run_bench(c, "compress_ast_ruby", Language::Ruby);
}
fn bench_php(c: &mut Criterion) {
    run_bench(c, "compress_ast_php", Language::Php);
}
fn bench_kotlin(c: &mut Criterion) {
    run_bench(c, "compress_ast_kotlin", Language::Kotlin);
}
fn bench_swift(c: &mut Criterion) {
    run_bench(c, "compress_ast_swift", Language::Swift);
}

fn print_savings() {
    println!("\n=== Savings comparison ===");
    for (lang_name, lang) in [
        ("Ruby", Language::Ruby),
        ("PHP", Language::Php),
        ("Kotlin", Language::Kotlin),
        ("Swift", Language::Swift),
    ] {
        println!("\n[{lang_name}]");
        for lines in [100, 500, 2000].iter() {
            let code = synth(lang, *lines);

            let struct_opts = CompressionOptions::default();
            let struct_result = compress_with_language(&code, struct_opts, None).unwrap();

            let ast_opts = CompressionOptions::default();
            let ast_result = compress_with_language(&code, ast_opts, Some(lang)).unwrap();

            println!(
                "  {lines:>5} lines: structural {s_struct:.1}% ({} -> {} bytes), AST {s_ast:.1}% ({} -> {} bytes)",
                struct_result.original_tokens,
                struct_result.compressed_tokens,
                ast_result.original_tokens,
                ast_result.compressed_tokens,
                s_struct = struct_result.savings_percent,
                s_ast = ast_result.savings_percent,
            );
        }
    }
}

fn bench_savings(_c: &mut Criterion) {
    print_savings();
}

criterion_group!(
    benches,
    bench_ruby,
    bench_php,
    bench_kotlin,
    bench_swift,
    bench_savings,
);
criterion_main!(benches);
