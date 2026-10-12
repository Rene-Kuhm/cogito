//! Benchmark: structural vs AST-aware compression for C# and Java
//!
//! Run with: `cargo bench --bench ast_csharp_java -- --quick`

use cogito_harness_core::Language;
use cogito_harness_core::compressor::compress_with_language;
use cogito_harness_core::types::CompressionOptions;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

fn synth_csharp(approx_lines: usize) -> String {
    let mut out = String::new();
    out.push_str("using System;\nusing System.Collections.Generic;\n\n");

    let mut i = 0;
    while out.lines().count() < approx_lines {
        out.push_str(&format!(
            "public class Service{i}\n\
             {{\n\
             \x20\x20\x20\x20private readonly Dictionary<int, string> _cache = new();\n\n\
             \x20\x20\x20\x20public string Get(int id)\n\
             \x20\x20\x20\x20{{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (_cache.TryGetValue(id, out var cached))\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return cached;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20var user = LoadUser(id);\n\
             \x20\x20\x20\x20\x20\x20\x20\x20_cache[id] = user;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return user;\n\
             \x20\x20\x20\x20}}\n\n\
             \x20\x20\x20\x20public bool Delete(int id)\n\
             \x20\x20\x20\x20{{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (!_cache.ContainsKey(id))\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return false;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20_cache.Remove(id);\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return true;\n\
             \x20\x20\x20\x20}}\n\n\
             \x20\x20\x20\x20private string LoadUser(int id) {{ return $\"user-{{id}}\"; }}\n\
             }}\n\n"
        ));
        i += 1;
    }
    out
}

fn synth_java(approx_lines: usize) -> String {
    let mut out = String::new();
    out.push_str("package com.example;\n\nimport java.util.HashMap;\nimport java.util.Map;\n\n");

    let mut i = 0;
    while out.lines().count() < approx_lines {
        out.push_str(&format!(
            "public class Service{i} {{\n\
             \x20\x20\x20\x20private final Map<Integer, String> cache = new HashMap<>();\n\n\
             \x20\x20\x20\x20public String get(int id) {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (cache.containsKey(id)) {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return cache.get(id);\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20String user = loadUser(id);\n\
             \x20\x20\x20\x20\x20\x20\x20\x20cache.put(id, user);\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return user;\n\
             \x20\x20\x20\x20}}\n\n\
             \x20\x20\x20\x20public boolean delete(int id) {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (!cache.containsKey(id)) {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return false;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20cache.remove(id);\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return true;\n\
             \x20\x20\x20\x20}}\n\n\
             \x20\x20\x20\x20private String loadUser(int id) {{ return \"user-\" + id; }}\n\
             }}\n\n"
        ));
        i += 1;
    }
    out
}

fn run_bench(c: &mut Criterion, name: &str, language: Language) {
    let mut group = c.benchmark_group(name);
    for lines in [100, 500, 2000].iter() {
        let code = match language {
            Language::CSharp => synth_csharp(*lines),
            Language::Java => synth_java(*lines),
            _ => String::new(),
        };
        let bytes = code.len() as u64;
        group.throughput(Throughput::Bytes(bytes));

        group.bench_with_input(BenchmarkId::from_parameter(lines), lines, |b, &n| {
            b.iter(|| {
                let code = match language {
                    Language::CSharp => synth_csharp(n),
                    Language::Java => synth_java(n),
                    _ => String::new(),
                };
                let opts = CompressionOptions::default();
                let _ = compress_with_language(&code, opts, Some(language));
            });
        });
    }
    group.finish();
}

fn bench_csharp(c: &mut Criterion) {
    run_bench(c, "compress_ast_csharp", Language::CSharp);
}

fn bench_java(c: &mut Criterion) {
    run_bench(c, "compress_ast_java", Language::Java);
}

fn print_savings() {
    println!("\n=== Savings comparison ===");
    for (lang_name, lang) in [("C#", Language::CSharp), ("Java", Language::Java)] {
        println!("\n[{lang_name}]");
        for lines in [100, 500, 2000].iter() {
            let code = match lang {
                Language::CSharp => synth_csharp(*lines),
                Language::Java => synth_java(*lines),
                _ => continue,
            };

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

criterion_group!(benches, bench_csharp, bench_java, bench_savings);
criterion_main!(benches);
