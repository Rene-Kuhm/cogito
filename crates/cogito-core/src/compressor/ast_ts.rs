//! AST-aware TypeScript/JavaScript compressor
//!
//! Uses `tree-sitter-typescript` + **byte-range replacement**: we find the
//! byte range of every function/method/arrow body and replace it with
//! `{ /* ... */ }`, leaving everything else verbatim.
//!
//! This preserves imports, interfaces, type aliases, class signatures,
//! decorators, and formatting exactly.
//!
//! Preserved: imports, interfaces, type aliases, classes, fields, enums,
//! decorators, function signatures.
//! Collapsed: function bodies, method bodies, arrow function bodies.

use super::ast_common::{apply_replacements, collect_body_ranges};
use crate::types::CompressionOptions;
use anyhow::{Result, anyhow};
use std::sync::OnceLock;
use tree_sitter::Parser;

/// Result of TS AST compression
#[derive(Debug, Clone)]
pub struct TsCompressResult {
    pub content: String,
    pub functions_collapsed: usize,
    pub classes_preserved: usize,
    pub interfaces_preserved: usize,
    pub type_aliases: usize,
}

/// Compress TypeScript/JavaScript source using AST awareness.
pub fn compress_typescript(
    content: &str,
    _options: &CompressionOptions,
) -> Result<TsCompressResult> {
    let mut parser = Parser::new();
    parser
        .set_language(&get_language())
        .map_err(|e| anyhow!("Failed to set tree-sitter language: {e}"))?;

    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow!("Failed to parse TypeScript"))?;

    let ranges = collect_body_ranges(tree.root_node(), is_function_node);
    let (compressed, applied) = apply_replacements(content, &ranges);

    // Count preserved declarations by walking the tree once.
    let mut stats = TsStats::default();
    count_declarations(tree.root_node(), &mut stats);

    Ok(TsCompressResult {
        content: compressed,
        functions_collapsed: applied,
        classes_preserved: stats.classes,
        interfaces_preserved: stats.interfaces,
        type_aliases: stats.type_aliases,
    })
}

#[derive(Default)]
struct TsStats {
    classes: usize,
    interfaces: usize,
    type_aliases: usize,
}

/// Node kinds that represent a callable whose body should be collapsed.
fn is_function_node(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "function_expression"
            | "generator_function"
            | "generator_function_declaration"
            | "method_definition"
            | "arrow_function"
    )
}

fn count_declarations(node: tree_sitter::Node, stats: &mut TsStats) {
    match node.kind() {
        "class_declaration" | "abstract_class_declaration" => stats.classes += 1,
        "interface_declaration" => stats.interfaces += 1,
        "type_alias_declaration" => stats.type_aliases += 1,
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_declarations(child, stats);
    }
}

static LANGUAGE: OnceLock<tree_sitter::Language> = OnceLock::new();

fn get_language() -> tree_sitter::Language {
    LANGUAGE
        .get_or_init(|| tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapse_function() {
        let code = r#"
            function add(a: number, b: number): number {
                const result = a + b;
                console.log("debug", result);
                if (a > 100) {
                    return b;
                }
                return result * 2;
            }
        "#;
        let opts = CompressionOptions::default();
        let result = compress_typescript(code, &opts).unwrap();
        assert!(result.functions_collapsed >= 1);
        assert!(result.content.contains("function add"));
        assert!(result.content.contains("a: number"));
        assert!(result.content.contains("b: number"));
        assert!(result.content.contains("{ /* ... */ }"));
        // Body should NOT be in output
        assert!(!result.content.contains("const result"));
        assert!(!result.content.contains("console.log"));
    }

    #[test]
    fn test_collapse_class_with_methods() {
        let code = r#"
            class User {
                private id: number;
                public name: string;

                constructor(id: number, name: string) {
                    this.id = id;
                    this.name = name;
                }

                public validate(): boolean {
                    if (this.name.length === 0) {
                        return false;
                    }
                    return true;
                }

                public getId(): number {
                    return this.id;
                }
            }
        "#;
        let opts = CompressionOptions::default();
        let result = compress_typescript(code, &opts).unwrap();
        assert!(result.classes_preserved >= 1);
        assert!(result.functions_collapsed >= 3);
        assert!(result.content.contains("class User"));
        assert!(result.content.contains("private id: number"));
        assert!(result.content.contains("validate(): boolean"));
        assert!(result.content.contains("getId(): number"));
        // Method bodies should NOT be in output
        assert!(!result.content.contains("this.name.length === 0"));
        assert!(!result.content.contains("this.id = id"));
    }

    #[test]
    fn test_collapse_interface_and_type_alias() {
        let code = r#"
            interface User {
                id: number;
                name: string;
                email?: string;
                validate(): boolean;
            }

            type Status = "active" | "inactive";
            type UserMap = Map<string, User>;
        "#;
        let opts = CompressionOptions::default();
        let result = compress_typescript(code, &opts).unwrap();
        assert!(result.interfaces_preserved >= 1);
        assert!(result.type_aliases >= 2);
        assert!(result.content.contains("interface User"));
        assert!(result.content.contains("validate(): boolean"));
        assert!(result.content.contains("type Status"));
        assert!(result.content.contains("type UserMap"));
    }

    #[test]
    fn test_preserves_imports() {
        let code = r#"
            import { useState, useEffect } from 'react';
            import type { User } from './types';

            export function useUser() {
                const [user, setUser] = useState(null);
                return user;
            }
        "#;
        let opts = CompressionOptions::default();
        let result = compress_typescript(code, &opts).unwrap();
        assert!(result.content.contains("import { useState"));
        assert!(result.content.contains("import type { User }"));
    }

    #[test]
    fn test_preserves_arrow_function_signature() {
        let code = r#"
            const add = (a: number, b: number): number => {
                const result = a + b;
                console.log(result);
                return result;
            };
        "#;
        let opts = CompressionOptions::default();
        let result = compress_typescript(code, &opts).unwrap();
        // Should contain the arrow function signature
        assert!(result.content.contains("a: number"));
        assert!(result.content.contains("b: number"));
        // Body should NOT be in output
        assert!(!result.content.contains("const result"));
        assert!(!result.content.contains("console.log"));
    }

    #[test]
    fn test_real_world_module_savings() {
        let code = r#"
            import { Injectable } from '@nestjs/common';
            import { UserRepository } from './user.repository';

            export interface User {
                id: number;
                name: string;
                email: string;
            }

            @Injectable()
            export class UserService {
                constructor(private readonly repo: UserRepository) {}

                async findById(id: number): Promise<User | null> {
                    if (id <= 0) {
                        return null;
                    }
                    const user = await this.repo.findOne({ where: { id } });
                    if (!user) {
                        throw new Error('not found');
                    }
                    return user;
                }

                async create(name: string, email: string): Promise<User> {
                    const exists = await this.repo.findOne({ where: { email } });
                    if (exists) {
                        throw new Error('already exists');
                    }
                    return this.repo.save({ name, email });
                }

                async update(id: number, data: Partial<User>): Promise<User> {
                    const user = await this.findById(id);
                    if (!user) {
                        throw new Error('not found');
                    }
                    return this.repo.save({ ...user, ...data });
                }
            }
        "#;
        let opts = CompressionOptions::default();
        let result = compress_typescript(code, &opts).unwrap();
        let orig = code.len();
        let comp = result.content.len();
        let savings = 100.0 * (1.0 - comp as f64 / orig as f64);
        println!(
            "Real-world TS module: {} -> {} bytes ({:.1}% savings)",
            orig, comp, savings
        );

        assert!(savings > 30.0, "Expected >30% savings, got {savings:.1}%");

        assert!(result.content.contains("interface User"));
        assert!(result.content.contains("class UserService"));
        assert!(result.content.contains("findById"));
        assert!(result.content.contains("create"));
        assert!(result.content.contains("update"));
    }
}
