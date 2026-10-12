//! Cross-language AST vs structural compression benchmark
//!
//! Compares structural and AST-aware compression for Python and Go.
//! Run with: `cargo bench --bench ast_all_langs -- --quick`

use cogito_core::Language;
use cogito_core::compressor::compress_with_language;
use cogito_core::types::CompressionOptions;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

// ============================================================
// Python
// ============================================================

fn synth_python(approx_lines: usize) -> String {
    let mut out = String::new();
    out.push_str("from typing import List, Optional\n\n");

    let mut i = 0;
    while out.lines().count() < approx_lines {
        out.push_str(&format!(
            "def process_{i}(data: List[str]) -> Optional[str]:\n\
             \x20\x20\x20\x20if not data:\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return None\n\
             \x20\x20\x20\x20if len(data) == 0:\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return None\n\
             \x20\x20\x20\x20result = []\n\
             \x20\x20\x20\x20for item in data:\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if item:\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20result.append(item.upper())\n\
             \x20\x20\x20\x20return result[0] if result else None\n\n"
        ));
        i += 1;
    }
    out
}

// ============================================================
// Go
// ============================================================

fn synth_go(approx_lines: usize) -> String {
    let mut out = String::new();
    out.push_str("package main\n\nimport \"fmt\"\n\n");

    let mut i = 0;
    while out.lines().count() < approx_lines {
        out.push_str(&format!(
            "func Process{i}(data []string) (string, error) {{\n\
             \x20\x20\x20\x20if len(data) == 0 {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return \"\", fmt.Errorf(\"empty data\")\n\
             \x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20if data[0] == \"\" {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return \"\", fmt.Errorf(\"empty string\")\n\
             \x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20result := make([]string, 0, len(data))\n\
             \x20\x20\x20\x20for _, item := range data {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if item != \"\" {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20result = append(result, strings.ToUpper(item))\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20if len(result) == 0 {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20return \"\", nil\n\
             \x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20return result[0], nil\n\
             }}\n\n"
        ));
        i += 1;
    }
    out
}

fn run_bench(c: &mut Criterion, name: &str, code: &str, language: Language) {
    let mut group = c.benchmark_group(name);
    for lines in [100, 500, 2000].iter() {
        let code_str: String = match language {
            Language::Python => synth_python(*lines),
            Language::Go => synth_go(*lines),
            _ => code.to_string(),
        };
        let bytes = code_str.len() as u64;
        group.throughput(Throughput::Bytes(bytes));

        group.bench_with_input(BenchmarkId::from_parameter(lines), lines, |b, &n| {
            b.iter(|| {
                let code = match language {
                    Language::Python => synth_python(n),
                    Language::Go => synth_go(n),
                    _ => code.to_string(),
                };
                let opts = CompressionOptions::default();
                let _ = compress_with_language(&code, opts, Some(language));
            });
        });
    }
    group.finish();
}

fn bench_python_ast(c: &mut Criterion) {
    run_bench(c, "compress_ast_python", "", Language::Python);
}

fn bench_go_ast(c: &mut Criterion) {
    run_bench(c, "compress_ast_go", "", Language::Go);
}

fn print_savings() {
    println!("\n=== Savings comparison ===");
    for (lang_name, lang) in &[("Python", Language::Python), ("Go", Language::Go)] {
        println!("\n[{lang_name}]");
        for lines in [100, 500, 2000].iter() {
            let code = match lang {
                Language::Python => synth_python(*lines),
                Language::Go => synth_go(*lines),
                _ => continue,
            };

            let struct_opts = CompressionOptions::default();
            let struct_result = compress_with_language(&code, struct_opts, None).unwrap();

            let ast_opts = CompressionOptions::default();
            let ast_result = compress_with_language(&code, ast_opts, Some(*lang)).unwrap();

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

criterion_group!(benches, bench_python_ast, bench_go_ast, bench_savings,);
criterion_main!(benches);
