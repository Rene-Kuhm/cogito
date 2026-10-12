//! AST-aware TypeScript/JavaScript compressor
//!
//! Uses `tree-sitter-typescript` to parse the source into an AST, then
//! walks the tree collapsing function/method bodies while preserving
//! signatures, types, and declarations.
//!
//! The result is **not valid TypeScript** in the sense of being
//! compilable, but it is **syntactically faithful**: every signature,
//! type, import, and export is preserved.

use crate::types::CompressionOptions;
use anyhow::{Result, anyhow};
use std::sync::OnceLock;
use tree_sitter::{Node, Parser};

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

    let mut stats = TsStats::default();
    let mut out = String::with_capacity(content.len() / 2);

    let root = tree.root_node();
    walk(root, content, &mut out, &mut stats);

    Ok(TsCompressResult {
        content: out,
        functions_collapsed: stats.functions,
        classes_preserved: stats.classes,
        interfaces_preserved: stats.interfaces,
        type_aliases: stats.type_aliases,
    })
}

#[derive(Default)]
struct TsStats {
    functions: usize,
    classes: usize,
    interfaces: usize,
    type_aliases: usize,
}

/// Cached tree-sitter language (parsing is expensive to set up).
static LANGUAGE: OnceLock<tree_sitter::Language> = OnceLock::new();

fn get_language() -> tree_sitter::Language {
    LANGUAGE
        .get_or_init(|| tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .clone()
}

/// Walk the tree and render a compressed version
fn walk(node: Node, source: &str, out: &mut String, stats: &mut TsStats) {
    match node.kind() {
        "program" => {
            // Top-level: render each statement
            render_children(node, source, out, stats, 0);
        }
        _ => {
            // Fallback: just render the text of this node
            out.push_str(&node_text(node, source));
        }
    }
}

fn render_children(node: Node, source: &str, out: &mut String, stats: &mut TsStats, indent: usize) {
    let mut cursor = node.walk();
    let mut needs_newline = false;

    for child in node.children(&mut cursor) {
        if render_node(child, source, out, stats, indent) {
            needs_newline = true;
        }
    }

    if needs_newline && indent == 0 {
        // Don't double-newline at the end
        while out.ends_with("\n\n") {
            out.pop();
        }
    }
}

/// Render a single node. Returns true if a newline should follow.
fn render_node(
    node: Node,
    source: &str,
    out: &mut String,
    stats: &mut TsStats,
    indent: usize,
) -> bool {
    let text = node_text(node, source);
    let pad = "  ".repeat(indent);

    match node.kind() {
        // Preserve imports
        "import_statement" | "import_declaration" => {
            out.push_str(&text);
            out.push('\n');
            false
        }
        // Preserve exports (whole statement)
        "export_statement" => {
            render_export(node, source, out, stats, indent);
            true
        }
        // Function declarations
        "function_declaration" | "generator_function_declaration" => {
            stats.functions += 1;
            render_function(node, source, out, stats, indent);
            true
        }
        // Methods in classes/interfaces
        "method_definition" => {
            stats.functions += 1;
            render_method(node, source, out, stats, indent + 1);
            true
        }
        // Classes
        "class_declaration" => {
            stats.classes += 1;
            render_class(node, source, out, stats, indent);
            true
        }
        "abstract_class_declaration" => {
            stats.classes += 1;
            render_class(node, source, out, stats, indent);
            true
        }
        // Interfaces and type aliases
        "interface_declaration" => {
            stats.interfaces += 1;
            render_interface(node, source, out, stats, indent);
            true
        }
        "type_alias_declaration" => {
            stats.type_aliases += 1;
            out.push_str(&pad);
            out.push_str(&text);
            out.push('\n');
            true
        }
        // Variable declarations (const/let/var): recurse so arrow functions
        // in the initializer get collapsed.
        "lexical_declaration" | "variable_statement" | "variable_declaration" => {
            out.push_str(&pad);
            render_variable_declaration(node, source, out, stats, indent);
            true
        }
        // Type-only constructs
        "ambient_declaration" | "declare_statement" => {
            out.push_str(&pad);
            out.push_str(&text);
            out.push('\n');
            true
        }
        // Function expressions and arrow functions: collapse body
        "arrow_function" | "function" | "function_expression" => {
            stats.functions += 1;
            render_arrow(node, source, out);
            true
        }
        // Skip semicolons and noise
        ";" | "," | "(" | ")" | "{" | "}" | "[" | "]" => {
            out.push_str(&text);
            false
        }
        // Comments - skip in AST mode (we don't need them in collapsed context)
        "comment" => false,
        // Top-level statements: just pass through their text
        _ => {
            // For unknown nodes, render their source text as-is
            out.push_str(&pad);
            out.push_str(&text);
            out.push('\n');
            true
        }
    }
}

fn render_function(node: Node, source: &str, out: &mut String, stats: &mut TsStats, indent: usize) {
    stats.functions += 1;
    let pad = "  ".repeat(indent);
    out.push_str(&pad);

    // Get function header: function name(params): return_type
    if let Some(name_node) = node.child_by_field_name("name") {
        out.push_str("function ");
        out.push_str(&node_text(name_node, source));
    } else {
        out.push_str("function");
    }

    // Parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        out.push_str(&node_text(params, source));
    } else {
        out.push_str("()");
    }

    // Return type
    if let Some(ret) = node.child_by_field_name("return_type") {
        out.push(' ');
        out.push_str(&node_text(ret, source));
    }

    out.push_str(" { /* ... */ }\n");
}

fn render_method(node: Node, source: &str, out: &mut String, _stats: &mut TsStats, indent: usize) {
    let pad = "  ".repeat(indent);
    out.push_str(&pad);

    // Modifiers (public, private, static, async, etc.)
    if let Some(modifiers) = node.child_by_field_name("modifiers") {
        out.push_str(&node_text(modifiers, source));
        out.push(' ');
    }

    // Method name
    if let Some(name_node) = node.child_by_field_name("name") {
        out.push_str(&node_text(name_node, source));
    }

    // Parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        out.push_str(&node_text(params, source));
    } else {
        out.push_str("()");
    }

    // Return type
    if let Some(ret) = node.child_by_field_name("return_type") {
        out.push(' ');
        out.push_str(&node_text(ret, source));
    }

    // Type parameters (<T>, <T extends Foo>)
    if let Some(type_params) = node.child_by_field_name("type_parameters") {
        out.push(' ');
        out.push_str(&node_text(type_params, source));
    }

    out.push_str(" { /* ... */ }\n");
}

fn render_class(node: Node, source: &str, out: &mut String, stats: &mut TsStats, indent: usize) {
    let pad = "  ".repeat(indent);
    out.push_str(&pad);

    // Modifiers + class keyword + name + generics + extends + implements
    let mut header = String::new();
    if let Some(modifiers) = node.child_by_field_name("modifiers") {
        header.push_str(&node_text(modifiers, source));
        header.push(' ');
    }
    header.push_str("class ");
    if let Some(name) = node.child_by_field_name("name") {
        header.push_str(&node_text(name, source));
    }
    if let Some(type_params) = node.child_by_field_name("type_parameters") {
        header.push(' ');
        header.push_str(&node_text(type_params, source));
    }
    if let Some(extends) = node.child_by_field_name("class_heritage") {
        header.push(' ');
        header.push_str(&node_text(extends, source));
    }
    out.push_str(&header);
    out.push_str(" {\n");

    // Render body (class body)
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "method_definition"
                || child.kind() == "public_field_definition"
                || child.kind() == "private_field_definition"
                || child.kind() == "protected_field_definition"
            {
                render_node(child, source, out, stats, indent + 1);
            } else {
                // Pass through other body members
                out.push_str(&"  ".repeat(indent + 1));
                out.push_str(&node_text(child, source));
                out.push('\n');
            }
        }
    }
    out.push_str(&pad);
    out.push_str("}\n");
}

fn render_interface(
    node: Node,
    source: &str,
    out: &mut String,
    _stats: &mut TsStats,
    indent: usize,
) {
    let pad = "  ".repeat(indent);
    out.push_str(&pad);

    let mut header = String::new();
    if let Some(modifiers) = node.child_by_field_name("modifiers") {
        header.push_str(&node_text(modifiers, source));
        header.push(' ');
    }
    header.push_str("interface ");
    if let Some(name) = node.child_by_field_name("name") {
        header.push_str(&node_text(name, source));
    }
    if let Some(type_params) = node.child_by_field_name("type_parameters") {
        header.push(' ');
        header.push_str(&node_text(type_params, source));
    }
    if let Some(extends) = node.child_by_field_name("extends") {
        let mut first = true;
        for child in extends.children(&mut extends.walk()) {
            if first {
                header.push_str(" extends ");
                first = false;
            }
            header.push_str(&node_text(child, source));
            header.push_str(", ");
        }
    }
    out.push_str(&header);
    out.push_str(" {\n");

    // Render body — properties and method signatures
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            out.push_str(&"  ".repeat(indent + 1));
            out.push_str(&node_text(child, source));
            out.push('\n');
        }
    }
    out.push_str(&pad);
    out.push_str("}\n");
}

fn render_export(node: Node, source: &str, out: &mut String, stats: &mut TsStats, indent: usize) {
    let pad = "  ".repeat(indent);

    // Look at the exported child
    let mut cursor = node.walk();
    let mut found_export = false;
    for child in node.children(&mut cursor) {
        if child.kind() == "export_clause" {
            out.push_str(&pad);
            out.push_str(&node_text(child, source));
            out.push('\n');
            found_export = true;
        } else if child.kind() == "default"
            || child.kind() == "from"
            || child.kind() == "as"
            || child.kind() == "type"
        {
            // Skip keywords
        } else {
            // Recurse into the exported declaration
            if render_node(child, source, out, stats, indent) {
                found_export = true;
            }
        }
    }

    if !found_export {
        // Just emit the text as-is
        out.push_str(&pad);
        out.push_str(&node_text(node, source));
        out.push('\n');
    }
}

fn render_arrow(node: Node, source: &str, out: &mut String) {
    // For arrow functions, keep the params and return type but collapse body
    let text = node_text(node, source);

    if let Some(body) = node.child_by_field_name("body") {
        // Find the body region and replace with /* ... */
        let body_start = body.start_byte();
        let body_end = body.end_byte();
        if body_end > node.end_byte() {
            out.push_str(&text);
            return;
        }
        let before = &text[..body_start.saturating_sub(node.start_byte())];
        let _ = body_end;
        out.push_str(before);
        out.push_str("/* ... */");
    } else {
        // No body field — look for a statement_block child
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "statement_block" {
                let body_start = child.start_byte();
                let before = &text[..body_start.saturating_sub(node.start_byte())];
                out.push_str(before);
                out.push_str("/* ... */");
                return;
            }
        }
        out.push_str(&text);
    }
}

/// Render a variable declaration, collapsing arrow function bodies.
fn render_variable_declaration(
    node: Node,
    source: &str,
    out: &mut String,
    stats: &mut TsStats,
    indent: usize,
) {
    // Reconstruct: keep the keyword and variable name, but process the initializer
    // through render_node so arrow functions get collapsed.
    let mut cursor = node.walk();
    let mut parts: Vec<String> = Vec::new();
    let mut last_was_value = false;
    let last_name_end: Option<usize> = None;

    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "=" || kind == "," || kind == ";" {
            parts.push(node_text(child, source));
            if kind == "=" {
                last_was_value = false;
            }
        } else if kind == "variable_declarator" {
            // Render declarator: name = <rendered initializer>
            if let Some(name) = child.child_by_field_name("name") {
                parts.push(node_text(name, source));
            }
            if let Some(value) = child.child_by_field_name("value") {
                parts.push("=".to_string());

                // Render the value (which may be an arrow function)
                let mut value_buf = String::new();
                render_node(value, source, &mut value_buf, stats, indent);
                parts.push(value_buf);
                last_was_value = true;
            }
        } else if matches!(kind, "const" | "let" | "var") {
            parts.push(node_text(child, source));
        } else if kind == "comment" {
            // Skip
        } else {
            // Other token (whitespace, etc.)
            parts.push(node_text(child, source));
        }
    }

    // Suppress warning
    let _ = last_name_end;
    let _ = last_was_value;

    out.push_str(&parts.join(""));
}

fn node_text(node: Node, source: &str) -> String {
    let start = node.start_byte();
    let end = node.end_byte();
    source[start..end].to_string()
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
        assert!(result.content.contains("/* ... */"));
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
        assert!(result.content.contains("validate"));
        assert!(result.content.contains("boolean"));
        // Method bodies should NOT be in output
        assert!(!result.content.contains("this.name.length === 0"));
    }

    #[test]
    fn test_collapse_interface() {
        let code = r#"
            interface User {
                id: number;
                name: string;
                email?: string;
                validate(): boolean;
            }
        "#;
        let opts = CompressionOptions::default();
        let result = compress_typescript(code, &opts).unwrap();
        assert!(result.interfaces_preserved >= 1);
        assert!(result.content.contains("interface User"));
        assert!(result.content.contains("id: number"));
        assert!(result.content.contains("validate(): boolean"));
    }

    #[test]
    fn test_collapse_type_alias() {
        let code = r#"
            type Status = "active" | "inactive";
            type UserMap = Map<string, User>;
        "#;
        let opts = CompressionOptions::default();
        let result = compress_typescript(code, &opts).unwrap();
        assert!(result.type_aliases >= 2);
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
                useEffect(() => { /* ... */ }, []);
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
        eprintln!("ARROW OUTPUT:\n{}", result.content);
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

        // Should be at least 30% smaller
        assert!(savings > 30.0, "Expected >30% savings, got {savings:.1}%");

        // All public APIs preserved
        assert!(result.content.contains("interface User"));
        assert!(result.content.contains("class UserService"));
        assert!(result.content.contains("findById"));
        assert!(result.content.contains("create"));
        assert!(result.content.contains("update"));
    }
}
