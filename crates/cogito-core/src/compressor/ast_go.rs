//! AST-aware Go compressor
//!
//! Uses `tree-sitter-go` to parse the source into an AST, then walks
//! the tree collapsing function/method bodies while preserving signatures,
//! type declarations, and imports.

use crate::types::CompressionOptions;
use anyhow::{Result, anyhow};
use std::sync::OnceLock;
use tree_sitter::{Node, Parser};

/// Result of Go AST compression
#[derive(Debug, Clone)]
pub struct GoCompressResult {
    pub content: String,
    pub functions_collapsed: usize,
    pub methods_collapsed: usize,
    pub types_preserved: usize,
}

/// Compress Go source using AST awareness.
pub fn compress_go(content: &str, _options: &CompressionOptions) -> Result<GoCompressResult> {
    let mut parser = Parser::new();
    parser
        .set_language(&get_language())
        .map_err(|e| anyhow!("Failed to set tree-sitter language: {e}"))?;

    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow!("Failed to parse Go"))?;

    let mut stats = GoStats::default();
    let mut out = String::with_capacity(content.len() / 2);

    let root = tree.root_node();
    walk(root, source_for(&tree, content), &mut out, &mut stats);

    Ok(GoCompressResult {
        content: out,
        functions_collapsed: stats.functions,
        methods_collapsed: stats.methods,
        types_preserved: stats.types,
    })
}

#[derive(Default)]
struct GoStats {
    functions: usize,
    methods: usize,
    types: usize,
}

fn source_for<'a>(_: &'a tree_sitter::Tree, content: &'a str) -> &'a str {
    content
}

static LANGUAGE: OnceLock<tree_sitter::Language> = OnceLock::new();

fn get_language() -> tree_sitter::Language {
    LANGUAGE
        .get_or_init(|| tree_sitter_go::LANGUAGE.into())
        .clone()
}

fn walk(node: Node, source: &str, out: &mut String, stats: &mut GoStats) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        render_node(child, source, out, stats, 0);
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
}

fn render_node(node: Node, source: &str, out: &mut String, stats: &mut GoStats, indent: usize) {
    let text = node_text(node, source);
    let pad = "\t".repeat(indent);

    match node.kind() {
        // Package declaration
        "package_clause" => {
            out.push_str(&pad);
            out.push_str(&text);
        }
        // Imports
        "import_declaration" => {
            out.push_str(&pad);
            out.push_str(&text);
        }
        // Function declarations
        "function_declaration" => {
            stats.functions += 1;
            render_function(node, source, out, indent);
        }
        // Methods (functions inside receivers)
        "method_declaration" => {
            stats.methods += 1;
            render_method(node, source, out, indent);
        }
        // Type declarations
        "type_declaration" => {
            stats.types += 1;
            out.push_str(&pad);
            out.push_str(&text);
        }
        // Variable declarations
        "var_declaration" | "const_declaration" | "short_var_declaration" => {
            out.push_str(&pad);
            out.push_str(&text);
        }
        // Top-level statements
        _ => {
            out.push_str(&pad);
            out.push_str(&text);
        }
    }
}

fn render_function(node: Node, source: &str, out: &mut String, indent: usize) {
    let pad = "\t".repeat(indent);

    // `func name(params) ReturnType {`
    if let Some(name) = node.child_by_field_name("name") {
        out.push_str(&pad);
        out.push_str("func ");
        out.push_str(&node_text(name, source));
    } else {
        out.push_str(&pad);
        out.push_str("func ");
    }

    if let Some(params) = node.child_by_field_name("parameters") {
        out.push_str(&node_text(params, source));
    } else {
        out.push_str("()");
    }

    if let Some(result) = node.child_by_field_name("result") {
        out.push(' ');
        out.push_str(&node_text(result, source));
    }

    out.push_str(" { /* ... */ }");
}

fn render_method(node: Node, source: &str, out: &mut String, indent: usize) {
    let pad = "\t".repeat(indent);

    // `func (r *Type) name(params) ReturnType {`
    if let Some(receiver) = node.child_by_field_name("receiver") {
        out.push_str(&pad);
        out.push_str("func ");
        out.push_str(&node_text(receiver, source));
        out.push(' ');
    } else {
        out.push_str(&pad);
        out.push_str("func ");
    }

    if let Some(name) = node.child_by_field_name("name") {
        out.push_str(&node_text(name, source));
    }

    if let Some(params) = node.child_by_field_name("parameters") {
        out.push_str(&node_text(params, source));
    } else {
        out.push_str("()");
    }

    if let Some(result) = node.child_by_field_name("result") {
        out.push(' ');
        out.push_str(&node_text(result, source));
    }

    out.push_str(" { /* ... */ }");
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
package main

import "fmt"

func Add(a int, b int) int {
    result := a + b
    fmt.Printf("debug: a=%d b=%d", a, b)
    if a > 100 {
        return b
    }
    return result * 2
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_go(code, &opts).unwrap();
        assert!(result.functions_collapsed >= 1);
        assert!(result.content.contains("package main"));
        assert!(result.content.contains("func Add"));
        assert!(result.content.contains("a int"));
        assert!(result.content.contains("b int"));
        assert!(result.content.contains("int {"));
        assert!(result.content.contains("/* ... */"));
        // Body should NOT be in output
        assert!(!result.content.contains("result :="));
        assert!(!result.content.contains("fmt.Printf"));
    }

    #[test]
    fn test_collapse_method() {
        let code = r#"
package service

type User struct {
    ID   int
    Name string
}

func (s *UserService) FindByID(id int) (*User, error) {
    if id <= 0 {
        return nil, errors.New("invalid id")
    }
    user, err := s.repo.Find(id)
    if err != nil {
        return nil, err
    }
    return user, nil
}

func (s *UserService) Create(u User) error {
    if u.Name == "" {
        return errors.New("name required")
    }
    return s.repo.Save(u)
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_go(code, &opts).unwrap();
        assert!(result.types_preserved >= 1);
        assert!(result.methods_collapsed >= 2);
        assert!(result.content.contains("type User struct"));
        assert!(result.content.contains("func (s *UserService) FindByID"));
        assert!(result.content.contains("func (s *UserService) Create"));
        // Method bodies should NOT be in output
        assert!(!result.content.contains("errors.New"));
    }

    #[test]
    fn test_preserves_imports() {
        let code = r#"
package main

import (
    "context"
    "fmt"
)

func Process(ctx context.Context, data []byte) error {
    return fmt.Errorf("not implemented")
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_go(code, &opts).unwrap();
        assert!(result.content.contains("package main"));
        assert!(result.content.contains("import ("));
        assert!(result.content.contains("context"));
        assert!(result.content.contains("fmt"));
    }

    #[test]
    fn test_preserves_type_declarations() {
        let code = r#"
package models

type Status string

const (
    StatusActive   Status = "active"
    StatusInactive Status = "inactive"
)

type User struct {
    ID        int    `json:"id"`
    Name      string `json:"name"`
    CreatedAt time.Time
}

type UserRepository interface {
    Find(id int) (*User, error)
    Save(user *User) error
    Delete(id int) error
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_go(code, &opts).unwrap();
        assert!(result.types_preserved >= 3);
        assert!(result.content.contains("type Status string"));
        assert!(result.content.contains("type User struct"));
        assert!(result.content.contains("type UserRepository interface"));
        assert!(result.content.contains("Find(id int)"));
    }

    #[test]
    fn test_real_world_module_savings() {
        let code = r#"
package service

import (
    "context"
    "errors"
    "fmt"
)

type User struct {
    ID   int
    Name string
}

type UserRepository interface {
    Find(ctx context.Context, id int) (*User, error)
    Save(ctx context.Context, user *User) error
    Delete(ctx context.Context, id int) error
}

type UserService struct {
    repo UserRepository
}

func NewUserService(repo UserRepository) *UserService {
    return &UserService{repo: repo}
}

func (s *UserService) FindByID(ctx context.Context, id int) (*User, error) {
    if id <= 0 {
        return nil, errors.New("invalid id")
    }
    user, err := s.repo.Find(ctx, id)
    if err != nil {
        return nil, fmt.Errorf("find user: %w", err)
    }
    return user, nil
}

func (s *UserService) Create(ctx context.Context, name string) (*User, error) {
    if name == "" {
        return nil, errors.New("name required")
    }
    user := &User{ID: generateID(), Name: name}
    if err := s.repo.Save(ctx, user); err != nil {
        return nil, fmt.Errorf("save user: %w", err)
    }
    return user, nil
}

func generateID() int {
    return int(time.Now().UnixNano() % 1_000_000)
}
"#;
        let opts = CompressionOptions::default();
        let result = compress_go(code, &opts).unwrap();
        let orig = code.len();
        let comp = result.content.len();
        let savings = 100.0 * (1.0 - comp as f64 / orig as f64);
        println!(
            "Real-world Go module: {} -> {} bytes ({:.1}% savings)",
            orig, comp, savings
        );

        // Should be at least 30% smaller
        assert!(savings > 30.0, "Expected >30% savings, got {savings:.1}%");

        // All public APIs preserved
        assert!(result.content.contains("type User struct"));
        assert!(result.content.contains("type UserRepository interface"));
        assert!(result.content.contains("type UserService struct"));
        assert!(result.content.contains("func NewUserService"));
        assert!(result.content.contains("func (s *UserService) FindByID"));
        assert!(result.content.contains("func (s *UserService) Create"));
    }
}
